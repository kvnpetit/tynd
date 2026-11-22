import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import { decodePng, encodePng, type RawImage } from "./png.ts";
import { diffImages, diffPng, expectScreenshot, ScreenshotMismatchError } from "./screenshot.ts";

function solid(width: number, height: number, rgba: [number, number, number, number]): RawImage {
  const data = new Uint8Array(width * height * 4);
  for (let pixel = 0; pixel < width * height; pixel++) data.set(rgba, pixel * 4);
  return { width, height, data };
}

function setPixel(image: RawImage, x: number, y: number, rgba: [number, number, number, number]) {
  image.data.set(rgba, (y * image.width + x) * 4);
}

describe("png codec", () => {
  test("encode then decode round-trips RGBA pixels", () => {
    const image = solid(3, 2, [10, 20, 30, 255]);
    setPixel(image, 2, 1, [200, 100, 50, 128]);
    const decoded = decodePng(encodePng(image));
    expect(decoded.width).toBe(3);
    expect(decoded.height).toBe(2);
    expect([...decoded.data]).toEqual([...image.data]);
  });

  test("rejects data that is not a PNG", () => {
    expect(() => decodePng(new Uint8Array([1, 2, 3]))).toThrow("not a PNG");
  });

  test("decodes filtered scanlines (sub, up, average, paeth)", () => {
    // 2x4 RGB image, one filter type per row, built by hand from raw bytes.
    const rows = [
      [10, 20, 30, 40, 50, 60],
      [11, 21, 31, 41, 51, 61],
      [12, 22, 32, 42, 52, 62],
      [13, 23, 33, 43, 53, 63],
    ];
    const channels = 3;
    const filtered: number[] = [];
    rows.forEach((row, index) => {
      const previous = index > 0 ? (rows[index - 1] ?? []) : row.map(() => 0);
      filtered.push(index + 1);
      row.forEach((value, i) => {
        const left = i >= channels ? (row[i - channels] ?? 0) : 0;
        const up = index > 0 ? (previous[i] ?? 0) : 0;
        const upLeft = index > 0 && i >= channels ? (previous[i - channels] ?? 0) : 0;
        const p = left + up - upLeft;
        const [pa, pb, pc] = [Math.abs(p - left), Math.abs(p - up), Math.abs(p - upLeft)];
        const paeth = pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft;
        const predictor = [left, up, (left + up) >> 1, paeth][index] ?? 0;
        filtered.push((value - predictor) & 0xff);
      });
    });
    const header = Buffer.alloc(13);
    header.writeUInt32BE(2, 0);
    header.writeUInt32BE(4, 4);
    header.set([8, 2, 0, 0, 0], 8);
    const fixed = Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      chunkBytes("IHDR", header),
      chunkBytes("IDAT", deflateSync(Uint8Array.from(filtered))),
      chunkBytes("IEND", new Uint8Array(0)),
    ]);
    const decoded = decodePng(fixed);
    expect(decoded.width).toBe(2);
    const expectedPixels = rows.flatMap((row) => [
      ...row.slice(0, 3),
      255,
      ...row.slice(3, 6),
      255,
    ]);
    expect([...decoded.data]).toEqual(expectedPixels);
  });
});

function chunkBytes(type: string, body: Uint8Array): Buffer {
  const out = Buffer.alloc(12 + body.length);
  out.writeUInt32BE(body.length, 0);
  out.write(type, 4, "ascii");
  out.set(body, 8);
  out.writeUInt32BE(crc(out.subarray(4, 8 + body.length)), 8 + body.length);
  return out;
}

function crc(bytes: Uint8Array): number {
  let value = 0xffffffff;
  for (const byte of bytes) {
    value ^= byte;
    for (let k = 0; k < 8; k++) value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  return (value ^ 0xffffffff) >>> 0;
}

describe("diffImages", () => {
  test("identical images match with no diff image", () => {
    const result = diffImages(solid(4, 4, [1, 2, 3, 255]), solid(4, 4, [1, 2, 3, 255]));
    expect(result.match).toBe(true);
    expect(result.diffPixels).toBe(0);
    expect(result.diffPng).toBeUndefined();
  });

  test("counts differing pixels and renders a diff image", () => {
    const actual = solid(4, 4, [255, 255, 255, 255]);
    setPixel(actual, 1, 1, [0, 0, 0, 255]);
    setPixel(actual, 2, 3, [0, 0, 0, 255]);
    const result = diffImages(actual, solid(4, 4, [255, 255, 255, 255]));
    expect(result.match).toBe(false);
    expect(result.diffPixels).toBe(2);
    expect(result.diffRatio).toBeCloseTo(2 / 16);
    const diff = decodePng(result.diffPng ?? new Uint8Array(0));
    expect([...diff.data.subarray((1 * 4 + 1) * 4, (1 * 4 + 1) * 4 + 4)]).toEqual([255, 0, 0, 255]);
  });

  test("a color change below the threshold is ignored, above it is reported", () => {
    const base = solid(2, 2, [100, 100, 100, 255]);
    const slightly = solid(2, 2, [104, 100, 100, 255]);
    expect(diffImages(slightly, base, { threshold: 0.1 }).match).toBe(true);
    expect(diffImages(slightly, base, { threshold: 0 }).match).toBe(false);
  });

  test("maxDiffPixels and maxDiffRatio tolerate small differences", () => {
    const actual = solid(10, 10, [255, 255, 255, 255]);
    setPixel(actual, 0, 0, [0, 0, 0, 255]);
    const expected = solid(10, 10, [255, 255, 255, 255]);
    expect(diffImages(actual, expected).match).toBe(false);
    expect(diffImages(actual, expected, { maxDiffPixels: 1 }).match).toBe(true);
    expect(diffImages(actual, expected, { maxDiffRatio: 0.01 }).match).toBe(true);
    expect(diffImages(actual, expected, { maxDiffRatio: 0.001 }).match).toBe(false);
  });

  test("different dimensions never match", () => {
    const result = diffImages(solid(2, 2, [0, 0, 0, 255]), solid(3, 2, [0, 0, 0, 255]));
    expect(result.match).toBe(false);
    expect(result.sizeMismatch).toBe(true);
  });

  test("diffPng works on encoded bytes", () => {
    const png = encodePng(solid(2, 2, [9, 9, 9, 255]));
    expect(diffPng(png, png).match).toBe(true);
  });
});

describe("expectScreenshot", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "tynd-test-shot-"));
  });
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  test("creates a missing baseline and reports it as updated", () => {
    const png = encodePng(solid(2, 2, [1, 1, 1, 255]));
    const baseline = join(dir, "nested", "home.png");
    expect(expectScreenshot(png, baseline, { update: false })).toEqual({ updated: true });
    expect([...readFileSync(baseline)]).toEqual([...png]);
  });

  test("passes when the screenshot matches the baseline", () => {
    const baseline = join(dir, "home.png");
    writeFileSync(baseline, encodePng(solid(2, 2, [1, 1, 1, 255])));
    const outcome = expectScreenshot(encodePng(solid(2, 2, [1, 1, 1, 255])), baseline, {
      update: false,
    });
    expect(outcome.updated).toBe(false);
    expect(outcome.diff?.match).toBe(true);
  });

  test("on mismatch writes actual and diff images and throws", () => {
    const baseline = join(dir, "home.png");
    writeFileSync(baseline, encodePng(solid(2, 2, [255, 255, 255, 255])));
    const changed = encodePng(solid(2, 2, [0, 0, 0, 255]));
    expect(() => expectScreenshot(changed, baseline, { update: false })).toThrow(
      ScreenshotMismatchError,
    );
    expect(existsSync(join(dir, "home.actual.png"))).toBe(true);
    expect(existsSync(join(dir, "home.diff.png"))).toBe(true);
  });

  test("update rewrites an existing baseline instead of comparing", () => {
    const baseline = join(dir, "home.png");
    writeFileSync(baseline, encodePng(solid(2, 2, [255, 255, 255, 255])));
    const changed = encodePng(solid(2, 2, [0, 0, 0, 255]));
    expect(expectScreenshot(changed, baseline, { update: true })).toEqual({ updated: true });
    expect([...readFileSync(baseline)]).toEqual([...changed]);
  });
});
