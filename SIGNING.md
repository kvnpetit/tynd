# Code signing & notarization

Tynd produces raw binaries and installers via `tynd build [--bundle]`, but **does not sign them for you**. This doc covers the standard post-build signing workflows for each host OS so you can ship apps that users (and their OSes) trust.

> **Why sign?** Unsigned binaries trigger SmartScreen warnings on Windows, are quarantined by Gatekeeper on macOS, and are flagged by Chrome/Edge on download. Signing + notarizing eliminates those prompts.

---

## Windows — Authenticode signing

### 1. Get a certificate

Pick one:

| Type | Provider examples | Cost / yr | SmartScreen prompt |
|---|---|---|---|
| **EV code-signing** (hardware token) | DigiCert, Sectigo, GlobalSign | ~$300-600 | removed immediately |
| Standard code-signing (OV) | Certum, Sectigo, SSL.com | ~$100-250 | removed after reputation builds (1-2 months of downloads) |
| Free for open source | [SignPath.io](https://signpath.io) (Foundation tier) | $0 | OV-level |

EV certs ship on a USB token (YubiKey or SafeNet); you can't export the key. Standard certs come as a `.pfx`.

### 2. Sign the binaries

Use **signtool** (ships with the Windows SDK) after `tynd build`:

```powershell
# Raw binary
signtool sign `
  /fd SHA256 `
  /tr http://timestamp.digicert.com `
  /td SHA256 `
  /f mycert.pfx /p $env:CERT_PASSWORD `
  release\my-app.exe

# NSIS / MSI installers (run after --bundle)
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
  /f mycert.pfx /p $env:CERT_PASSWORD `
  release\my-app-1.0.0-setup.exe `
  release\my-app-1.0.0-x64.msi
```

**Always include a timestamp (`/tr`)** — without it, the signature expires when the cert does.

Verify:

```powershell
signtool verify /pa /v release\my-app.exe
```

### 3. CI (GitHub Actions)

```yaml
- name: Sign Windows binaries
  if: runner.os == 'Windows'
  env:
    CERT_BASE64:   ${{ secrets.WINDOWS_PFX_BASE64 }}
    CERT_PASSWORD: ${{ secrets.WINDOWS_PFX_PASSWORD }}
  run: |
    [IO.File]::WriteAllBytes("cert.pfx", [Convert]::FromBase64String($env:CERT_BASE64))
    & "C:/Program Files (x86)/Windows Kits/10/bin/10.0.22621.0/x64/signtool.exe" sign `
      /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
      /f cert.pfx /p $env:CERT_PASSWORD `
      release/*.exe release/*.msi
```

For EV certs, use a cloud-signing service (DigiCert KeyLocker, SSL.com eSigner) — hardware tokens are incompatible with CI.

---

## macOS — codesign + notarize + staple

### 1. Get an Apple Developer ID

- Enroll in the [Apple Developer Program](https://developer.apple.com/programs/) ($99/yr, individual or org).
- In Xcode or the web console, create a **Developer ID Application** certificate and a **Developer ID Installer** certificate.
- Export both as `.p12` (with password) for CI use.

### 2. Sign the `.app` and `.dmg`

```bash
# Sign the app bundle (after `tynd build --bundle app,dmg`)
codesign --deep --force --options runtime \
  --entitlements entitlements.plist \
  --sign "Developer ID Application: Your Name (TEAMID)" \
  release/MyApp.app

# Sign the DMG
codesign --force --sign "Developer ID Application: Your Name (TEAMID)" \
  release/MyApp-1.0.0.dmg
```

Minimal `entitlements.plist` (WebView needs JIT):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.cs.allow-jit</key>             <true/>
    <key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
    <key>com.apple.security.cs.disable-library-validation</key>     <true/>
    <key>com.apple.security.network.client</key>            <true/>
</dict>
</plist>
```

### 3. Notarize

Notarization is a second check by Apple after signing — required for Gatekeeper to accept the app without warnings:

```bash
# Store app-specific password in keychain once:
xcrun notarytool store-credentials "notarytool-profile" \
  --apple-id "you@example.com" \
  --team-id "TEAMID" \
  --password "app-specific-password"

# Notarize the DMG (the .app inside gets notarized transitively)
xcrun notarytool submit release/MyApp-1.0.0.dmg \
  --keychain-profile "notarytool-profile" \
  --wait

# Staple the notarization ticket to the DMG so it works offline
xcrun stapler staple release/MyApp-1.0.0.dmg
xcrun stapler staple release/MyApp.app
```

Generate an **app-specific password** at [appleid.apple.com](https://appleid.apple.com) → Sign-in and security → App-specific passwords.

### 4. Verify

```bash
spctl --assess -vv release/MyApp.app
# Expected: "source=Notarized Developer ID"

codesign --verify --deep --strict --verbose=2 release/MyApp.app
```

### 5. CI (GitHub Actions)

```yaml
- name: Import certs
  if: runner.os == 'macOS'
  env:
    CERT_BASE64: ${{ secrets.MACOS_DEV_ID_P12 }}
    CERT_PASSWORD: ${{ secrets.MACOS_DEV_ID_PASSWORD }}
    KEYCHAIN_PASSWORD: ${{ secrets.KEYCHAIN_PASSWORD }}
  run: |
    echo $CERT_BASE64 | base64 --decode > cert.p12
    security create-keychain -p "$KEYCHAIN_PASSWORD" build.keychain
    security default-keychain -s build.keychain
    security unlock-keychain -p "$KEYCHAIN_PASSWORD" build.keychain
    security import cert.p12 -k build.keychain -P "$CERT_PASSWORD" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" build.keychain

- name: Sign + notarize
  if: runner.os == 'macOS'
  env:
    APPLE_ID: ${{ secrets.APPLE_ID }}
    APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
    APPLE_APP_PASSWORD: ${{ secrets.APPLE_APP_SPECIFIC_PASSWORD }}
  run: |
    codesign --deep --force --options runtime \
      --entitlements entitlements.plist \
      --sign "Developer ID Application: Your Org (${APPLE_TEAM_ID})" \
      release/MyApp.app
    codesign --force \
      --sign "Developer ID Application: Your Org (${APPLE_TEAM_ID})" \
      release/MyApp-1.0.0.dmg
    xcrun notarytool submit release/MyApp-1.0.0.dmg \
      --apple-id "$APPLE_ID" \
      --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" \
      --wait
    xcrun stapler staple release/MyApp-1.0.0.dmg
    xcrun stapler staple release/MyApp.app
```

---

## Linux — GPG signing + repository trust

Linux has no centralized signature-verification model like Gatekeeper. The two practical paths are:

### 1. Detached `.sig` files for manual verification

```bash
gpg --armor --detach-sign release/my-app-1.0.0.AppImage
gpg --armor --detach-sign release/my-app-1.0.0.deb
gpg --armor --detach-sign release/my-app-1.0.0.rpm

# Users verify with:
gpg --verify my-app-1.0.0.AppImage.asc my-app-1.0.0.AppImage
```

Publish the public key alongside the downloads (e.g. on your GitHub Release).

### 2. Signed `.deb` / `.rpm` repositories

- **Debian/APT**: sign packages with `dpkg-sig --sign builder file.deb` and the `Release` file of your APT repo with `gpg --clearsign`. Users add your key via `apt-key` or drop it in `/etc/apt/trusted.gpg.d/`.
- **RPM/dnf**: sign with `rpm --addsign file.rpm` (needs `%_gpg_name` set in `~/.rpmmacros`). Users import your key with `rpm --import`.

For most desktop apps shipping via GitHub Releases, **path 1 (detached `.sig`) is enough** — serious distros will package you themselves anyway.

### 3. AppImage

AppImages can embed the signature inside the image itself:

```bash
./appimagetool --sign --sign-key <KEYID> release/MyApp.AppImage
```

Verification is then done by the AppImage runtime itself.

---

## Putting it all together in CI

Tynd's own `.github/workflows/build-host.yml` does not sign — signing is app-specific (your cert, your Apple ID). A typical downstream app workflow looks like:

```yaml
jobs:
  build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: oven-sh/setup-bun@v2
      - uses: dtolnay/rust-toolchain@stable

      - run: bun install
      - run: bun run --cwd . build            # your frontend build
      - run: bunx tynd build --bundle         # raw binary + installers

      # sign/notarize steps from the OS-specific sections above

      - uses: actions/upload-artifact@v4
        with:
          name: signed-${{ matrix.os }}
          path: release/
```

---

## What Tynd does NOT do (yet)

- No built-in `tynd sign` command — run `signtool` / `codesign` / `gpg` manually or via CI
- No cert / keychain management — store in CI secrets
- No auto-updater signature verification — planned alongside the auto-updater itself
- No WebView2 Evergreen bootstrapper bundling — users on Windows 10 without WebView2 will be prompted by the system

Contributions to integrate signing into `tynd build --sign` are welcome — see [CONTRIBUTING.md](./CONTRIBUTING.md).
