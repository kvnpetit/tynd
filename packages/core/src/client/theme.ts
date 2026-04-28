/**
 * Theme manager — keeps the app's theme in sync with the OS preference,
 * lets the user override it, persists the choice via localStorage.
 *
 * Promoted from the (pure-JS) `@tynd/plugin-theme`. The DOM-side palette
 * application is unchanged; the only difference is the OS preference is
 * now read through the host (`os.theme.current()` / `theme:change`
 * events) rather than a `matchMedia` query — this means the same code
 * works inside the lite runtime (no `matchMedia` in QuickJS) and stays
 * consistent with the host's `os.isDarkMode()`.
 *
 * Works with CSS-variable palettes: pass `{ light, dark }` maps and the
 * client applies the matching one to `:root` every time the active theme
 * changes. Apps that ship Tailwind / CSS-in-JS can skip the palette and
 * just read `current()` / subscribe to `onChange`.
 */

import { osCall } from "./_internal";

export type ThemePreference = "light" | "dark" | "system";
export type EffectiveTheme = "light" | "dark";

/** CSS variable map — `{ "--bg": "#fff", "--fg": "#111" }`. */
export type Palette = Record<string, string>;

interface Config {
  palettes?: { light: Palette; dark: Palette };
  /** localStorage key — override when embedding multiple Tynd apps in the same origin. */
  storageKey: string;
  /** `class="dark"` on `<html>` when active. Compatible with Tailwind's `dark:` variant. */
  htmlClassName: string | null;
  /** DOM element to mutate. Defaults to `document.documentElement`. */
  root: HTMLElement | null;
}

const state: {
  config: Config;
  preference: ThemePreference;
  effective: EffectiveTheme;
  listeners: Set<(t: EffectiveTheme) => void>;
  osMode: EffectiveTheme;
  unsubscribeOs: (() => void) | null;
} = {
  config: {
    palettes: undefined,
    storageKey: "tynd:theme",
    htmlClassName: "dark",
    root: null,
  },
  preference: "system",
  effective: "light",
  listeners: new Set(),
  osMode: "light",
  unsubscribeOs: null,
};

export interface ConfigureOptions {
  palettes?: { light: Palette; dark: Palette };
  storageKey?: string;
  /** `null` = don't toggle any class. Defaults to `"dark"`. */
  htmlClassName?: string | null;
  root?: HTMLElement;
}

/**
 * One-time setup. Reads the saved preference from localStorage, asks the
 * host for the current OS theme, subscribes to `theme:change`, applies
 * the resolved theme immediately. Idempotent — calling again replaces
 * the prior subscription.
 */
async function configure(opts: ConfigureOptions = {}): Promise<void> {
  // Drop any previous OS subscription so calling configure() twice
  // (e.g. during Vite HMR) doesn't stack handlers.
  state.unsubscribeOs?.();
  state.unsubscribeOs = null;

  state.config.palettes = opts.palettes;
  state.config.storageKey = opts.storageKey ?? "tynd:theme";
  state.config.htmlClassName = opts.htmlClassName === undefined ? "dark" : opts.htmlClassName;
  state.config.root =
    opts.root ?? (typeof document !== "undefined" ? document.documentElement : null);

  state.preference = loadPreference(state.config.storageKey);

  // Initial OS snapshot — defaults to "light" if the host or call fails.
  try {
    const mode = await osCall<string>("theme", "current", null);
    state.osMode = mode === "dark" ? "dark" : "light";
  } catch {
    state.osMode = "light";
  }

  // Start the host-side watcher and subscribe.
  try {
    await osCall("theme", "startWatch", null);
  } catch {
    /* watcher already running or host pre-1.x — fine */
  }
  state.unsubscribeOs = window.__tynd__.os_on("theme:change", (raw) => {
    const evt = raw as { mode?: string };
    state.osMode = evt.mode === "dark" ? "dark" : "light";
    if (state.preference === "system") apply();
  });

  apply();
}

/** Unsubscribe from OS events + clear listeners. Call on teardown. */
function dispose(): void {
  state.unsubscribeOs?.();
  state.unsubscribeOs = null;
  state.listeners.clear();
  // Best-effort stop — a sibling app on the same host might still want it,
  // but the host's startWatch is idempotent so a re-configure simply re-arms.
  void osCall("theme", "stopWatch", null).catch(() => {
    /* ignore — host may already have stopped */
  });
}

/** Current user-visible preference — may be `"system"`. */
function preference(): ThemePreference {
  return state.preference;
}

/** Resolved theme (`"system"` -> whichever the OS is currently). */
function current(): EffectiveTheme {
  return state.effective;
}

/** Set the user preference. Persists to localStorage and re-applies. */
function setPreference(p: ThemePreference): void {
  if (p === state.preference) return;
  state.preference = p;
  savePreference(state.config.storageKey, p);
  apply();
}

/** Quick toggle for explicit light/dark — ignores `"system"`. */
function toggle(): void {
  setPreference(state.effective === "dark" ? "light" : "dark");
}

/** Subscribe to effective-theme changes. Returns unsubscribe. */
function onChange(handler: (t: EffectiveTheme) => void): () => void {
  state.listeners.add(handler);
  return () => state.listeners.delete(handler);
}

function apply(): void {
  const next: EffectiveTheme = state.preference === "system" ? state.osMode : state.preference;
  const changed = next !== state.effective;
  state.effective = next;

  if (state.config.root && state.config.htmlClassName) {
    if (next === "dark") state.config.root.classList.add(state.config.htmlClassName);
    else state.config.root.classList.remove(state.config.htmlClassName);
  }

  if (state.config.root) {
    // color-scheme ensures form controls (scrollbars, date pickers) match.
    state.config.root.style.colorScheme = next;
  }

  if (state.config.palettes && state.config.root) {
    const pal = state.config.palettes[next];
    for (const [prop, value] of Object.entries(pal)) {
      state.config.root.style.setProperty(prop, value);
    }
  }

  if (changed) {
    for (const l of state.listeners) {
      try {
        l(next);
      } catch {
        /* keep other subscribers going */
      }
    }
  }
}

function loadPreference(key: string): ThemePreference {
  try {
    const raw = localStorage.getItem(key);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    /* private-mode storage blocked */
  }
  return "system";
}

function savePreference(key: string, value: ThemePreference): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore storage failures */
  }
}

/**
 * Bundled namespace export so apps can `import { theme } from "@tynd/core/client"`
 * and call `theme.configure()` / `theme.current()` / `theme.onChange()` —
 * matches the shape of the other os.* clients (autolaunch, keyring, …).
 */
export const theme = {
  configure,
  dispose,
  preference,
  current,
  setPreference,
  toggle,
  onChange,
};
