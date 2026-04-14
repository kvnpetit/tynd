import path from "path";
import fs from "fs";
import os from "os";

export interface SavedWindowState {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

/** Resolve the persistence path for a given app title. */
export function windowStatePath(appTitle: string): string {
  const safe     = appTitle.replace(/[^a-zA-Z0-9_-]/g, "-");
  const stateDir = path.join(os.homedir(), ".bunview");
  return path.join(stateDir, `${safe}-window-state.json`);
}

/** Read persisted window state. Returns an empty object when nothing is saved. */
export function loadWindowState(statePath: string): SavedWindowState {
  try {
    const raw = fs.readFileSync(statePath, "utf-8");
    return JSON.parse(raw) as SavedWindowState;
  } catch {
    return {};
  }
}

/** Atomically persist window state. Silently ignores I/O errors. */
export function saveWindowState(statePath: string, state: SavedWindowState): void {
  try {
    fs.mkdirSync(path.dirname(statePath), { recursive: true });
    fs.writeFileSync(statePath, JSON.stringify(state, null, 2));
  } catch { /* best-effort persistence */ }
}
