import path from "path";
import { BunviewServer } from "./server";
import { WebviewHost  } from "./host";
import { AppPaths } from "./paths";
import { OsInfo } from "./os-info";
import { KeyValueStore } from "./store";
import { Logger, type LoggerOptions } from "./logger";
import { watchPath, type FsEvent } from "./fs-watch";
import { parseArgs, type ParsedArgs } from "./cli-args";
import type {
  AppConfig, BunviewConfig, CommandMap, EventMap,
  WindowOptions, WindowHandle,
  SecondInstancePayload, FileAssociation,
} from "./types";
import { loadRuntimeConfig } from "./load-config";
import { acquireLock, watchTrigger } from "./lifecycle/single-instance";
import { windowStatePath, loadWindowState, saveWindowState } from "./lifecycle/window-state";
import { validateHandlers, dispatchFromArgv, invokeHandler } from "./lifecycle/deep-links";
import { windowFacade } from "./facades/window";
import { dialogFacade } from "./facades/dialog";
import { clipboardFacade } from "./facades/clipboard";
import { trayFacade, menuFacade, contextMenuFacade } from "./facades/menus";
import { shortcutFacade } from "./facades/shortcut";
import { hardwareFacade } from "./facades/hardware";

function shouldUseScheme(devUrl: string | null): boolean {
  if (devUrl) return false;
  if ((globalThis as any).__BUNVIEW_EMBEDDED__) return false;
  return true;
}

function platformAppUrl(): string {
  return "bv://localhost/index.html";
}

function resolveStaticDir(entry: string): string {
  const abs = path.isAbsolute(entry) ? entry : path.join(process.cwd(), entry);
  return abs.endsWith(".html") ? path.dirname(abs) : abs;
}

/** Detach from the Windows console so double-clicking the .exe doesn't flash a terminal. */
async function detachWindowsConsole(): Promise<void> {
  if (process.platform !== "win32") return;
  try {
    const { dlopen, FFIType } = await import("bun:ffi");
    const { symbols } = dlopen("kernel32.dll", {
      FreeConsole: { args: [], returns: FFIType.bool },
    });
    symbols.FreeConsole();
  } catch { /* FFI unavailable or already detached */ }
}

export class BunviewApp<
  TCommands extends CommandMap,
  TEvents extends EventMap = {},
> {
  private server?: BunviewServer;
  private host!: WebviewHost;
  private readyCbs: (() => void)[] = [];
  private closeCbs: (() => void)[] = [];
  private secondInstanceCbs: ((p: SecondInstancePayload) => void)[] = [];
  private deepLinkCbs: ((url: string) => void)[] = [];
  private fileOpenCbs: ((p: string) => void)[] = [];
  private frontendEventHandlers = new Map<string, Set<(payload: unknown) => void>>();
  private _windows = new Set<WebviewHost>();

  private resolvedCommands!: TCommands;
  private mergedConfig!: AppConfig<TCommands, TEvents> & { fileAssociations?: FileAssociation[] };

  constructor(private readonly config: AppConfig<TCommands, TEvents>) {}

  private mergeConfig(file: BunviewConfig | null): typeof this.mergedConfig {
    if (!file) return this.config as typeof this.mergedConfig;
    return {
      ...this.config,
      entry:            this.config.entry          ?? file.entry,
      window:           { ...(file.window ?? {}),    ...(this.config.window ?? {}) },
      windowState:      this.config.windowState     ?? file.windowState,
      singleInstance:   this.config.singleInstance  ?? file.singleInstance,
      urlScheme:        this.config.urlScheme       ?? file.urlScheme,
      fileAssociations: this.config.fileAssociations ?? file.fileAssociations,
    } as typeof this.mergedConfig;
  }

  private schemeName():    string | undefined { return this.mergedConfig?.urlScheme?.name; }
  private schemeHandler(): string | undefined { return this.mergedConfig?.urlScheme?.handler; }
  private singleInstanceHandler(): string | undefined {
    const s = this.mergedConfig?.singleInstance;
    return typeof s === "object" ? s?.handler : undefined;
  }

  onReady(cb: () => void): this { this.readyCbs.push(cb); return this; }
  onClose(cb: () => void): this { this.closeCbs.push(cb); return this; }
  onSecondInstance(cb: (payload: SecondInstancePayload) => void): this { this.secondInstanceCbs.push(cb); return this; }
  onDeepLink(cb: (url: string) => void): this                          { this.deepLinkCbs.push(cb);       return this; }
  onFileOpen(cb: (p: string) => void): this                            { this.fileOpenCbs.push(cb);       return this; }

  /** Subscribe to an event emitted by the frontend. */
  on<K extends string>(event: K, handler: (payload: unknown) => void): this {
    if (!this.frontendEventHandlers.has(event)) this.frontendEventHandlers.set(event, new Set());
    this.frontendEventHandlers.get(event)!.add(handler);
    return this;
  }

  async run(): Promise<void> {
    await detachWindowsConsole();

    const fileConfig = await loadRuntimeConfig();
    this.mergedConfig = this.mergeConfig(fileConfig);

    const cmds = this.config.commands;
    this.resolvedCommands = (typeof cmds === "function"
      ? await (cmds as () => TCommands | Promise<TCommands>)()
      : cmds) as TCommands;

    validateHandlers(this.resolvedCommands, {
      schemeName:            this.schemeName(),
      schemeHandler:         this.schemeHandler(),
      fileAssociations:      this.mergedConfig.fileAssociations,
      singleInstanceHandler: this.singleInstanceHandler(),
    });

    const triggerPath = await this.acquireInstanceLock();
    const { webviewUrl, staticDir, useScheme, port, devUrl } = await this.resolveUrl();
    const windowConfig = this.prepareWindowConfig();

    this.host = new WebviewHost({
      url:       webviewUrl,
      window:    windowConfig,
      commands:  this.resolvedCommands,
      staticDir: useScheme ? staticDir : undefined,
    });

    this.wireHostEvents();
    this.wireInitialAppearance();
    this.wireWindowStatePersistence();

    await this.host.launch();

    void this.dispatchArgv(process.argv);

    if (triggerPath) this.watchForSecondInstance(triggerPath);
    this.restoreSavedWindowPosition();
    this.logStartup(devUrl, useScheme, webviewUrl, port);

    await this.host.waitForClose();
    this.server?.stop();
    process.exit(0);
  }

  private async acquireInstanceLock(): Promise<string | null> {
    if (!this.mergedConfig.singleInstance) return null;
    const lock = await acquireLock(this.mergedConfig.window?.title ?? "bunview", process.argv);
    if (!lock.acquired) {
      console.log("[bunview] Forwarded to existing instance. Exiting.");
      process.exit(0);
    }
    return lock.triggerPath;
  }

  private async resolveUrl() {
    const entry     = this.mergedConfig.entry ?? this.config.entry!;
    const staticDir = resolveStaticDir(entry);
    const devUrl    = process.env.BUNVIEW_DEV_URL || null;
    const useScheme = shouldUseScheme(devUrl);

    let port = 0;
    if (!useScheme) {
      this.server = new BunviewServer(staticDir, this.config.port ?? 0);
      port = await this.server.start();
    }
    const webviewUrl = devUrl ?? (useScheme ? platformAppUrl() : `http://localhost:${port}`);
    return { webviewUrl, staticDir, useScheme, port, devUrl };
  }

  private prepareWindowConfig() {
    const cfg = { ...(this.mergedConfig.window ?? {}) };
    if (!this.mergedConfig.windowState) return cfg;

    const saved = loadWindowState(windowStatePath(cfg.title ?? "bunview"));
    if (saved.width)  cfg.width  = saved.width;
    if (saved.height) cfg.height = saved.height;
    return cfg;
  }

  private wireHostEvents(): void {
    for (const cb of this.readyCbs) this.host.on("ready", cb);
    for (const cb of this.closeCbs) this.host.on("close", cb);
    this.host.on("frontendEvent", (name: string, payload: unknown) => {
      const handlers = this.frontendEventHandlers.get(name);
      if (handlers) for (const h of handlers) h(payload);
    });
  }

  /** Options that only apply after the window is ready. */
  private wireInitialAppearance(): void {
    const win = this.config.window ?? {};
    if (win.decorations === false) this.host.on("ready", () => this.host.setDecorations(false));
    if (win.shadow      === false) this.host.on("ready", () => this.host.setShadow(false));
    if (win.backgroundColor)       this.host.on("ready", () => {
      const c = win.backgroundColor!;
      this.host.setBackgroundColor(c.r, c.g, c.b, c.a ?? 255);
    });
    if (win.titleBarStyle)         this.host.on("ready", () => this.host.setTitleBarStyle(win.titleBarStyle!));
  }

  private wireWindowStatePersistence(): void {
    if (!this.mergedConfig.windowState) return;
    const statePath = windowStatePath(this.mergedConfig.window?.title ?? "bunview");
    this.host.on("close", async () => {
      try {
        const pos = await this.host.getPosition();
        saveWindowState(statePath, pos);
      } catch { /* ignore save errors */ }
    });
  }

  private restoreSavedWindowPosition(): void {
    if (!this.mergedConfig.windowState) return;
    const saved = loadWindowState(windowStatePath(this.mergedConfig.window?.title ?? "bunview"));
    if (saved.x != null && saved.y != null) this.host.setPosition(saved.x, saved.y);
  }

  private logStartup(devUrl: string | null, useScheme: boolean, url: string, port: number): void {
    if (devUrl)         console.log(`[bunview] dev mode → ${devUrl} (HMR enabled)`);
    else if (useScheme) console.log(`[bunview] app running → ${url} (zero-network mode)`);
    else                console.log(`[bunview] app running → http://localhost:${port}`);
    console.log(`[bunview] IPC bridge → injected via webview_init (no HTTP)`);
  }

  private dispatchArgv(argv: string[]): Promise<void> {
    return dispatchFromArgv(argv, this.resolvedCommands, {
      schemeName:       this.schemeName(),
      schemeHandler:    this.schemeHandler(),
      fileAssociations: this.mergedConfig?.fileAssociations,
    }, {
      onDeepLink: (url) => { for (const cb of this.deepLinkCbs) { try { cb(url); } catch {} } },
      onFileOpen: (p)   => { for (const cb of this.fileOpenCbs) { try { cb(p);   } catch {} } },
    });
  }

  private watchForSecondInstance(triggerPath: string): void {
    watchTrigger(triggerPath, async (payload) => {
      this.host?.show();
      this.host?.restore();
      this.host?.focus();

      const siHandler = this.singleInstanceHandler();
      if (siHandler) await invokeHandler(this.resolvedCommands, siHandler, payload);

      for (const cb of this.secondInstanceCbs) { try { cb(payload); } catch {} }
      await this.dispatchArgv(payload.argv ?? []);
    });
  }

  /** Emit a typed event to the frontend. */
  emit<K extends keyof TEvents & string>(event: K, payload: TEvents[K]): void {
    this.host?.emitEvent(event, payload);
  }

  eval(code: string): void { this.host?.eval(code); }

  /** Cross-platform standard dirs (data, config, cache, logs, downloads…). Auto-created on access. */
  get paths(): AppPaths { return new AppPaths(this.config.window?.title ?? "bunview"); }

  /** OS/runtime introspection. */
  get os(): OsInfo { return new OsInfo(); }

  private _store?:  KeyValueStore;
  private _logger?: Logger;

  /** Persistent JSON key-value store at `paths.data()/store.json`. */
  get store(): KeyValueStore {
    if (!this._store) this._store = new KeyValueStore(path.join(this.paths.data(), "store.json"));
    return this._store;
  }

  get log(): Logger {
    if (!this._logger) this._logger = this.createLogger();
    return this._logger;
  }

  createLogger(opts: LoggerOptions = {}): Logger {
    this._logger = new Logger({ dir: this.paths.logs(), ...opts });
    return this._logger;
  }

  /** Returns an unwatch function. */
  watch(target: string, handler: (event: FsEvent) => void, opts?: { recursive?: boolean }): () => void {
    return watchPath(target, handler, opts);
  }

  get cliArgs(): ParsedArgs { return parseArgs(); }

  exit(code = 0): never {
    try { this.host?.close(); } catch {}
    process.exit(code);
  }

  /** Spawn a fresh instance of this process, then exit. */
  relaunch(): never {
    const child = Bun.spawn([process.execPath, ...process.argv.slice(1)], {
      stdin:  "ignore",
      stdout: "ignore",
      stderr: "ignore",
      cwd: process.cwd(),
    });
    child.unref();
    this.exit(0);
  }

  get window()      { return windowFacade(this.host); }
  get dialog()      { return dialogFacade(this.host); }
  get clipboard()   { return clipboardFacade(this.host); }
  get tray()        { return trayFacade(this.host); }
  get menu()        { return menuFacade(this.host); }
  get contextMenu() { return contextMenuFacade(this.host); }
  get shortcut()    { return shortcutFacade(this.host); }
  get hardware()    { return hardwareFacade(this.host); }

  notify(title: string, body: string, icon?: string): void { this.host?.notify(title, body, icon); }

  /** Each window is a separate OS process sharing the same `commands` surface. */
  async createWindow(options: WindowOptions = {}): Promise<WindowHandle> {
    const { resolveBinary } = await import("./host-resolve");
    const binary = await resolveBinary(import.meta.dir);
    if (!binary) throw new Error("[bunview] Cannot create window: webview-host not found");

    const { url: optUrl, ...windowOpts } = options;
    const url = optUrl ?? this.host?.url ?? "about:blank";

    const winHost = new WebviewHost({
      url,
      window: { title: "Bunview", width: 800, height: 600, ...windowOpts },
      commands:  this.resolvedCommands,
      staticDir: this.host?.staticDir,
    });

    await winHost.launch();
    this._windows.add(winHost);

    let closed = false;
    winHost.on("close", () => {
      closed = true;
      this._windows.delete(winHost);
    });

    return {
      get closed() { return closed; },
      close:          () => winHost.close(),
      setTitle:       (t) => winHost.setTitle(t),
      setSize:        (w, h) => winHost.setSize(w, h),
      setPosition:    (x, y) => winHost.setPosition(x, y),
      minimize:       () => winHost.minimize(),
      maximize:       () => winHost.maximize(),
      restore:        () => winHost.restore(),
      show:           () => winHost.show(),
      hide:           () => winHost.hide(),
      center:         () => winHost.center(),
      setFocus:       () => winHost.focus(),
      fullscreen:     (enter = true) => winHost.fullscreen(enter),
      setAlwaysOnTop: (on) => winHost.setAlwaysOnTop(on),
      navigate:       (u) => winHost.navigate(u),
      eval:           (code) => winHost.eval(code),
      emit:           (event, payload) => winHost.emitEvent(event, payload),
      onClose: (cb) => {
        winHost.on("close", cb);
        return () => winHost.off("close", cb);
      },
    };
  }

  get windows(): readonly WebviewHost[] { return [...this._windows]; }

  onFileDrop(cb: (paths: string[]) => void): this           { return this.onHostEvent("fileDrop", cb); }
  onFileDragEnter(cb: (paths: string[]) => void): this      { return this.onHostEvent("fileDragEnter", cb); }
  onFileDragLeave(cb: () => void): this                     { return this.onHostEvent("fileDragLeave", cb); }
  onThemeChanged(cb: (theme: "dark" | "light") => void): this { return this.onHostEvent("themeChanged", cb); }

  /** Queues the subscription on ready if the host isn't up yet. */
  private onHostEvent(event: string, cb: (...args: any[]) => void): this {
    if (this.host) this.host.on(event, cb);
    else this.readyCbs.push(() => this.host.on(event, cb));
    return this;
  }
}

export function createApp<
  TCommands extends CommandMap,
  TEvents   extends EventMap = {},
>(config: AppConfig<TCommands, TEvents>): BunviewApp<TCommands, TEvents> {
  return new BunviewApp(config);
}
