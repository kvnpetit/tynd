/**
 * Minimal dependency-free PNG codec, enough for the screenshots the host produces
 * (8-bit, non-interlaced). Decoding always yields RGBA8.
 */
import { deflateSync, inflateSync } from "node:zlib";

export interface RawImage {
  width: number;
  height: number;
  /** RGBA, 4 bytes per pixel, row-major. */
  data: Uint8Array;
}

const SIGNATURE = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const CHANNELS: Record<number, number> = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 };

interface Header {
  width: number;
  height: number;
  colorType: number;
  channels: number;
  palette?: Uint8Array;
  alphaPalette?: Uint8Array;
  idat: Uint8Array[];
}

export function decodePng(png: Uint8Array): RawImage {
  if (png.length < 8 || SIGNATURE.some((byte, index) => png[index] !== byte)) {
    throw new Error("not a PNG file");
  }
  const header = readChunks(png);
  const raw = inflateSync(Buffer.concat(header.idat));
  const { width, height, channels } = header;
  if (raw.length < (width * channels + 1) * height) throw new Error("PNG pixel data is truncated");
  const pixels = unfilter(raw, width, height, channels);
  const data = new Uint8Array(width * height * 4);
  for (let index = 0; index < width * height; index++) {
    data.set(rgbaAt(pixels, index * channels, header), index * 4);
  }
  return { width, height, data };
}

function readChunks(png: Uint8Array): Header {
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  const header: Header = { width: 0, height: 0, colorType: -1, channels: 0, idat: [] };
  let offset = 8;
  while (offset + 8 <= png.length) {
    const length = view.getUint32(offset);
    const type = String.fromCharCode(...png.subarray(offset + 4, offset + 8));
    const body = png.subarray(offset + 8, offset + 8 + length);
    offset += 12 + length;
    if (type === "IEND") break;
    applyChunk(header, type, body);
  }
  if (header.channels === 0 || header.width === 0 || header.height === 0) {
    throw new Error("invalid or unsupported PNG");
  }
  return header;
}

function applyChunk(header: Header, type: string, body: Uint8Array): void {
  if (type === "IHDR") {
    const fields = new DataView(body.buffer, body.byteOffset, body.byteLength);
    header.width = fields.getUint32(0);
    header.height = fields.getUint32(4);
    header.colorType = body[9] ?? -1;
    header.channels = CHANNELS[header.colorType] ?? 0;
    if (body[8] !== 8) throw new Error(`unsupported PNG bit depth ${body[8]}`);
    if (body[12] !== 0) throw new Error("interlaced PNG is not supported");
  } else if (type === "PLTE") {
    header.palette = body;
  } else if (type === "tRNS") {
    header.alphaPalette = body;
  } else if (type === "IDAT") {
    header.idat.push(body);
  }
}

function rgbaAt(pixels: Uint8Array, source: number, header: Header): number[] {
  const first = pixels[source] ?? 0;
  switch (header.colorType) {
    case 6:
      return [first, pixels[source + 1] ?? 0, pixels[source + 2] ?? 0, pixels[source + 3] ?? 255];
    case 2:
      return [first, pixels[source + 1] ?? 0, pixels[source + 2] ?? 0, 255];
    case 0:
      return [first, first, first, 255];
    case 4:
      return [first, first, first, pixels[source + 1] ?? 255];
    default: {
      const { palette } = header;
      if (!palette) throw new Error("PNG palette missing");
      const entry = first * 3;
      return [
        palette[entry] ?? 0,
        palette[entry + 1] ?? 0,
        palette[entry + 2] ?? 0,
        header.alphaPalette?.[first] ?? 255,
      ];
    }
  }
}

export function encodePng(image: RawImage): Uint8Array {
  const { width, height, data } = image;
  if (data.length !== width * height * 4) throw new Error("image buffer size mismatch");
  const stride = width * 4;
  const filtered = new Uint8Array((stride + 1) * height);
  for (let row = 0; row < height; row++) {
    filtered.set(data.subarray(row * stride, (row + 1) * stride), row * (stride + 1) + 1);
  }
  const header = new Uint8Array(13);
  const headerView = new DataView(header.buffer);
  headerView.setUint32(0, width);
  headerView.setUint32(4, height);
  header.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([
    SIGNATURE,
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(filtered)),
    chunk("IEND", new Uint8Array(0)),
  ]);
}

function unfilter(raw: Uint8Array, width: number, height: number, channels: number): Uint8Array {
  const stride = width * channels;
  const out = new Uint8Array(stride * height);
  const byteAt = (row: number, column: number) =>
    row >= 0 && column >= 0 ? (out[row * stride + column] ?? 0) : 0;
  for (let row = 0; row < height; row++) {
    const filter = raw[row * (stride + 1)] ?? 0;
    const src = row * (stride + 1) + 1;
    for (let i = 0; i < stride; i++) {
      const predictor = predict(
        filter,
        byteAt(row, i - channels),
        byteAt(row - 1, i),
        byteAt(row - 1, i - channels),
      );
      out[row * stride + i] = ((raw[src + i] ?? 0) + predictor) & 0xff;
    }
  }
  return out;
}

function predict(filter: number, left: number, up: number, upLeft: number): number {
  switch (filter) {
    case 0:
      return 0;
    case 1:
      return left;
    case 2:
      return up;
    case 3:
      return (left + up) >> 1;
    case 4:
      return paeth(left, up, upLeft);
    default:
      throw new Error(`unknown PNG filter ${filter}`);
  }
}

function paeth(a: number, b: number, c: number): number {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  if (pa <= pb && pa <= pc) return a;
  return pb <= pc ? b : c;
}

function chunk(type: string, body: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + body.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, body.length);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  out.set(body, 8);
  view.setUint32(8 + body.length, crc32(out.subarray(4, 8 + body.length)));
  return out;
}

let crcTable: Uint32Array | undefined;

function crc32(bytes: Uint8Array): number {
  if (!crcTable) {
    crcTable = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crcTable[n] = c >>> 0;
    }
  }
  let crc = 0xffffffff;
  for (const byte of bytes) crc = (crcTable[(crc ^ byte) & 0xff] ?? 0) ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}
