/**
 * Minimal argv parser — supports `--flag`, `--key=value`, `--key value`,
 * `-x`, and positional args. No external dependency.
 */
export interface ParsedArgs {
  flags:       Record<string, boolean | string>;
  positionals: string[];
  raw:         string[];
}

export function parseArgs(argv: string[] = process.argv.slice(2)): ParsedArgs {
  const flags:       Record<string, boolean | string> = {};
  const positionals: string[] = [];
  const raw = argv.slice();

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg.startsWith("--")) {
      const body = arg.slice(2);
      const eq = body.indexOf("=");
      if (eq >= 0) {
        flags[body.slice(0, eq)] = body.slice(eq + 1);
      } else {
        const next = argv[i + 1];
        if (next !== undefined && !next.startsWith("-")) {
          flags[body] = next;
          i++;
        } else {
          flags[body] = true;
        }
      }
    } else if (arg.startsWith("-") && arg.length > 1) {
      for (const ch of arg.slice(1)) flags[ch] = true;
    } else {
      positionals.push(arg);
    }
  }

  return { flags, positionals, raw };
}
