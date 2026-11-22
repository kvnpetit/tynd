/**
 * Theme manager — keeps the app's theme in sync with the OS preference,
 * lets the user override it, persists the choice via localStorage.
 * The OS preference is read through the host (`os.theme.current()` /
 * `theme:change` events) rather than `matchMedia`, so the same code works
 * inside backend contexts (no browser `matchMedia`) and stays
 * consistent with the host's `os.isDarkMode()`.
 * Works with CSS-variable palettes: pass `{ light, dark }` maps and the
 * client applies the matching one to `:root` every time the active theme
 * changes. Apps that ship Tailwind / CSS-in-JS can skip the palette and
 * just read `current()` / subscribe to `onChange`.
 */

import { osCall, osOn } from "@tynd/core/client/bridge";

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
  /** Bumped by every `configure()`/`dispose()`; lets a superseded configure bail out after its awaits. */
  generation: number;
  /** Whether this webview started the host watcher (so `dispose` only stops what it started). */
  watching: boolean;
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
  generation: 0,
  watching: false,
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
  // Concurrent calls (HMR): only the latest one may subscribe below.
  const generation = ++state.generation;

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

  // Start the host-side watcher (once per webview) and subscribe.
  if (generation !== state.generation) return;
  if (!state.watching) {
    state.watching = true;
    try {
      await osCall("theme", "startWatch", null);
    } catch {
      // The host counted no subscriber for this webview: a later dispose() must
      // not send stopWatch, which would drop another window's watcher.
      state.watching = false;
    }
  }
  if (generation !== state.generation) return;
  state.unsubscribeOs = osOn("theme:change", (raw) => {
    const evt = raw as { mode?: string };
    state.osMode = evt.mode === "dark" ? "dark" : "light";
    if (state.preference === "system") apply();
  });

  apply();
}

/** Unsubscribe from OS events + clear listeners. Call on teardown. */
function dispose(): void {
  state.generation++;
  state.unsubscribeOs?.();
  state.unsubscribeOs = null;
  state.listeners.clear();
  // Only stop a watcher this webview started: it is shared host-side, so a
  // secondary window that never called configure() must not stop the main one.
  if (!state.watching) return;
  state.watching = false;
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

function applyRootTheme(root: HTMLElement, next: EffectiveTheme): void {
  if (state.config.htmlClassName) {
    if (next === "dark") root.classList.add(state.config.htmlClassName);
    else root.classList.remove(state.config.htmlClassName);
  }

  // color-scheme ensures form controls (scrollbars, date pickers) match.
  root.style.colorScheme = next;

  if (state.config.palettes) {
    const pal = state.config.palettes[next];
    for (const [prop, value] of Object.entries(pal)) {
      root.style.setProperty(prop, value);
    }
  }
}

function notifyThemeListeners(next: EffectiveTheme): void {
  for (const listener of state.listeners) {
    try {
      listener(next);
    } catch {
      /* keep other subscribers going */
    }
  }
}

function apply(): void {
  const next: EffectiveTheme = state.preference === "system" ? state.osMode : state.preference;
  const changed = next !== state.effective;
  state.effective = next;

  if (state.config.root) applyRootTheme(state.config.root, next);
  if (changed) notifyThemeListeners(next);
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
 * Bundled namespace export so apps can `import { theme } from "@tynd/plugin-theme/client"`
 * and call `theme.configure()` / `theme.current()` / `theme.onChange()` —
 * matches the shape of the other os.* clients (autolaunch, keyring, …).
 */
/** System appearance snapshot from `theme.systemPreferences()`. */
export interface SystemPreferences {
  /** OS color scheme; `"unspecified"` when the OS has no preference. */
  mode: "light" | "dark" | "unspecified";
  /** Accent color as `"#rrggbb"`; `null` where the OS does not expose it (Linux). */
  accentColor: string | null;
  /** High-contrast mode (Windows) / "Increase contrast" (macOS); `null` on Linux. */
  highContrast: boolean | null;
  /** Animations turned off in the OS accessibility settings; `null` on Linux. */
  reducedMotion: boolean | null;
  /** Transparency effects turned off; `null` on Linux. */
  reducedTransparency: boolean | null;
}

/** Handle returned by `theme.onSystemPreferencesChange`. */
export interface ThemeSubscription {
  dispose(): void;
}

/** Read the OS appearance preferences (accent color, contrast, motion, transparency). */
function systemPreferences(): Promise<SystemPreferences> {
  return osCall("theme", "preferences", null);
}

/**
 * Subscribe to changes of `systemPreferences()` (the OS is polled once a
 * second). Resolves once the host watcher runs; rejects when it cannot
 * start. Each subscription holds one watcher reference until `dispose()`.
 */
async function onSystemPreferencesChange(
  handler: (prefs: SystemPreferences) => void,
): Promise<ThemeSubscription> {
  const off = osOn<SystemPreferences>("theme:preferences-change", (prefs) => handler(prefs));
  try {
    await osCall("theme", "startWatch", null);
  } catch (error) {
    off();
    throw error;
  }
  let active = true;
  return {
    dispose() {
      if (!active) return;
      active = false;
      off();
      void osCall("theme", "stopWatch", null).catch(() => {
        /* ignore: host may already have stopped */
      });
    },
  };
}

export const theme = {
  configure,
  dispose,
  preference,
  current,
  setPreference,
  toggle,
  onChange,
  systemPreferences,
  onSystemPreferencesChange,
};
