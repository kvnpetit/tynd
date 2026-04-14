import { EventEmitter } from "events";
import { resolveBinary } from "./host-resolve";
import { CLIENT_SCRIPT } from "./client-script";
import { serialize, deserialize } from "./binary";
import { materializeChannels } from "./channel";
import type {
  CommandMap, HostCmd, HostEvt, WindowConfig,
  TrayMenuItem, MenuItem,
  OpenFileOptions, SaveFileOptions, OpenDirectoryOptions,
  MonitorInfo, SystemInfo, CpuUsage, MemoryInfo,
  BatteryInfo, DiskInfo, NetworkInterface, GpuUsageInfo,
  TemperatureInfo, UsbDevice, AiCapabilities,
  NetworkSpeed, ProcessInfo, UserInfo, AudioDevice, DisplayInfo, CpuDetails, RamModule,
} from "./types";

export interface HostOptions {
  url:        string;
  window:     WindowConfig;
  commands:   CommandMap;
  staticDir?: string;  // production static files dir → zero-network scheme (no HTTP server)
}

export class WebviewHost extends EventEmitter {
  private proc: ReturnType<typeof Bun.spawn> | null = null;
  private _closed  = false;
  private readonly onClose: (() => void)[] = [];
  private pendingRequests = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  private reqId = 0;

  constructor(private readonly options: HostOptions) {
    super();
  }

  async launch(): Promise<void> {
    const binary = await resolveBinary(import.meta.dir);
    if (!binary) {
      await this.launchFallback();
      return;
    }

    const { window: win } = this.options;
    this.proc = Bun.spawn(
      [
        binary,
        `--title=${win.title ?? "Bunview App"}`,
        `--width=${win.width ?? 900}`,
        `--height=${win.height ?? 600}`,
        ...(win.debug ? ["--debug"] : []),
        ...(win.resizable === false ? ["--resizable=false"] : []),
        ...(win.frameless ? ["--frameless"] : []),
        ...(win.transparent ? ["--transparent"] : []),
        ...(win.icon ? [`--icon=${win.icon}`] : []),
        ...(win.vibrancy ? [`--vibrancy=${win.vibrancy}`] : []),
        ...(win.showMinimizeButton === false ? ["--no-minimize-btn"] : []),
        ...(win.showMaximizeButton === false ? ["--no-maximize-btn"] : []),
        ...(win.showCloseButton === false ? ["--no-close-btn"] : []),
        ...(win.hardwareAcceleration === true ? [] : ["--no-hw-accel"]),
        ...(this.options.staticDir ? [`--static-dir=${this.options.staticDir}`] : []),
      ],
      {
        stdin: "pipe",
        stdout: "pipe",
        stderr: "pipe",
      },
    );

    this.startStdoutReader();
    this.startStderrReader();

    await new Promise<void>((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("[bunview] Webview host timeout")), 10_000);
      this.once("ready", () => { clearTimeout(t); resolve(); });
    });
  }

  private startStderrReader() {
    const proc = this.proc!;
    (async () => {
      const stream = proc.stderr as ReadableStream<Uint8Array> | null;
      if (!stream) return;
      for await (const chunk of stream) {
        const msg = Buffer.from(chunk).toString("utf8").trim();
        if (msg) console.error(`[bunview:host] ${msg}`);
      }
    })();
  }

  private startStdoutReader() {
    const proc = this.proc!;
    const commands = this.options.commands;

    (async () => {
      const stream = proc.stdout as ReadableStream<Uint8Array> | null;
      if (!stream) return;

      let buf = "";
      for await (const chunk of stream) {
        buf += Buffer.from(chunk).toString("utf8");
        const lines = buf.split("\n");
        buf = lines.pop() ?? "";

        for (const raw of lines) {
          const line = raw.trim();
          if (!line) continue;
          let msg: HostEvt;
          try { msg = JSON.parse(line); } catch { continue; }
          this.handleHostEvent(msg, commands);
        }
      }
      this.triggerClose();
    })();
  }

  private async handleHostEvent(msg: HostEvt, commands: CommandMap) {
    if (msg.type === "ready") {
      // Inject the IPC bridge before the first navigation — runs on every page
      // load without any XHR fetch or HTTP server involvement.
      this.sendRaw({ type: "initScript", code: CLIENT_SCRIPT });
      this.sendRaw({ type: "navigate", url: this.options.url });
      this.emit("ready");

    } else if (msg.type === "invoke") {
      const handler = commands[msg.command];
      if (!handler) {
        this.sendRaw({
          type: "return", id: msg.id, status: 1,
          result: `Unknown command: "${msg.command}"`,
        });
        return;
      }
      try {
        // Deserialize binary markers → Uint8Array, then materialize any Channel
        // markers into live Channel instances that emit on their wire id.
        const decoded = deserialize(msg.payload);
        const payload = materializeChannels(decoded, (chId, data) => {
          this.emitEvent(chId, serialize(data));
        });
        const result = await handler(payload);
        this.sendRaw({ type: "return", id: msg.id, status: 0, result: serialize(result) });
      } catch (err) {
        this.sendRaw({
          type: "return", id: msg.id, status: 1,
          result: err instanceof Error ? err.message : String(err),
        });
      }

    } else if (msg.type === "response") {
      const pending = this.pendingRequests.get(msg.id);
      if (pending) {
        this.pendingRequests.delete(msg.id);
        if (msg.error) {
          pending.reject(new Error(msg.error));
        } else {
          pending.resolve(msg.result);
        }
      }

    } else if (msg.type === "trayClick") {
      this.emit("trayClick");

    } else if (msg.type === "trayMenuItemClick") {
      this.emit("trayMenuItemClick", msg.id);

    } else if (msg.type === "menuItemClick") {
      this.emit("menuItemClick", msg.id);

    } else if (msg.type === "contextMenuItemClick") {
      this.emit("contextMenuItemClick", msg.id);

    } else if (msg.type === "shortcutTriggered") {
      this.emit("shortcutTriggered", msg.id);

    } else if (msg.type === "windowMoved") {
      this.emit("windowMoved", { x: (msg as any).x, y: (msg as any).y });
    } else if (msg.type === "windowResized") {
      this.emit("windowResized", { width: (msg as any).width, height: (msg as any).height });
    } else if (msg.type === "windowFocusChanged") {
      this.emit("windowFocusChanged", (msg as any).focused);
    } else if (msg.type === "frontendEvent") {
      // Emitted by the frontend via window.__bv_emit__() — routed through C++ stdin
      this.emit("frontendEvent", msg.name, msg.payload);

    } else if (msg.type === "fileDrop") {
      this.emit("fileDrop", (msg as any).paths);

    } else if (msg.type === "fileDragEnter") {
      this.emit("fileDragEnter", (msg as any).paths);

    } else if (msg.type === "fileDragLeave") {
      this.emit("fileDragLeave");

    } else if (msg.type === "themeChanged") {
      this.emit("themeChanged", (msg as any).theme);

    } else if (msg.type === "hwMonitorUpdate") {
      this.emit("hwMonitorUpdate", msg as any);

    } else if (msg.type === "close") {
      this.triggerClose();
    }
  }

  private triggerClose() {
    if (this._closed) return;
    this._closed = true;
    this.emit("close");
    for (const cb of this.onClose) cb();
  }

  private sendRaw(cmd: HostCmd): void {
    const sink = this.proc?.stdin as import("bun").FileSink | undefined;
    if (!sink) return;
    sink.write(JSON.stringify(cmd) + "\n");
  }

  private sendRequest(cmd: HostCmd): Promise<unknown> {
    const id = String(++this.reqId);
    return new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`[bunview] Request ${cmd.type} timed out after 30s`));
      }, 30_000);

      this.pendingRequests.set(id, {
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject:  (e) => { clearTimeout(timer); reject(e); },
      });

      this.sendRaw({ ...cmd, id } as HostCmd);
    });
  }

  /** Fire a `{type, id: ""}` request; id is assigned in sendRequest(). */
  private request<T>(type: string): Promise<T> {
    return this.sendRequest({ type, id: "" } as unknown as HostCmd) as Promise<T>;
  }

  get url(): string          { return this.options.url; }
  get staticDir(): string | undefined { return this.options.staticDir; }

  navigate(url: string)                  { this.sendRaw({ type: "navigate", url }); }
  setTitle(title: string)                { this.sendRaw({ type: "setTitle", title }); }
  setSize(w: number, h: number)          { this.sendRaw({ type: "setSize", width: w, height: h }); }
  close()                                { this.sendRaw({ type: "terminate" }); }
  eval(code: string)                     { this.sendRaw({ type: "eval", code }); }
  /** Push a named event to the frontend via webview_eval — zero TCP overhead. */
  emitEvent(name: string, payload: unknown): void {
    this.sendRaw({ type: "event", name, payload });
  }
  minimize()                             { this.sendRaw({ type: "minimize" }); }
  maximize()                             { this.sendRaw({ type: "maximize" }); }
  restore()                              { this.sendRaw({ type: "restore" }); }
  fullscreen(enter = true)               { this.sendRaw({ type: "fullscreen", enter }); }
  center()                               { this.sendRaw({ type: "center" }); }
  setMinSize(w: number, h: number)       { this.sendRaw({ type: "setMinSize", width: w, height: h }); }
  setMaxSize(w: number, h: number)       { this.sendRaw({ type: "setMaxSize", width: w, height: h }); }
  setAlwaysOnTop(on: boolean)            { this.sendRaw({ type: "setAlwaysOnTop", on }); }
  hide()                                 { this.sendRaw({ type: "hide" }); }
  show()                                 { this.sendRaw({ type: "show" }); }

  openFile(options: OpenFileOptions = {}): Promise<string | string[] | null> {
    return this.sendRequest({ type: "openFile", id: "", options }) as Promise<string | string[] | null>;
  }

  saveFile(options: SaveFileOptions = {}): Promise<string | null> {
    return this.sendRequest({ type: "saveFile", id: "", options }) as Promise<string | null>;
  }

  openDirectory(options: OpenDirectoryOptions = {}): Promise<string | null> {
    return this.sendRequest({ type: "openDirectory", id: "", options }) as Promise<string | null>;
  }

  clipboardRead(): Promise<string> {
    return this.sendRequest({ type: "clipboardRead", id: "" }) as Promise<string>;
  }

  clipboardWrite(text: string): void {
    this.sendRaw({ type: "clipboardWrite", text });
  }

  notify(title: string, body: string, icon?: string): void {
    this.sendRaw({ type: "notify", title, body, icon });
  }

  trayCreate(tooltip?: string, icon?: string): void {
    this.sendRaw({ type: "trayCreate", tooltip, icon });
  }

  traySetMenu(items: TrayMenuItem[]): void {
    this.sendRaw({ type: "traySetMenu", items });
  }

  trayRemove(): void {
    this.sendRaw({ type: "trayRemove" });
  }

  menuSet(items: MenuItem[]): void {
    this.sendRaw({ type: "menuSet", items });
  }

  menuRemove(): void {
    this.sendRaw({ type: "menuRemove" });
  }

  getPosition(): Promise<{ x: number; y: number; width: number; height: number }> {
    return this.sendRequest({ type: "getPosition", id: "" }) as any;
  }

  setPosition(x: number, y: number): void {
    this.sendRaw({ type: "setPosition", x, y });
  }

  contextMenuShow(items: MenuItem[], x?: number, y?: number): void {
    this.sendRaw({ type: "contextMenuShow", id: "", items, x, y });
  }

  shortcutRegister(shortcutId: string, accelerator: string): void {
    this.sendRaw({ type: "shortcutRegister", shortcutId, accelerator });
  }

  shortcutUnregister(shortcutId: string): void {
    this.sendRaw({ type: "shortcutUnregister", shortcutId });
  }

  messageDialog(options: { title?: string; message: string; dialogType: string; defaultValue?: string }): Promise<string | boolean | null> {
    return this.sendRequest({ type: "messageDialog", id: "", title: options.title, message: options.message, dialogType: options.dialogType, defaultValue: options.defaultValue }) as Promise<string | boolean | null>;
  }

  setVibrancy(effect: string): void { this.sendRaw({ type: "setVibrancy", effect }); }
  setButtons(minimize: boolean, maximize: boolean, close: boolean): void {
    this.sendRaw({ type: "setButtons", minimize, maximize, close });
  }
  positionWindow(position: string, monitor?: number): void {
    this.sendRaw({ type: "positionWindow", position, monitor });
  }
  getMonitors(): Promise<MonitorInfo[]> {
    return this.sendRequest({ type: "getMonitors", id: "" }) as Promise<MonitorInfo[]>;
  }

  focus(): void                                    { this.sendRaw({ type: "setFocus" }); }
  setEnabled(enabled: boolean): void               { this.sendRaw({ type: "setEnabled", enabled }); }
  setDecorations(decorated: boolean): void         { this.sendRaw({ type: "setDecorations", decorated }); }
  setShadow(shadow: boolean): void                 { this.sendRaw({ type: "setShadow", shadow }); }
  setBackgroundColor(r: number, g: number, b: number, a = 255): void {
    this.sendRaw({ type: "setBackgroundColor", r, g, b, a });
  }
  setTitleBarStyle(style: "visible" | "transparent" | "overlay" | "hidden"): void {
    this.sendRaw({ type: "setTitleBarStyle", style });
  }

  setSkipTaskbar(skip: boolean): void {
    this.sendRaw({ type: "setSkipTaskbar", skip });
  }

  requestUserAttention(critical = false): void {
    this.sendRaw({ type: "requestUserAttention", critical });
  }

  getTheme(): Promise<"dark" | "light"> {
    return this.sendRequest({ type: "getTheme", id: "" }) as Promise<"dark" | "light">;
  }

  setContentProtected(protect: boolean): void {
    this.sendRaw({ type: "setContentProtected", protected: protect });
  }

  setAlwaysOnBottom(on: boolean): void {
    this.sendRaw({ type: "setAlwaysOnBottom", on });
  }

  clipboardWriteHtml(html: string, text?: string): void {
    this.sendRaw({ type: "clipboardWriteHtml", html, text });
  }

  clipboardClear(): void {
    this.sendRaw({ type: "clipboardClear" });
  }

  setProgressBar(progress: number | null): void {
    this.sendRaw({ type: "setProgressBar", progress });
  }

  setBadgeCount(count: number | null): void {
    this.sendRaw({ type: "setBadgeCount", count });
  }

  getSystemInfo()     { return this.request<SystemInfo>("getSystemInfo"); }
  getCpuUsage()       { return this.request<CpuUsage>("getCpuUsage"); }
  getMemoryInfo()     { return this.request<MemoryInfo>("getMemoryInfo"); }
  getBatteryInfo()    { return this.request<BatteryInfo>("getBatteryInfo"); }
  getDiskInfo()       { return this.request<DiskInfo[]>("getDiskInfo"); }
  getNetworkInfo()    { return this.request<NetworkInterface[]>("getNetworkInfo"); }
  getGpuUsage()       { return this.request<GpuUsageInfo[]>("getGpuUsage"); }
  getTemperature()    { return this.request<TemperatureInfo>("getTemperature"); }
  getUsbDevices()     { return this.request<UsbDevice[]>("getUsbDevices"); }
  getAiCapabilities() { return this.request<AiCapabilities>("getAiCapabilities"); }
  getNetworkSpeed()   { return this.request<NetworkSpeed[]>("getNetworkSpeed"); }
  getProcessList()    { return this.request<ProcessInfo[]>("getProcessList"); }
  getUsers()          { return this.request<UserInfo[]>("getUsers"); }
  getAudioDevices()   { return this.request<AudioDevice[]>("getAudioDevices"); }
  getDisplayInfo()    { return this.request<DisplayInfo[]>("getDisplayInfo"); }
  getCpuDetails()     { return this.request<CpuDetails>("getCpuDetails"); }
  getRamDetails()     { return this.request<RamModule[]>("getRamDetails"); }

  startHwMonitor(intervalMs = 1000): void { this.sendRaw({ type: "startHwMonitor", intervalMs }); }
  stopHwMonitor():                   void { this.sendRaw({ type: "stopHwMonitor" }); }

  waitForClose(): Promise<void> {
    if (this._closed) return Promise.resolve();
    return new Promise<void>((resolve) => { this.onClose.push(resolve); });
  }

  private async launchFallback(): Promise<void> {
    const { url, window: win } = this.options;
    const w = win.width  ?? 900;
    const h = win.height ?? 600;

    console.warn(
      "\n[bunview] ⚠  Native host binary not found — falling back to browser app mode.\n" +
      "          Run `bun packages/bunview/scripts/build-host.ts` to compile it.\n",
    );

    const args = [`--app=${url}`, `--window-size=${w},${h}`, "--no-first-run"];

    const candidates =
      process.platform === "win32"
        ? [
            "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
          ]
        : process.platform === "darwin"
        ? [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
          ]
        : ["google-chrome", "chromium-browser", "microsoft-edge"];

    for (const exe of candidates) {
      try {
        this.proc = Bun.spawn([exe, ...args], {
          stdin:  "pipe",
          stdout: "pipe",
          stderr: "pipe",
        });
        this.emit("ready");
        // Bun subprocess signals exit via the `exited` promise (no EventEmitter).
        this.proc.exited.then(() => this.triggerClose());

        // In fallback mode, native IPC isn't available — warn clearly
        console.warn(
          "[bunview] ⚠  In fallback mode, webview_bind IPC is NOT available.\n" +
          "          Compile the native host for full native IPC.\n",
        );
        return;
      } catch { /* try next */ }
    }

    throw new Error(
      "[bunview] No native host binary found and no compatible browser available.\n" +
      "          Build the host: bun packages/bunview/scripts/build-host.ts",
    );
  }
}
