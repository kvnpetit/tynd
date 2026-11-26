import { execFileSync } from "node:child_process";
import { readFileSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { deflateRawSync, inflateRawSync } from "node:zlib";

function writeAscii(target: Uint8Array, offset: number, value: string): void {
  target.set(new TextEncoder().encode(value), offset);
}

function writeOctal(target: Uint8Array, offset: number, length: number, value: number): void {
  const text = `${Math.trunc(value)
    .toString(8)
    .padStart(length - 1, "0")}\0`;
  if (text.length > length) throw new Error(`tar numeric field overflow: ${value}`);
  writeAscii(target, offset, text);
}

function sourceDateEpoch(): number {
  const value =
    process.env.SOURCE_DATE_EPOCH ??
    execFileSync("git", ["log", "-1", "--format=%ct"], {
      encoding: "utf8",
      windowsHide: true,
    }).trim();
  const epoch = Number(value);
  if (!Number.isSafeInteger(epoch) || epoch < 0) {
    throw new Error(`SOURCE_DATE_EPOCH must be a non-negative integer, got "${value}"`);
  }
  return epoch;
}

export function createDeterministicTar(
  filename: string,
  contents: Uint8Array,
  epoch = sourceDateEpoch(),
): Uint8Array<ArrayBuffer> {
  const name = filename.replaceAll("\\", "/");
  const encodedName = new TextEncoder().encode(name);
  if (encodedName.length === 0 || encodedName.length > 100 || name.includes("/")) {
    throw new Error(`release tar expects one root filename up to 100 bytes, got "${filename}"`);
  }

  const bodyBlocks = Math.ceil(contents.length / 512);
  const archive = new Uint8Array(512 + bodyBlocks * 512 + 1024);
  const header = archive.subarray(0, 512);
  header.set(encodedName, 0);
  writeOctal(header, 100, 8, 0o755);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, contents.length);
  writeOctal(header, 136, 12, epoch);
  header.fill(32, 148, 156);
  header[156] = 48;
  writeAscii(header, 257, "ustar\0");
  writeAscii(header, 263, "00");

  const checksum = header.reduce((sum, byte) => sum + byte, 0);
  writeAscii(header, 148, `${checksum.toString(8).padStart(6, "0")}\0 `);
  archive.set(contents, 512);
  return archive;
}

function crc32(contents: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of contents) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function ascii(bytes: Uint8Array): string {
  return new TextDecoder("ascii").decode(bytes);
}

function rootFilename(name: string, kind: string): string {
  if (!name || name.includes("/") || name.includes("\\")) {
    throw new Error(`${kind} must contain exactly one root file`);
  }
  return name;
}

function extractTarGz(archive: Uint8Array): { filename: string; contents: Uint8Array } {
  const tar = Bun.gunzipSync(new Uint8Array(archive));
  if (tar.length < 1_536 || ascii(tar.subarray(257, 263)) !== "ustar\0") {
    throw new Error("invalid release tar header");
  }
  const filename = rootFilename(ascii(tar.subarray(0, 100)).split("\0", 1)[0] ?? "", "tar");
  const sizeField = ascii(tar.subarray(124, 136)).replaceAll("\0", "").trim();
  if (!/^[0-7]+$/.test(sizeField)) throw new Error("invalid release tar size");
  const size = Number.parseInt(sizeField, 8);
  const bodyEnd = 512 + Math.ceil(size / 512) * 512;
  if (
    !Number.isSafeInteger(size) ||
    tar.length !== bodyEnd + 1_024 ||
    tar.subarray(bodyEnd).some((byte) => byte !== 0)
  ) {
    throw new Error("release tar must contain one file and two zero terminator blocks");
  }
  const expectedChecksum = Number.parseInt(
    ascii(tar.subarray(148, 156)).replaceAll("\0", "").trim(),
    8,
  );
  let actualChecksum = 0;
  for (let index = 0; index < 512; index += 1) {
    actualChecksum += index >= 148 && index < 156 ? 32 : (tar[index] ?? 0);
  }
  if (!Number.isSafeInteger(expectedChecksum) || expectedChecksum !== actualChecksum) {
    throw new Error("release tar header checksum mismatch");
  }
  return { filename, contents: tar.slice(512, 512 + size) };
}

function extractZip(archive: Uint8Array): { filename: string; contents: Uint8Array } {
  if (archive.length < 98) throw new Error("invalid release zip");
  const view = new DataView(archive.buffer, archive.byteOffset, archive.byteLength);
  if (view.getUint32(0, true) !== 0x04034b50) throw new Error("invalid release zip header");
  const flags = view.getUint16(6, true);
  const method = view.getUint16(8, true);
  const expectedCrc = view.getUint32(14, true);
  const compressedSize = view.getUint32(18, true);
  const uncompressedSize = view.getUint32(22, true);
  const nameLength = view.getUint16(26, true);
  const extraLength = view.getUint16(28, true);
  if (flags !== 0x0800 || method !== 8 || extraLength !== 0) {
    throw new Error("unsupported release zip encoding");
  }
  const filename = rootFilename(
    new TextDecoder().decode(archive.subarray(30, 30 + nameLength)),
    "zip",
  );
  const compressedStart = 30 + nameLength;
  const central = compressedStart + compressedSize;
  if (central + 68 + nameLength !== archive.length) {
    throw new Error("release zip must contain exactly one file");
  }
  if (
    view.getUint32(central, true) !== 0x02014b50 ||
    view.getUint16(central + 10, true) !== method ||
    view.getUint32(central + 16, true) !== expectedCrc ||
    view.getUint32(central + 20, true) !== compressedSize ||
    view.getUint32(central + 24, true) !== uncompressedSize ||
    view.getUint16(central + 28, true) !== nameLength ||
    view.getUint16(central + 30, true) !== 0 ||
    view.getUint16(central + 32, true) !== 0 ||
    view.getUint32(central + 42, true) !== 0 ||
    new TextDecoder().decode(archive.subarray(central + 46, central + 46 + nameLength)) !== filename
  ) {
    throw new Error("release zip central directory disagrees with local entry");
  }
  const end = central + 46 + nameLength;
  if (
    view.getUint32(end, true) !== 0x06054b50 ||
    view.getUint16(end + 8, true) !== 1 ||
    view.getUint16(end + 10, true) !== 1 ||
    view.getUint32(end + 12, true) !== 46 + nameLength ||
    view.getUint32(end + 16, true) !== central ||
    view.getUint16(end + 20, true) !== 0
  ) {
    throw new Error("invalid release zip end record");
  }
  const contents = inflateRawSync(
    archive.subarray(compressedStart, compressedStart + compressedSize),
  );
  if (contents.length !== uncompressedSize || crc32(contents) !== expectedCrc) {
    throw new Error("release zip payload integrity mismatch");
  }
  return { filename, contents };
}

export function extractReleaseAsset(
  filename: string,
  archive: Uint8Array,
): { filename: string; contents: Uint8Array } {
  if (filename.endsWith(".tar.gz")) return extractTarGz(archive);
  if (filename.endsWith(".zip")) return extractZip(archive);
  throw new Error(`unsupported release archive: ${filename}`);
}

function dosTimestamp(epoch: number): { date: number; time: number } {
  const minimumZipEpoch = 315_532_800;
  const date = new Date(Math.max(epoch, minimumZipEpoch) * 1000);
  const year = Math.min(Math.max(date.getUTCFullYear(), 1980), 2107);
  return {
    date: ((year - 1980) << 9) | ((date.getUTCMonth() + 1) << 5) | date.getUTCDate(),
    time: (date.getUTCHours() << 11) | (date.getUTCMinutes() << 5) | (date.getUTCSeconds() >> 1),
  };
}

interface ZipEntry {
  name: Uint8Array;
  compressed: Uint8Array;
  uncompressedSize: number;
  checksum: number;
  timestamp: { date: number; time: number };
}

function createZipEntry(filename: string, contents: Uint8Array, epoch: number): ZipEntry {
  const name = filename.replaceAll("\\", "/");
  if (!name || name.includes("/")) {
    throw new Error(`release zip expects one root filename, got "${filename}"`);
  }
  const encodedName = new TextEncoder().encode(name);
  if (encodedName.length > 0xffff || contents.length > 0xffffffff) {
    throw new Error("release asset exceeds classic ZIP limits");
  }
  const compressed = deflateRawSync(contents, { level: 9 });
  if (compressed.length > 0xffffffff) {
    throw new Error("compressed release asset exceeds ZIP limits");
  }
  return {
    name: encodedName,
    compressed,
    uncompressedSize: contents.length,
    checksum: crc32(contents),
    timestamp: dosTimestamp(epoch),
  };
}

function writeLocalZipEntry(output: Uint8Array, view: DataView, entry: ZipEntry): number {
  view.setUint32(0, 0x04034b50, true);
  view.setUint16(4, 20, true);
  view.setUint16(6, 0x0800, true);
  view.setUint16(8, 8, true);
  view.setUint16(10, entry.timestamp.time, true);
  view.setUint16(12, entry.timestamp.date, true);
  view.setUint32(14, entry.checksum, true);
  view.setUint32(18, entry.compressed.length, true);
  view.setUint32(22, entry.uncompressedSize, true);
  view.setUint16(26, entry.name.length, true);
  view.setUint16(28, 0, true);
  output.set(entry.name, 30);
  output.set(entry.compressed, 30 + entry.name.length);
  return 30 + entry.name.length + entry.compressed.length;
}

function writeCentralZipEntry(
  output: Uint8Array,
  view: DataView,
  entry: ZipEntry,
  offset: number,
): number {
  view.setUint32(offset, 0x02014b50, true);
  view.setUint16(offset + 4, 0x0314, true);
  view.setUint16(offset + 6, 20, true);
  view.setUint16(offset + 8, 0x0800, true);
  view.setUint16(offset + 10, 8, true);
  view.setUint16(offset + 12, entry.timestamp.time, true);
  view.setUint16(offset + 14, entry.timestamp.date, true);
  view.setUint32(offset + 16, entry.checksum, true);
  view.setUint32(offset + 20, entry.compressed.length, true);
  view.setUint32(offset + 24, entry.uncompressedSize, true);
  view.setUint16(offset + 28, entry.name.length, true);
  view.setUint16(offset + 30, 0, true);
  view.setUint16(offset + 32, 0, true);
  view.setUint16(offset + 34, 0, true);
  view.setUint16(offset + 36, 0, true);
  view.setUint32(offset + 38, (0o100755 << 16) >>> 0, true);
  view.setUint32(offset + 42, 0, true);
  output.set(entry.name, offset + 46);
  return 46 + entry.name.length;
}

function writeZipEnd(
  view: DataView,
  offset: number,
  centralOffset: number,
  centralSize: number,
): void {
  view.setUint32(offset, 0x06054b50, true);
  view.setUint16(offset + 4, 0, true);
  view.setUint16(offset + 6, 0, true);
  view.setUint16(offset + 8, 1, true);
  view.setUint16(offset + 10, 1, true);
  view.setUint32(offset + 12, centralSize, true);
  view.setUint32(offset + 16, centralOffset, true);
  view.setUint16(offset + 20, 0, true);
}

export function createDeterministicZip(
  filename: string,
  contents: Uint8Array,
  epoch = sourceDateEpoch(),
): Uint8Array<ArrayBuffer> {
  const entry = createZipEntry(filename, contents, epoch);
  const localSize = 30 + entry.name.length + entry.compressed.length;
  const centralSize = 46 + entry.name.length;
  const output = new Uint8Array(localSize + centralSize + 22);
  const view = new DataView(output.buffer);
  const centralOffset = writeLocalZipEntry(output, view, entry);
  const writtenCentralSize = writeCentralZipEntry(output, view, entry, centralOffset);
  writeZipEnd(view, centralOffset + writtenCentralSize, centralOffset, writtenCentralSize);
  return output;
}
export function packageReleaseAsset(input: string, output: string): void {
  const file = path.resolve(input);
  const destination = path.resolve(output);
  const stats = statSync(file);
  if (!stats.isFile()) throw new Error(`release asset input is not a file: ${file}`);
  const contents = readFileSync(file);

  if (destination.endsWith(".tar.gz")) {
    const tar = createDeterministicTar(path.basename(file), contents);
    const compressed = Bun.gzipSync(tar, { level: 9 });
    const bytes = new Uint8Array(compressed.byteLength);
    bytes.set(compressed);
    writeFileSync(destination, bytes);
  } else if (destination.endsWith(".zip")) {
    writeFileSync(destination, createDeterministicZip(path.basename(file), contents));
  } else {
    throw new Error(`unsupported release archive: ${destination}`);
  }
}

function main(): void {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) {
    throw new Error(
      "usage: bun scripts/package-release-asset.ts <input> <output.tar.gz|output.zip>",
    );
  }
  packageReleaseAsset(input, output);
  process.stdout.write(`[release-asset] ${input} -> ${output}\n`);
}

if (import.meta.main) main();
