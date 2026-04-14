import { existsSync, mkdirSync, readFileSync, writeFileSync } from "fs";
import path from "path";
import png2icons from "png2icons";
import { Resvg } from "@resvg/resvg-js";
import type { BunviewConfig } from "../../src/types";

export interface ResolvedIcons {
  ico?: string;
  icns?: string;
  png?: string;
}

/**
 * Automatically find the app icon (favicon.*, icon.png, etc.) based on frontend conventions.
 */
export async function resolveIconPath(cwd: string, config: BunviewConfig): Promise<string | null> {
  const candidates: string[] = [];

  if (config.icon) {
    candidates.push(path.resolve(cwd, config.icon));
  }

  if (config.frontend) {
    const frontAbs = path.resolve(cwd, config.frontend);
    candidates.push(path.join(frontAbs, "favicon.svg"));
    candidates.push(path.join(frontAbs, "favicon.png"));
    candidates.push(path.join(frontAbs, "favicon.ico"));
    candidates.push(path.join(frontAbs, "icon.png"));
    candidates.push(path.join(frontAbs, "logo.png"));
  }

  candidates.push(path.join(cwd, "public", "favicon.svg"));
  candidates.push(path.join(cwd, "public", "favicon.png"));
  candidates.push(path.join(cwd, "public", "favicon.ico"));
  candidates.push(path.join(cwd, "public", "icon.png"));

  candidates.push(path.join(cwd, "favicon.svg"));
  candidates.push(path.join(cwd, "favicon.png"));
  candidates.push(path.join(cwd, "favicon.ico"));

  for (const p of candidates) {
    if (existsSync(p)) return p;
  }

  return null;
}

/**
 * Generate native icon files (.ico, .icns, .png) from a source image.
 * Uses a temp directory for generated files.
 */
export async function generateNativeIcons(sourcePath: string, tmpDir: string): Promise<ResolvedIcons> {
  const ext = path.extname(sourcePath).toLowerCase();
  let pngBuffer: Buffer;

  if (ext === ".svg") {
    // Render SVG to PNG (1024x1024 for maximum quality icons)
    const svgData = readFileSync(sourcePath);
    const resvg   = new Resvg(svgData, { fitTo: { mode: "width", value: 1024 } });
    pngBuffer     = resvg.render().asPng();
  } else if (ext === ".png") {
    pngBuffer = readFileSync(sourcePath);
  } else if (ext === ".ico") {
    return { ico: sourcePath };
  } else {
    throw new Error(`Unsupported icon format: ${ext}. Use .png or .svg.`);
  }

  mkdirSync(tmpDir, { recursive: true });
  const result: ResolvedIcons = {};

  const icoData = (png2icons as any).createICO(pngBuffer, (png2icons as any).BICUBIC, 0, false, true);
  if (icoData) {
    const icoPath = path.join(tmpDir, "app.ico");
    writeFileSync(icoPath, icoData);
    result.ico = icoPath;
  }

  const icnsData = (png2icons as any).createICNS(pngBuffer, (png2icons as any).BICUBIC, 0);
  if (icnsData) {
    const icnsPath = path.join(tmpDir, "app.icns");
    writeFileSync(icnsPath, icnsData);
    result.icns = icnsPath;
  }

  const pngPath = path.join(tmpDir, "app.png");
  writeFileSync(pngPath, pngBuffer);
  result.png = pngPath;

  return result;
}
