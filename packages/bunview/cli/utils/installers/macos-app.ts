import { mkdirSync } from "fs";
import path from "path";
import type { FileAssociation } from "../../../src/types";

/** `.app` bundle with icon, URL scheme, and file-type associations embedded in Info.plist. */
export async function generateMacAppBundle(exePath: string, appName: string, outDir: string, icnsPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  const appDir       = path.join(outDir, `${appName}.app`);
  const contentsDir  = path.join(appDir, "Contents");
  const resourcesDir = path.join(contentsDir, "Resources");
  const macosDir     = path.join(contentsDir, "MacOS");
  mkdirSync(macosDir, { recursive: true });
  mkdirSync(resourcesDir, { recursive: true });

  const { copyFileSync, writeFileSync, chmodSync } = await import("fs");
  const binPath = path.join(macosDir, appName);
  copyFileSync(exePath, binPath);
  chmodSync(binPath, 0o755);

  if (icnsPath) {
    copyFileSync(icnsPath, path.join(resourcesDir, "app.icns"));
  }

  const urlTypes = urlScheme ? `
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>com.bunview.${appName.toLowerCase()}.${urlScheme}</string>
      <key>CFBundleURLSchemes</key><array><string>${urlScheme}</string></array>
    </dict>
  </array>` : "";

  // LSHandlerRank=Alternate → candidate handler without stealing the user's default.
  const docTypes = fileAssociations?.length ? `
  <key>CFBundleDocumentTypes</key>
  <array>
${fileAssociations.map(a => `    <dict>
      <key>CFBundleTypeExtensions</key><array><string>${a.ext}</string></array>
      <key>CFBundleTypeName</key><string>${a.name}</string>
      <key>CFBundleTypeRole</key><string>${a.role ?? "Editor"}</string>
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>`).join("\n")}
  </array>` : "";

  writeFileSync(path.join(contentsDir, "Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>${appName}</string>
  <key>CFBundleIdentifier</key><string>com.bunview.${appName.toLowerCase()}</string>
  <key>CFBundleName</key><string>${appName}</string>
  ${icnsPath ? `<key>CFBundleIconFile</key><string>app.icns</string>` : ""}
  <key>CFBundleVersion</key><string>1.0.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>${urlTypes}${docTypes}
</dict>
</plist>`);

  console.log(`[bunview] ✅ .app bundle: ${path.relative(process.cwd(), appDir)}`);
}
