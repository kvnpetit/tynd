import type { FileAssociation, CommandMap } from "../types";

export interface DispatchConfig {
  schemeName?: string;
  schemeHandler?: string;
  fileAssociations?: FileAssociation[];
  singleInstanceHandler?: string;
}

/** Safe no-op if the command is missing. */
export async function invokeHandler(
  commands: CommandMap,
  name: string,
  payload: unknown,
): Promise<void> {
  const cmd = commands[name];
  if (!cmd) {
    console.warn(`[bunview] Handler "${name}" not found in commands.`);
    return;
  }
  try { await cmd(payload); } catch (err) {
    console.error(`[bunview] Handler "${name}" threw:`, err);
  }
}

export function validateHandlers(commands: CommandMap, cfg: DispatchConfig): void {
  const check = (name: string | undefined, where: string) => {
    if (name && !(name in commands)) {
      console.warn(`[bunview] ⚠ Handler "${name}" declared in ${where} is not a command.`);
    }
  };
  check(cfg.schemeHandler,         "urlScheme.handler");
  check(cfg.singleInstanceHandler, "singleInstance.handler");
  for (const assoc of cfg.fileAssociations ?? []) {
    check(assoc.handler, `fileAssociations["${assoc.ext}"].handler`);
  }
}

/** Invokes the declarative handler + imperative callbacks for any matching argv entry. */
export async function dispatchFromArgv(
  argv: string[],
  commands: CommandMap,
  cfg: DispatchConfig,
  callbacks: {
    onDeepLink?:  (url: string)  => void;
    onFileOpen?:  (path: string) => void;
  } = {},
): Promise<void> {
  const { schemeName, schemeHandler, fileAssociations = [] } = cfg;

  for (const arg of argv) {
    if (arg.startsWith("-")) continue;

    if (schemeName && arg.startsWith(`${schemeName}://`)) {
      if (schemeHandler) await invokeHandler(commands, schemeHandler, arg);
      try { callbacks.onDeepLink?.(arg); } catch {}
      continue;
    }

    if (fileAssociations.length === 0) continue;
    const lower = arg.toLowerCase();
    for (const assoc of fileAssociations) {
      if (lower.endsWith(`.${assoc.ext.toLowerCase()}`)) {
        await invokeHandler(commands, assoc.handler, arg);
        try { callbacks.onFileOpen?.(arg); } catch {}
        break;
      }
    }
  }
}
