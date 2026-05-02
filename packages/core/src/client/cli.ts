import { osCall } from "./_internal";

/**
 * Parsed shape of `process.argv`. Mirrors the Rust `CliMatches` struct
 * in `host-rs::os::cli`.
 */
export interface CliMatches {
  /** Positional args after the subcommand. Order preserved. */
  args: string[];
  /**
   * `--name=value` → `{ name: "value" }`, `--flag` → `{ flag: true }`,
   * `--name value` → `{ name: "value" }`. Repeated keys overwrite.
   */
  flags: Record<string, string | boolean>;
  /** First non-flag argument after `argv[0]`, when present. */
  subcommand: string | null;
  /** Verbatim argv snapshot, including `argv[0]` (the binary path). */
  raw: string[];
}

let cached: Promise<CliMatches> | null = null;

/**
 * `cli` — read the host process's command-line arguments.
 *
 * Argv is captured once at the host's first call and parsed lazily; the
 * frontend additionally caches the parsed `Promise` so repeated
 * `cli.matches()` calls don't roundtrip through the IPC bridge.
 */
export const cli = {
  /** Parsed argv (cached). */
  matches(): Promise<CliMatches> {
    cached ??= osCall<CliMatches>("cli", "matches", {});
    return cached;
  },
  /** Untouched argv snapshot (one IPC call per invocation — rarely used). */
  raw(): Promise<string[]> {
    return osCall("cli", "raw", {});
  },
};
