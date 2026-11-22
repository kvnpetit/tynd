import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { decodePng, encodePng, type RawImage } from "./png.ts";

export interface DiffOptions {
  /**
   * Per-pixel color sensitivity from 0 (exact) to 1 (anything matches), as in pixelmatch.
   * Default 0.1.
   */
  threshold?: number;
  /** Differing pixels tolerated before the images count as different. Default 0. */
  maxDiffPixels?: number;
  /** Fraction (0..1) of differing pixels tolerated. Default 0. */
  maxDiffRatio?: number;
}

export interface DiffResult {
  match: boolean;
  /** True when the two images have different dimensions. */
  sizeMismatch: boolean;
  width: number;
  height: number;
  diffPixels: number;
  diffRatio: number;
  /** PNG highlighting differing pixels in red over a faded copy of the baseline. */
  diffPng?: Uint8Array;
}

/** Largest possible squared YIQ distance, used to scale the threshold (same constant as pixelmatch). */
const MAX_YIQ_DELTA = 35215;

/** Compare two PNG images pixel by pixel. */
export function diffPng(
  actual: Uint8Array,
  expected: Uint8Array,
  options: DiffOptions = {},
): DiffResult {
  return diffImages(decodePng(actual), decodePng(expected), options);
}

export function diffImages(
  actual: RawImage,
  expected: RawImage,
  options: DiffOptions = {},
): DiffResult {
  const threshold = options.threshold ?? 0.1;
  if (actual.width !== expected.width || actual.height !== expected.height) {
    const width = Math.max(actual.width, expected.width);
    const height = Math.max(actual.height, expected.height);
    return {
      match: false,
      sizeMismatch: true,
      width,
      height,
      diffPixels: width * height,
      diffRatio: 1,
    };
  }
  const { width, height } = actual;
  const total = width * height;
  const maxDelta = MAX_YIQ_DELTA * threshold * threshold;
  const diff = new Uint8Array(total * 4);
  let diffPixels = 0;
  for (let pixel = 0; pixel < total; pixel++) {
    const at = pixel * 4;
    const delta = colorDelta(actual.data, expected.data, at);
    if (delta > maxDelta) {
      diffPixels++;
      diff.set([255, 0, 0, 255], at);
    } else {
      const gray = 255 + (grayOf(expected.data, at) - 255) * 0.1;
      diff.set([gray, gray, gray, 255], at);
    }
  }
  const diffRatio = total === 0 ? 0 : diffPixels / total;
  const match =
    diffPixels <= (options.maxDiffPixels ?? 0) || diffRatio <= (options.maxDiffRatio ?? 0);
  return {
    match,
    sizeMismatch: false,
    width,
    height,
    diffPixels,
    diffRatio,
    diffPng: diffPixels === 0 ? undefined : encodePng({ width, height, data: diff }),
  };
}

export class ScreenshotMismatchError extends Error {
  readonly result: DiffResult;
  readonly baselinePath: string;
  readonly actualPath: string;
  readonly diffPath: string;

  constructor(result: DiffResult, baselinePath: string, actualPath: string, diffPath: string) {
    const detail = result.sizeMismatch
      ? "image size differs from the baseline"
      : `${result.diffPixels} pixels differ (${(result.diffRatio * 100).toFixed(2)}%)`;
    super(`screenshot does not match ${baselinePath}: ${detail}. Actual: ${actualPath}`);
    this.name = "ScreenshotMismatchError";
    this.result = result;
    this.baselinePath = baselinePath;
    this.actualPath = actualPath;
    this.diffPath = diffPath;
  }
}

export interface ExpectScreenshotOptions extends DiffOptions {
  /** Rewrite the baseline instead of comparing. Defaults to `TYND_UPDATE_SNAPSHOTS=1`. */
  update?: boolean;
}

export interface ExpectScreenshotResult {
  /** True when the baseline was created or rewritten rather than compared. */
  updated: boolean;
  diff?: DiffResult;
}

/**
 * Compare `actual` (PNG bytes) with the baseline file. A missing baseline is created, and
 * `update` (or `TYND_UPDATE_SNAPSHOTS=1`, set by `tynd test --update-snapshots`) rewrites it.
 * On a mismatch the actual and diff images are written next to the baseline and an error is thrown.
 */
export function expectScreenshot(
  actual: Uint8Array,
  baselinePath: string,
  options: ExpectScreenshotOptions = {},
): ExpectScreenshotResult {
  const update = options.update ?? process.env.TYND_UPDATE_SNAPSHOTS === "1";
  if (update || !existsSync(baselinePath)) {
    mkdirSync(dirname(baselinePath), { recursive: true });
    writeFileSync(baselinePath, actual);
    return { updated: true };
  }
  const result = diffPng(actual, new Uint8Array(readFileSync(baselinePath)), options);
  if (result.match) return { updated: false, diff: result };
  const stem = baselinePath.replace(/\.png$/i, "");
  const actualPath = `${stem}.actual.png`;
  const diffPath = `${stem}.diff.png`;
  writeFileSync(actualPath, actual);
  if (result.diffPng) writeFileSync(diffPath, result.diffPng);
  throw new ScreenshotMismatchError(result, baselinePath, actualPath, diffPath);
}

function grayOf(data: Uint8Array, at: number): number {
  const alpha = (data[at + 3] ?? 255) / 255;
  const [r, g, b] = blend(data, at, alpha);
  return r * 0.299 + g * 0.587 + b * 0.114;
}

function blend(data: Uint8Array, at: number, alpha: number): [number, number, number] {
  return [
    255 + ((data[at] ?? 0) - 255) * alpha,
    255 + ((data[at + 1] ?? 0) - 255) * alpha,
    255 + ((data[at + 2] ?? 0) - 255) * alpha,
  ];
}

/** Squared perceptual (YIQ) distance between two RGBA pixels, alpha-blended on white. */
function colorDelta(a: Uint8Array, b: Uint8Array, at: number): number {
  if (
    a[at] === b[at] &&
    a[at + 1] === b[at + 1] &&
    a[at + 2] === b[at + 2] &&
    a[at + 3] === b[at + 3]
  ) {
    return 0;
  }
  const [r1, g1, b1] = blend(a, at, (a[at + 3] ?? 255) / 255);
  const [r2, g2, b2] = blend(b, at, (b[at + 3] ?? 255) / 255);
  const y = rgb2y(r1, g1, b1) - rgb2y(r2, g2, b2);
  const i = rgb2i(r1, g1, b1) - rgb2i(r2, g2, b2);
  const q = rgb2q(r1, g1, b1) - rgb2q(r2, g2, b2);
  return 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
}

function rgb2y(r: number, g: number, b: number): number {
  return r * 0.29889531 + g * 0.58662247 + b * 0.11448223;
}

function rgb2i(r: number, g: number, b: number): number {
  return r * 0.59597799 - g * 0.2741761 - b * 0.32180189;
}

function rgb2q(r: number, g: number, b: number): number {
  return r * 0.21147017 - g * 0.52261711 + b * 0.31114694;
}
