import { open } from "fs/promises";

/**
 * Patch the Windows PE Subsystem field from CONSOLE (3) to WINDOWS GUI (2)
 * so the executable does not open a terminal window on double-click.
 *
 * PE layout (PE32 and PE32+ share the same Subsystem offset):
 *   0x3C                     → 4-byte LE offset to "PE\0\0" signature
 *   peOffset + 4             → IMAGE_FILE_HEADER (20 bytes)
 *   peOffset + 24            → IMAGE_OPTIONAL_HEADER
 *   peOffset + 24 + 68 = 92  → Subsystem (WORD, 2 bytes LE)
 *
 * Only reads the first 4 KB of the file (enough to reach the header),
 * then writes exactly 2 bytes in-place — never rewrites the whole binary.
 */
export async function patchWindowsSubsystem(exePath: string): Promise<void> {
  const fd = await open(exePath, "r+");
  try {
    // Read enough to cover the PE header (0x3C + 4 + 24 + 68 < 256 bytes, 4 KB is plenty)
    const header = Buffer.alloc(4096);
    const { bytesRead } = await fd.read(header, 0, 4096, 0);

    if (bytesRead < 0x40) throw new Error(`File too small to be a PE: ${exePath}`);

    const peOffset = header.readUInt32LE(0x3C);
    if (peOffset + 92 + 2 > bytesRead) throw new Error(`PE header out of range: ${exePath}`);
    if (header.toString("ascii", peOffset, peOffset + 4) !== "PE\0\0") {
      throw new Error(`Not a valid PE file: ${exePath}`);
    }

    const subsystemOffset = peOffset + 4 + 20 + 68; // = peOffset + 92
    const current = header.readUInt16LE(subsystemOffset);

    if (current === 2) return; // Already IMAGE_SUBSYSTEM_WINDOWS_GUI — nothing to do

    // Write exactly 2 bytes (little-endian 2 = IMAGE_SUBSYSTEM_WINDOWS_GUI)
    await fd.write(Buffer.from([0x02, 0x00]), 0, 2, subsystemOffset);
  } finally {
    await fd.close();
  }
}
