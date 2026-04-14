import { readFileSync, writeFileSync } from "fs";
import { NtExecutable, NtExecutableResource, Resource, Data } from "resedit";

/**
 * Inject an icon into a Windows executable using resedit (pure JS).
 * Falls back to rcedit.exe on failure (Bun-compiled binaries append data past
 * the PE sections, which resedit sometimes refuses to parse).
 */
export async function injectIcon(exePath: string, icoPath: string): Promise<void> {
  try {
    const exeBuffer = readFileSync(exePath);
    const exe = NtExecutable.from(exeBuffer);
    const res = NtExecutableResource.from(exe);

    const icoBuffer = readFileSync(icoPath);
    const iconFile = Data.IconFile.from(icoBuffer);
    const icons = iconFile.icons.map((i) => i.data);

    // Resource ID 1 / lang 1033 (en-US) — standard slot for the main app icon.
    Resource.IconGroupEntry.replaceIconsForResource(res.entries, 1, 1033, icons);

    res.outputResource(exe);
    const newExeBuffer = exe.generate();
    writeFileSync(exePath, Buffer.from(newExeBuffer));
  } catch (err) {
    // resedit fails on Bun-compiled binaries (appended data after PE sections).
    // rcedit.exe is more tolerant. Only log the root cause in debug mode.
    if (process.env.BUNVIEW_DEBUG) {
      console.warn(`[bunview] resedit failed: ${err instanceof Error ? err.message : err}`);
    }
    await fallbackToRcedit(exePath, icoPath);
  }
}

async function fallbackToRcedit(exePath: string, icoPath: string) {
  const { fileURLToPath } = await import("url");
  const path = await import("path");
  const { existsSync, mkdirSync } = await import("fs");

  const bunviewRoot = fileURLToPath(new URL("../../", import.meta.url));
  const depsDir   = path.join(bunviewRoot, "host", ".deps");
  const rceditExe = path.join(depsDir, "rcedit-x64.exe");

  if (!existsSync(rceditExe)) {
    console.log(`[bunview] Downloading rcedit (PE resource editor)…`);
    mkdirSync(depsDir, { recursive: true });
    const url = "https://github.com/electron/rcedit/releases/download/v2.0.0/rcedit-x64.exe";
    const res = await fetch(url);
    if (res.ok) await Bun.write(rceditExe, await res.arrayBuffer());
  }

  if (existsSync(rceditExe)) {
    await Bun.spawn([rceditExe, exePath, "--set-icon", icoPath], { stdout: "pipe", stderr: "pipe" }).exited;
  }
}
