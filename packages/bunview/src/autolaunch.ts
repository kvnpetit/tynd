import os from "os";
import path from "path";
import { writeFileSync, unlinkSync, existsSync, mkdirSync } from "fs";

export interface AutolaunchOptions {
  name: string;
  execPath?: string;
  args?: string[];
}

function resolveOptions(options: AutolaunchOptions) {
  return {
    name: options.name,
    execPath: options.execPath ?? process.execPath,
    args: options.args ?? [],
  };
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

const WIN_REG_KEY =
  "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";

async function winEnable(name: string, execPath: string, args: string[]) {
  const value =
    args.length > 0
      ? `"${execPath}" ${args.join(" ")}`
      : `"${execPath}"`;

  const proc = Bun.spawn([
    "reg",
    "add",
    WIN_REG_KEY,
    "/v",
    name,
    "/t",
    "REG_SZ",
    "/d",
    value,
    "/f",
  ]);
  await proc.exited;
  if (proc.exitCode !== 0) {
    throw new Error(`Failed to enable autolaunch for "${name}" (reg add exited with ${proc.exitCode})`);
  }
}

async function winDisable(name: string) {
  const proc = Bun.spawn([
    "reg",
    "delete",
    WIN_REG_KEY,
    "/v",
    name,
    "/f",
  ]);
  await proc.exited;
  if (proc.exitCode !== 0) {
    throw new Error(`Failed to disable autolaunch for "${name}" (reg delete exited with ${proc.exitCode})`);
  }
}

async function winIsEnabled(name: string): Promise<boolean> {
  const proc = Bun.spawn([
    "reg",
    "query",
    WIN_REG_KEY,
    "/v",
    name,
  ]);
  await proc.exited;
  return proc.exitCode === 0;
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

function macPlistPath(name: string): string {
  return path.join(os.homedir(), "Library", "LaunchAgents", `com.bunview.${name}.plist`);
}

function macPlistContent(name: string, execPath: string, args: string[]): string {
  const argsXml = [execPath, ...args]
    .map((a) => `    <string>${a}</string>`)
    .join("\n");

  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.bunview.${name}</string>
  <key>ProgramArguments</key>
  <array>
${argsXml}
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
`;
}

function macEnable(name: string, execPath: string, args: string[]) {
  const plistPath = macPlistPath(name);
  const dir = path.dirname(plistPath);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
  writeFileSync(plistPath, macPlistContent(name, execPath, args), "utf-8");
}

function macDisable(name: string) {
  const plistPath = macPlistPath(name);
  if (existsSync(plistPath)) {
    unlinkSync(plistPath);
  }
}

function macIsEnabled(name: string): boolean {
  return existsSync(macPlistPath(name));
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

function linuxDesktopPath(name: string): string {
  return path.join(os.homedir(), ".config", "autostart", `${name}.desktop`);
}

function linuxDesktopContent(name: string, execPath: string, args: string[]): string {
  const execLine =
    args.length > 0
      ? `${execPath} ${args.join(" ")}`
      : execPath;

  return `[Desktop Entry]
Type=Application
Name=${name}
Exec=${execLine}
X-GNOME-Autostart-enabled=true
`;
}

function linuxEnable(name: string, execPath: string, args: string[]) {
  const desktopPath = linuxDesktopPath(name);
  const dir = path.dirname(desktopPath);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
  writeFileSync(desktopPath, linuxDesktopContent(name, execPath, args), "utf-8");
}

function linuxDisable(name: string) {
  const desktopPath = linuxDesktopPath(name);
  if (existsSync(desktopPath)) {
    unlinkSync(desktopPath);
  }
}

function linuxIsEnabled(name: string): boolean {
  return existsSync(linuxDesktopPath(name));
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export async function enable(options: AutolaunchOptions): Promise<void> {
  const { name, execPath, args } = resolveOptions(options);
  const platform = process.platform;

  if (platform === "win32") {
    await winEnable(name, execPath, args);
  } else if (platform === "darwin") {
    macEnable(name, execPath, args);
  } else if (platform === "linux") {
    linuxEnable(name, execPath, args);
  } else {
    throw new Error(`Unsupported platform: ${platform}`);
  }
}

export async function disable(options: AutolaunchOptions): Promise<void> {
  const { name } = resolveOptions(options);
  const platform = process.platform;

  if (platform === "win32") {
    await winDisable(name);
  } else if (platform === "darwin") {
    macDisable(name);
  } else if (platform === "linux") {
    linuxDisable(name);
  } else {
    throw new Error(`Unsupported platform: ${platform}`);
  }
}

export async function isEnabled(options: AutolaunchOptions): Promise<boolean> {
  const { name } = resolveOptions(options);
  const platform = process.platform;

  if (platform === "win32") {
    return winIsEnabled(name);
  } else if (platform === "darwin") {
    return macIsEnabled(name);
  } else if (platform === "linux") {
    return linuxIsEnabled(name);
  } else {
    throw new Error(`Unsupported platform: ${platform}`);
  }
}
