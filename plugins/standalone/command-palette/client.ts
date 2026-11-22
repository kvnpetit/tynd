/**
 * Headless command-palette registry: typed command store + default
 * Cmd/Ctrl+K shortcut + open/close signal. No components shipped — apps
 * render their own UI on top of `search()` / `run()`.
 */

/** Single command entry registered in the palette. */
export interface Command {
  /** Stable, unique ID — used for updates + unregister. */
  id: string;
  /** Title shown in the palette list. */
  title: string;
  /** Optional subtitle (e.g. section, module). */
  subtitle?: string;
  /** Synonyms that make the command findable by related terms. */
  keywords?: readonly string[];
  /** Optional icon name/emoji — UI decides how to render. */
  icon?: string;
  /** Optional group/category for UI grouping. */
  group?: string;
  /** Displayed on the right (shortcut hint, version, etc.). */
  hint?: string;
  /** Called when the user picks this entry. Async results awaited. */
  action: () => void | Promise<void>;
  /** Dynamic visibility — evaluated on each search. */
  when?: () => boolean;
}

/** Command with the fuzzy-match score from `search`. */
export interface SearchResult extends Command {
  /** 0 = perfect match, higher = worse. */
  score: number;
}

interface ShortcutBinding {
  handler: (e: KeyboardEvent) => void;
  target: EventTarget;
}

interface State {
  commands: Map<string, Command>;
  open: boolean;
  openListeners: Set<(open: boolean) => void>;
  keyHandler: ShortcutBinding | null;
}

const state: State = {
  commands: new Map(),
  open: false,
  openListeners: new Set(),
  keyHandler: null,
};

/** Overwrites an existing entry with the same `id`. */
export function register(cmd: Command): void {
  state.commands.set(cmd.id, cmd);
}

/** Register a batch of commands; later registrations with the same id win. */
export function registerMany(cmds: readonly Command[]): void {
  for (const c of cmds) state.commands.set(c.id, c);
}

/** No-op on unknown id. */
export function unregister(id: string): void {
  state.commands.delete(id);
}

/** Drop every registered command. */
export function clear(): void {
  state.commands.clear();
}

/** Snapshot of every currently-registered command in registration order. */
export function list(): readonly Command[] {
  return [...state.commands.values()];
}

/**
 * Score 0 = exact title, 1 = title startsWith, 2 = title contains,
 * 3 = subtitle / keyword contains, 4 = subsequence in title. Anything
 * else is filtered out. Empty query returns visible commands in
 * insertion order.
 */
function commandScore(command: Command, query: string): number | null {
  if (!query) return 0;
  const title = command.title.toLowerCase();
  if (title === query) return 0;
  if (title.startsWith(query)) return 1;
  if (title.includes(query)) return 2;

  const subtitle = command.subtitle?.toLowerCase() ?? "";
  const keywords = (command.keywords ?? []).join(" ").toLowerCase();
  if (subtitle.includes(query) || keywords.includes(query)) return 3;
  return isSubsequence(query, title) ? 4 : null;
}

/** A `when()` that throws hides its command instead of aborting the caller. */
function isVisible(command: Command): boolean {
  if (!command.when) return true;
  try {
    return command.when();
  } catch {
    return false;
  }
}

/** Visible commands matching `query`, best score first, capped at `limit`. */
export function search(query: string, limit = 50): SearchResult[] {
  const normalized = query.trim().toLowerCase();
  const results: SearchResult[] = [];
  for (const command of state.commands.values()) {
    if (!isVisible(command)) continue;
    const score = commandScore(command, normalized);
    if (score !== null) results.push({ ...command, score });
  }
  results.sort((left, right) => left.score - right.score);
  return results.slice(0, limit);
}

/** Returns `true` if the command ran; closes the palette in either
 *  branch (success or thrown action) so a buggy action can't leave the
 *  palette stuck open. Re-throws so callers still see the error. */
export async function run(id: string): Promise<boolean> {
  const cmd = state.commands.get(id);
  if (!cmd) return false;
  if (!isVisible(cmd)) return false;
  try {
    await cmd.action();
  } finally {
    close();
  }
  return true;
}

/** Whether the palette is currently considered open. */
export function isOpen(): boolean {
  return state.open;
}

/** Mark the palette as open and notify listeners. */
export function open(): void {
  setOpen(true);
}

/** Mark the palette as closed and notify listeners. */
export function close(): void {
  setOpen(false);
}

/** Toggle the open state and notify listeners. */
export function toggle(): void {
  setOpen(!state.open);
}

/** Returns an unsubscribe fn. */
export function onOpenChange(handler: (open: boolean) => void): () => void {
  state.openListeners.add(handler);
  return () => state.openListeners.delete(handler);
}

function setOpen(next: boolean): void {
  if (next === state.open) return;
  state.open = next;
  for (const l of state.openListeners) {
    try {
      l(next);
    } catch {
      // keep others going
    }
  }
}

/** Options for `bindShortcut`. */
export interface ShortcutOptions {
  /** Default `"k"` (Cmd+K / Ctrl+K). */
  key?: string;
  /** Default `true` — matches Cmd on mac, Ctrl elsewhere. */
  metaOrCtrl?: boolean;
  /** Default `toggle()`. */
  onTrigger?: () => void;
  /** Default `window`. */
  target?: EventTarget | null;
}

/** Returns an unsubscribe fn. */
export function bindShortcut(opts: ShortcutOptions = {}): () => void {
  const key = (opts.key ?? "k").toLowerCase();
  const metaOrCtrl = opts.metaOrCtrl !== false;
  const target = opts.target ?? window;
  const trigger = opts.onTrigger ?? toggle;

  const handler = (e: KeyboardEvent) => {
    // Autofill / IME keydowns can arrive without a `key`.
    if (typeof e.key !== "string" || e.key.toLowerCase() !== key) return;
    if (metaOrCtrl) {
      if (!(e.metaKey || e.ctrlKey)) return;
    } else if (isEditableTarget(e.target)) {
      // A bare-key binding must not hijack typing in form fields.
      return;
    }
    e.preventDefault();
    trigger();
  };

  // Drop any previous handler so HMR / re-binds don't leak listeners.
  if (state.keyHandler) {
    state.keyHandler.target.removeEventListener(
      "keydown",
      state.keyHandler.handler as EventListener,
    );
  }
  const binding: ShortcutBinding = { handler, target };
  state.keyHandler = binding;
  target.addEventListener("keydown", handler as EventListener);
  return () => {
    target.removeEventListener("keydown", handler as EventListener);
    if (state.keyHandler === binding) state.keyHandler = null;
  };
}

function isEditableTarget(target: unknown): boolean {
  if (typeof target !== "object" || target === null) return false;
  const element = target as { tagName?: unknown; isContentEditable?: unknown };
  if (element.isContentEditable === true) return true;
  if (typeof element.tagName !== "string") return false;
  const tag = element.tagName.toUpperCase();
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

function isSubsequence(needle: string, haystack: string): boolean {
  let i = 0;
  for (let j = 0; j < haystack.length && i < needle.length; j++) {
    if (haystack[j] === needle[i]) i++;
  }
  return i === needle.length;
}
