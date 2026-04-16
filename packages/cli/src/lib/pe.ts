/**
 * Minimal Windows PE utilities for vorn build post-processing.
 */

import { readFileSync, writeFileSync } from "node:fs"

// Windows PE subsystem constants
const SUBSYSTEM_WINDOWS = 2

/**
 * Patch a Windows PE binary to use the GUI subsystem (no console window).
 * Does nothing silently on non-Windows builds or non-PE files.
 */
export function patchPeSubsystem(filePath: string): void {
  if (process.platform !== "win32") return

  let buf: Buffer
  try {
    buf = readFileSync(filePath)
  } catch (e) {
    console.error(`patchPeSubsystem: could not read "${filePath}": ${e}`)
    return
  }

  // Verify MZ signature
  if (buf.length < 0x40 || buf[0] !== 0x4d || buf[1] !== 0x5a) {
    console.error(`patchPeSubsystem: invalid MZ signature in "${filePath}", skipping patch`)
    return
  }

  // e_lfanew at offset 0x3C → offset to PE signature
  const peOffset = buf.readUInt32LE(0x3c)
  if (peOffset + 26 > buf.length) {
    console.error(`patchPeSubsystem: PE offset out of bounds in "${filePath}", skipping patch`)
    return
  }

  // Verify PE\0\0 signature
  if (buf[peOffset] !== 0x50 || buf[peOffset + 1] !== 0x45) {
    console.error(`patchPeSubsystem: invalid PE signature in "${filePath}", skipping patch`)
    return
  }

  // Optional header starts at peOffset + 4 (PE sig) + 20 (COFF header)
  // Subsystem field is at offset 68 within the Optional Header
  const subsystemOffset = peOffset + 24 + 68
  if (subsystemOffset + 2 > buf.length) {
    console.error(
      `patchPeSubsystem: subsystem offset out of bounds in "${filePath}", skipping patch`,
    )
    return
  }

  const current = buf.readUInt16LE(subsystemOffset)
  if (current === SUBSYSTEM_WINDOWS) return // already GUI, nothing to do

  buf.writeUInt16LE(SUBSYSTEM_WINDOWS, subsystemOffset)
  writeFileSync(filePath, buf)
}
