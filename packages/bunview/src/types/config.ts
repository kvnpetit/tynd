import type { CommandMap, EventMap } from "./rpc";

/** OS file-type registration. Handler points to an exported command. */
export interface FileAssociation<TCommands extends CommandMap = CommandMap> {
  /** Extension without the leading dot (e.g. `"md"`). */
  ext: string;
  name: string;
  handler: keyof TCommands & string;
  /** MIME type — required on Linux for MIME-handler registration. */
  mimeType?: string;
  /** macOS role — defaults to `"Editor"`. */
  role?: "Editor" | "Viewer";
}

export interface UrlScheme<TCommands extends CommandMap = CommandMap> {
  /** Scheme name (e.g. `"myapp"` → `myapp://…`). */
  name: string;
  handler?: keyof TCommands & string;
}

export type SingleInstance<TCommands extends CommandMap = CommandMap> =
  | boolean
  | { enabled: true; handler?: keyof TCommands & string };

export interface WindowConfig {
  title?:       string;
  width?:       number;
  height?:      number;
  resizable?:   boolean;
  debug?:       boolean;
  frameless?:   boolean;
  transparent?: boolean;
  icon?:        string;
  vibrancy?: "mica" | "acrylic" | "tabbed" | "dark" | "light" | "none";
  showMinimizeButton?: boolean;
  showMaximizeButton?: boolean;
  showCloseButton?: boolean;
  decorations?: boolean;
  shadow?: boolean;
  backgroundColor?: { r: number; g: number; b: number; a: number };
  /** macOS only — no-op on Windows/Linux. */
  titleBarStyle?: "visible" | "transparent" | "overlay" | "hidden";
  /** Required for WebGPU / Transformers.js. Default: false (software rendering). */
  hardwareAcceleration?: boolean;
}

export type WindowPosition = "topLeft" | "topCenter" | "topRight" | "centerLeft" | "center" | "centerRight" | "bottomLeft" | "bottomCenter" | "bottomRight" | "trayLeft" | "trayRight";

export interface WindowOptions extends WindowConfig {
  url?: string;
}

export interface WindowHandle {
  readonly closed: boolean;
  close(): void;
  setTitle(title: string): void;
  setSize(width: number, height: number): void;
  setPosition(x: number, y: number): void;
  minimize(): void;
  maximize(): void;
  restore(): void;
  show(): void;
  hide(): void;
  center(): void;
  setFocus(): void;
  fullscreen(enter?: boolean): void;
  setAlwaysOnTop(on: boolean): void;
  navigate(url: string): void;
  eval(code: string): void;
  emit(event: string, payload: unknown): void;
  /** Returns an unsubscribe fn. */
  onClose(cb: () => void): () => void;
}

export interface BunviewConfig<TCommands extends CommandMap = CommandMap> {
  entry: string;
  frontend?: string;
  icon?: string;
  /** Output binary name — defaults to package.json `name` or `"app"`. */
  name?: string;
  /** Output directory — defaults to `"release"`. */
  outDir?: string;
  window?: WindowConfig;
  windowState?: boolean;
  /** Override auto-detected dev server. */
  dev?: { url: string; command: string };
  urlScheme?: UrlScheme<TCommands>;
  fileAssociations?: FileAssociation<TCommands>[];
  singleInstance?: SingleInstance<TCommands>;
  /** String values support `${ENV_VAR}` interpolation. Skipped if the current platform is not configured. */
  codeSigning?: {
    windows?: {
      certificate: string;
      password?: string;
      timestampUrl?: string;
      description?: string;
    };
    macos?: {
      identity: string;
      entitlements?: string;
      hardenedRuntime?: boolean;
      notarize?: {
        appleId: string;
        teamId: string;
        password: string;
      };
    };
  };
}

export interface AppConfig<
  TCommands extends CommandMap = CommandMap,
  TEvents   extends EventMap   = EventMap,
> {
  entry?:    string;
  /** Plain module or lazy thunk (`() => import("./commands")`) for deferred loading. */
  commands:  TCommands | (() => TCommands | Promise<TCommands>);
  events?:   TEvents;
  window?:   WindowConfig;
  port?:     number;
  singleInstance?: SingleInstance<TCommands>;
  windowState?: boolean;
  urlScheme?: UrlScheme<TCommands>;
  fileAssociations?: FileAssociation<TCommands>[];
}

/** Payload delivered to `onSecondInstance`. */
export interface SecondInstancePayload {
  argv: string[];
  cwd:  string;
  timestamp: number;
}
