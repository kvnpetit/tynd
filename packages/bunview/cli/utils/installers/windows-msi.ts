import { fileURLToPath } from "url";
import path from "path";
import type { FileAssociation } from "../../../src/types";
import { ensureWix, xmlEscape, deterministicGuid } from "./shared";

/**
 * MSI via WiX v3 (standalone — no .NET). UpgradeCode is deterministic from
 * `appName` so a new version triggers MajorUpgrade instead of coexisting.
 */
export async function generateWindowsMsi(exePath: string, appName: string, outDir: string, icoPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  const bunviewRoot = fileURLToPath(new URL("../../../", import.meta.url));
  const wix = await ensureWix(bunviewRoot);
  if (!wix) return;

  const wxsPath     = path.join(outDir, `${appName}.wxs`);
  const wixobjPath  = path.join(outDir, `${appName}.wixobj`);
  const msiPath     = path.join(outDir, `${appName}.msi`);
  const exeName     = path.basename(exePath);
  const upgradeCode = deterministicGuid(`bunview:${appName}:upgrade`);
  const productId   = deterministicGuid(`bunview:${appName}:product:1.0.0`);

  const regScheme = urlScheme ? `
        <RegistryKey Root="HKCU" Key="Software\\Classes\\${xmlEscape(urlScheme)}" Action="createAndRemoveOnUninstall">
          <RegistryValue Type="string" Value="URL:${xmlEscape(appName)} Protocol" />
          <RegistryValue Name="URL Protocol" Type="string" Value="" />
        </RegistryKey>
        <RegistryKey Root="HKCU" Key="Software\\Classes\\${xmlEscape(urlScheme)}\\shell\\open\\command" Action="createAndRemoveOnUninstall">
          <RegistryValue Type="string" Value='"[INSTALLFOLDER]${exeName}" "%1"' />
        </RegistryKey>` : "";

  const regAssoc = (fileAssociations ?? []).map((a) => `
        <RegistryKey Root="HKCU" Key="Software\\Classes\\.${xmlEscape(a.ext)}" Action="createAndRemoveOnUninstall">
          <RegistryValue Type="string" Value="${xmlEscape(appName)}.${xmlEscape(a.ext)}" />
        </RegistryKey>
        <RegistryKey Root="HKCU" Key="Software\\Classes\\${xmlEscape(appName)}.${xmlEscape(a.ext)}" Action="createAndRemoveOnUninstall">
          <RegistryValue Type="string" Value="${xmlEscape(a.name)}" />
        </RegistryKey>
        <RegistryKey Root="HKCU" Key="Software\\Classes\\${xmlEscape(appName)}.${xmlEscape(a.ext)}\\shell\\open\\command" Action="createAndRemoveOnUninstall">
          <RegistryValue Type="string" Value='"[INSTALLFOLDER]${exeName}" "%1"' />
        </RegistryKey>`).join("");

  const wxs = `<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="${productId}" Name="${xmlEscape(appName)}" Language="1033" Version="1.0.0.0"
           Manufacturer="bunview" UpgradeCode="${upgradeCode}">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perUser" InstallPrivileges="limited" />
    <MajorUpgrade DowngradeErrorMessage="A newer version of ${xmlEscape(appName)} is already installed." />
    <Media Id="1" Cabinet="app.cab" EmbedCab="yes" />
    ${icoPath ? `<Icon Id="AppIcon.exe" SourceFile="${icoPath}" />
    <Property Id="ARPPRODUCTICON" Value="AppIcon.exe" />` : ""}

    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="INSTALLFOLDER" Name="${xmlEscape(appName)}" />
      </Directory>
      <Directory Id="ProgramMenuFolder" />
      <Directory Id="DesktopFolder" />
    </Directory>

    <DirectoryRef Id="INSTALLFOLDER">
      <Component Id="MainExe" Guid="${deterministicGuid(`bunview:${appName}:exe`)}">
        <File Id="MainExeFile" Source="${exePath}" KeyPath="yes">
          <Shortcut Id="StartMenuShortcut" Directory="ProgramMenuFolder" Name="${xmlEscape(appName)}"
                    WorkingDirectory="INSTALLFOLDER" Advertise="yes" />
          <Shortcut Id="DesktopShortcut" Directory="DesktopFolder" Name="${xmlEscape(appName)}"
                    WorkingDirectory="INSTALLFOLDER" Advertise="yes" />
        </File>
      </Component>
      <Component Id="AppRegistry" Guid="${deterministicGuid(`bunview:${appName}:reg`)}">
        <RegistryValue Root="HKCU" Key="Software\\${xmlEscape(appName)}" Name="installed" Type="integer" Value="1" KeyPath="yes" />${regScheme}${regAssoc}
      </Component>
    </DirectoryRef>

    <Feature Id="Main" Title="${xmlEscape(appName)}" Level="1">
      <ComponentRef Id="MainExe" />
      <ComponentRef Id="AppRegistry" />
    </Feature>
  </Product>
</Wix>`;

  await Bun.write(wxsPath, wxs);

  let proc = Bun.spawn([wix.candle, "-nologo", "-out", wixobjPath, wxsPath], { stdout: "pipe", stderr: "pipe" });
  if ((await proc.exited) !== 0) {
    console.error(`[bunview] ❌ candle failed:\n${await new Response(proc.stderr).text()}`);
    try { (await import("fs")).unlinkSync(wxsPath); } catch {}
    return;
  }

  proc = Bun.spawn([wix.light, "-nologo", "-sw1076", "-out", msiPath, wixobjPath], { stdout: "pipe", stderr: "pipe" });
  const code = await proc.exited;

  for (const f of [wxsPath, wixobjPath, msiPath.replace(/\.msi$/, ".wixpdb")]) {
    try { (await import("fs")).unlinkSync(f); } catch {}
  }

  if (code !== 0) {
    console.error(`[bunview] ❌ light failed:\n${await new Response(proc.stderr).text()}`);
    return;
  }
  if (await Bun.file(msiPath).exists()) {
    const size = (Bun.file(msiPath).size / 1024 / 1024).toFixed(1);
    console.log(`  → ${path.relative(process.cwd(), msiPath)} (${size} MB)`);
  }
}
