//! `msix` target: lays out the packed executable, its resources and the MSIX
//! visual assets, writes `AppxManifest.xml`, then packs the layout with
//! `makeappx.exe` (Windows SDK). The CLI signs the `.msix` with `signtool`
//! afterwards when Windows signing is configured; the manifest `Publisher`
//! must equal the certificate subject.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, ensure, Context, Result};
use tracing::{info, warn};
use tynd_bundle_contract::{safe_file_stem, BundleConfig, ExecutionLevel, MsixConfig};

use crate::icon_set;

/// Windows 10 1809: first release with MSIX.
const DEFAULT_MIN_VERSION: &str = "10.0.17763.0";
/// Windows 11 23H2 SDK.
const MAX_VERSION_TESTED: &str = "10.0.22621.0";

/// Package version: `bundle.windows.msix.version` when set, else the app
/// version cut to its numeric core. A prerelease or build suffix is dropped
/// (`1.2.3-beta.1` and `1.2.3` both give `1.2.3.0`), so it is reported.
pub(crate) fn package_version(cfg: &BundleConfig, msix: &MsixConfig) -> Result<String> {
    if let Some(version) = &msix.version {
        let parts: Vec<&str> = version.split('.').collect();
        ensure!(
            parts.len() == 4 && parts.iter().all(|part| part.parse::<u16>().is_ok()),
            "bundle.windows.msix.version {version:?} must have four numeric parts of at most 65535"
        );
        return Ok(version.clone());
    }
    let version = msix_version(&cfg.version)?;
    if cfg.version.contains(['-', '+']) {
        warn!(
            "MSIX version {version} drops the suffix of the app version {:?}: MSIX versions are four \
             numbers, so a prerelease and its release get the same version. Set \
             `bundle.windows.msix.version` to tell them apart (the Microsoft Store requires the \
             fourth part to be 0)",
            cfg.version
        );
    }
    Ok(version)
}

/// `Identity Name`: 3 to 50 characters of `[A-Za-z0-9.-]`.
pub(crate) fn identity_name(identifier: &str) -> Result<&str> {
    ensure!(
        (3..=50).contains(&identifier.len())
            && identifier
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-')),
        "the msix target uses `bundle.identifier` as the package Identity Name, which must be 3 to \
         50 characters of letters, digits, `.` and `-`; {identifier:?} is not"
    );
    Ok(identifier)
}

/// `1.2.3-beta.1` -> `1.2.3.0`: MSIX wants four numeric parts of at most 65535.
pub(crate) fn msix_version(version: &str) -> Result<String> {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let mut parts = core
        .split('.')
        .map(|part| {
            part.parse::<u16>()
                .with_context(|| format!("version {version:?} has a non-numeric or >65535 part"))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        (1..=4).contains(&parts.len()),
        "version {version:?} must have one to four numeric parts for MSIX"
    );
    parts.resize(4, 0);
    // The fourth part is reserved by the Microsoft Store; keep it 0 when the
    // app version did not set it.
    Ok(parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join("."))
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `ProcessorArchitecture` from the PE machine field.
pub(crate) fn processor_architecture(binary: &[u8]) -> Result<&'static str> {
    let pe = goblin::pe::PE::parse(binary).context("the app binary is not a PE executable")?;
    Ok(match pe.header.coff_header.machine {
        goblin::pe::header::COFF_MACHINE_X86_64 => "x64",
        goblin::pe::header::COFF_MACHINE_ARM64 => "arm64",
        goblin::pe::header::COFF_MACHINE_X86 => "x86",
        other => bail!("unsupported PE machine 0x{other:04x} for MSIX"),
    })
}

/// Lowercase `[a-z0-9._-]` name for `uap:FileTypeAssociation`.
fn association_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|character| {
            let character = character.to_ascii_lowercase();
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .take(100)
        .collect();
    if name.is_empty() {
        "files".to_owned()
    } else {
        name
    }
}

pub(crate) fn manifest(
    cfg: &BundleConfig,
    msix: &MsixConfig,
    version: &str,
    executable: &str,
    architecture: &str,
) -> Result<String> {
    let identity = identity_name(&cfg.identifier)?;
    // An elevating `requestedExecutionLevel` needs the restricted capability.
    let elevation = match cfg.windows.requested_execution_level {
        Some(ExecutionLevel::HighestAvailable | ExecutionLevel::RequireAdministrator) => {
            "\n    <rescap:Capability Name=\"allowElevation\"/>"
        },
        Some(ExecutionLevel::AsInvoker) | None => "",
    };
    let publisher_display_name = msix
        .publisher_display_name
        .clone()
        .or_else(|| {
            cfg.authors
                .as_ref()
                .and_then(|authors| authors.first().cloned())
        })
        .unwrap_or_else(|| cfg.name.clone());
    let description = cfg.description.as_deref().unwrap_or(&cfg.name);
    let min_version = msix.min_version.as_deref().unwrap_or(DEFAULT_MIN_VERSION);

    let mut extensions = String::new();
    for scheme in &cfg.macos.url_schemes {
        let _ = write!(
            extensions,
            "        <uap:Extension Category=\"windows.protocol\">\n          <uap:Protocol Name=\"{}\"/>\n        </uap:Extension>\n",
            xml(&scheme.to_ascii_lowercase())
        );
    }
    for association in &cfg.file_associations {
        let Some(first) = association.ext.first() else {
            continue;
        };
        let name = association_name(association.name.as_deref().unwrap_or(first));
        let _ = write!(
            extensions,
            "        <uap:Extension Category=\"windows.fileTypeAssociation\">\n          <uap:FileTypeAssociation Name=\"{}\">\n",
            xml(&name)
        );
        if let Some(description) = &association.description {
            let _ = writeln!(
                extensions,
                "            <uap:DisplayName>{}</uap:DisplayName>",
                xml(description)
            );
        }
        extensions.push_str("            <uap:SupportedFileTypes>\n");
        for ext in &association.ext {
            let _ = writeln!(
                extensions,
                "              <uap:FileType>.{}</uap:FileType>",
                xml(&ext.trim_start_matches('.').to_ascii_lowercase())
            );
        }
        extensions.push_str(
            "            </uap:SupportedFileTypes>\n          </uap:FileTypeAssociation>\n        </uap:Extension>\n",
        );
    }
    let extensions = if extensions.is_empty() {
        String::new()
    } else {
        format!("      <Extensions>\n{extensions}      </Extensions>\n")
    };

    Ok(format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap rescap">
  <Identity Name="{identity}" Publisher="{publisher}" Version="{version}" ProcessorArchitecture="{architecture}"/>
  <Properties>
    <DisplayName>{name}</DisplayName>
    <PublisherDisplayName>{publisher_display_name}</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="{min_version}" MaxVersionTested="{max_tested}"/>
  </Dependencies>
  <Resources>
    <Resource Language="en-us"/>
  </Resources>
  <Applications>
    <Application Id="App" Executable="{executable}" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="{name}" Description="{description}" BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile Wide310x150Logo="Assets\Wide310x150Logo.png" Square71x71Logo="Assets\Square71x71Logo.png" Square310x310Logo="Assets\Square310x310Logo.png"/>
      </uap:VisualElements>
{extensions}    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust"/>{elevation}
  </Capabilities>
</Package>
"#,
        identity = xml(identity),
        publisher = xml(&msix.publisher),
        name = xml(&cfg.name),
        publisher_display_name = xml(&publisher_display_name),
        description = xml(description),
        executable = xml(executable),
        max_tested = MAX_VERSION_TESTED,
    ))
}

/// `<name>_<version>_<arch>.msix`, the app name made file-name safe as for
/// the packed binary.
fn package_file_name(cfg: &BundleConfig, version: &str, architecture: &str) -> String {
    format!(
        "{}_{version}_{architecture}.msix",
        safe_file_stem(&cfg.name)
    )
}

/// `makeappx.exe` on `PATH`, else the newest one under the Windows 10/11 SDK.
fn find_makeappx() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join("makeappx.exe"))
            .find(|candidate| candidate.is_file())
    }) {
        return Some(path);
    }
    let kits = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)?
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let host = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let mut versions: Vec<PathBuf> = fs::read_dir(&kits)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(host).join("makeappx.exe"))
        .filter(|candidate| candidate.is_file())
        .collect();
    versions.sort();
    versions.pop()
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination)
            .with_context(|| format!("copy {} into the MSIX layout", source.display()))?;
    }
    Ok(())
}

/// Build the `.msix`; returns its path.
pub(crate) fn build(cfg: &BundleConfig, icons: &[PathBuf]) -> Result<PathBuf> {
    ensure!(
        cfg!(windows),
        "the msix target builds on Windows only (it needs makeappx.exe from the Windows SDK)"
    );
    let msix = cfg.windows.msix.as_ref().context(
        "the msix target needs `bundle.windows.msix.publisher` in tynd.config.ts: the subject of \
         your signing certificate, for example \"CN=Example Corp\"",
    )?;
    identity_name(&cfg.identifier)?;
    let version = package_version(cfg, msix)?;
    let makeappx = find_makeappx().context(
        "makeappx.exe not found: install the Windows 10/11 SDK (App packaging tools) from \
         https://developer.microsoft.com/windows/downloads/windows-sdk/ or add makeappx.exe to PATH",
    )?;
    let source = icons
        .iter()
        .map(|icon| {
            if icon.is_absolute() {
                icon.clone()
            } else {
                cfg.project_root.join(icon)
            }
        })
        .find(|icon| {
            icon.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("png")
                })
        })
        .context("the msix target needs a PNG or SVG `icon` for its tile assets")?;

    let layout = cfg.out_dir.join("msix-layout");
    if layout.exists() {
        fs::remove_dir_all(&layout).with_context(|| format!("clean {}", layout.display()))?;
    }
    let assets = layout.join("Assets");
    let rendered = cfg.out_dir.join("msix-icons");
    icon_set::generate(&source, &rendered)?;
    fs::create_dir_all(&assets)?;
    for (name, _, _) in icon_set::MSIX_ASSETS {
        fs::copy(rendered.join("msix").join(name), assets.join(name))?;
    }
    let _ = fs::remove_dir_all(&rendered);

    let executable = cfg
        .binary_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("the binary path has no UTF-8 file name")?;
    let binary = fs::read(&cfg.binary_path)
        .with_context(|| format!("read {}", cfg.binary_path.display()))?;
    let architecture = processor_architecture(&binary)?;
    fs::write(layout.join(executable), &binary)?;
    for resource in &cfg.resources {
        let name = resource
            .file_name()
            .with_context(|| format!("resource {} has no file name", resource.display()))?;
        copy_tree(resource, &layout.join(name))?;
    }
    fs::write(
        layout.join("AppxManifest.xml"),
        manifest(cfg, msix, &version, executable, architecture)?,
    )?;

    let output = cfg
        .out_dir
        .join(package_file_name(cfg, &version, architecture));
    info!("packing {}", output.display());
    let status = Command::new(&makeappx)
        .arg("pack")
        .arg("/o")
        .arg("/d")
        .arg(&layout)
        .arg("/p")
        .arg(&output)
        .status()
        .with_context(|| format!("start {}", makeappx.display()))?;
    ensure!(status.success(), "makeappx pack failed with {status}");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BundleConfig {
        serde_json::from_value(serde_json::json!({
            "name": "My <App>",
            "version": "1.2.3-beta.1",
            "identifier": "com.example.myapp",
            "authors": ["Example Corp"],
            "targets": ["msix"],
            "project_root": "C:/project",
            "out_dir": "C:/project/release",
            "binary_path": "C:/project/release/my-app.exe",
            "resources": [],
            "icons": [],
            "macos": { "url_schemes": ["MyApp"] },
            "file_associations": [{ "ext": ["MYD", ".myx"], "name": "My Doc", "description": "Doc" }],
            "windows": { "msix": { "publisher": "CN=Example Corp, O=Example" } },
        }))
        .expect("bundle config")
    }

    #[test]
    fn versions_become_four_numeric_parts() {
        assert_eq!(msix_version("1.2.3-beta.1").unwrap(), "1.2.3.0");
        assert_eq!(msix_version("2").unwrap(), "2.0.0.0");
        assert_eq!(msix_version("1.2.3.4+build").unwrap(), "1.2.3.4");
        assert!(msix_version("1.2.3.4.5").is_err());
        assert!(msix_version("1.70000.0").is_err());
        assert!(msix_version("one").is_err());
    }

    #[test]
    fn manifest_declares_identity_protocols_and_file_types() {
        let cfg = config();
        let msix = cfg.windows.msix.clone().unwrap();
        let text = manifest(&cfg, &msix, "1.2.3.0", "my-app.exe", "x64").unwrap();
        assert!(text.contains(
            r#"<Identity Name="com.example.myapp" Publisher="CN=Example Corp, O=Example" Version="1.2.3.0" ProcessorArchitecture="x64"/>"#
        ));
        assert!(text.contains("<DisplayName>My &lt;App&gt;</DisplayName>"));
        assert!(text.contains("<PublisherDisplayName>Example Corp</PublisherDisplayName>"));
        assert!(text.contains(r#"<uap:Protocol Name="myapp"/>"#));
        assert!(text.contains(r#"<uap:FileTypeAssociation Name="my-doc">"#));
        assert!(text.contains("<uap:FileType>.myd</uap:FileType>"));
        assert!(text.contains("<uap:FileType>.myx</uap:FileType>"));
        assert!(text.contains(r#"MinVersion="10.0.17763.0""#));
        assert!(text.contains(r#"<rescap:Capability Name="runFullTrust"/>"#));
        for (asset, _, _) in icon_set::MSIX_ASSETS {
            assert!(text.contains(asset), "{asset} is referenced");
        }
    }

    #[test]
    fn package_version_prefers_the_override() {
        let mut cfg = config();
        let mut msix = cfg.windows.msix.clone().unwrap();
        assert_eq!(package_version(&cfg, &msix).unwrap(), "1.2.3.0");
        msix.version = Some("1.2.2.0".into());
        assert_eq!(package_version(&cfg, &msix).unwrap(), "1.2.2.0");
        msix.version = Some("1.2.3".into());
        assert!(package_version(&cfg, &msix).is_err());
        msix.version = Some("1.2.3.70000".into());
        assert!(package_version(&cfg, &msix).is_err());
        cfg.version = "1.2.3".into();
        msix.version = None;
        assert_eq!(package_version(&cfg, &msix).unwrap(), "1.2.3.0");
    }

    #[test]
    fn identity_names_follow_the_msix_rules() {
        assert!(identity_name("com.example.myapp").is_ok());
        assert!(identity_name("a.b").is_ok());
        assert!(identity_name("ab").is_err());
        assert!(identity_name(&format!("com.example.{}", "x".repeat(40))).is_err());
        assert!(identity_name("com.example.my_app").is_err());
        let mut cfg = config();
        cfg.identifier = "com.x".repeat(11);
        let msix = cfg.windows.msix.clone().unwrap();
        let error = manifest(&cfg, &msix, "1.0.0.0", "a.exe", "x64").unwrap_err();
        assert!(error.to_string().contains("Identity Name"), "{error}");
    }

    #[test]
    fn elevated_execution_levels_allow_elevation() {
        let mut cfg = config();
        let msix = cfg.windows.msix.clone().unwrap();
        let plain = manifest(&cfg, &msix, "1.0.0.0", "a.exe", "x64").unwrap();
        assert!(!plain.contains("allowElevation"));
        cfg.windows.requested_execution_level = Some(ExecutionLevel::AsInvoker);
        let invoker = manifest(&cfg, &msix, "1.0.0.0", "a.exe", "x64").unwrap();
        assert!(!invoker.contains("allowElevation"));
        cfg.windows.requested_execution_level = Some(ExecutionLevel::RequireAdministrator);
        let elevated = manifest(&cfg, &msix, "1.0.0.0", "a.exe", "x64").unwrap();
        assert!(elevated.contains(
            "<rescap:Capability Name=\"runFullTrust\"/>\n    <rescap:Capability Name=\"allowElevation\"/>\n  </Capabilities>"
        ));
        assert!(elevated.contains("xmlns:rescap="));
        assert!(elevated.contains("IgnorableNamespaces=\"uap rescap\""));
    }

    #[test]
    fn package_file_name_is_file_name_safe() {
        let mut cfg = config();
        cfg.name = "My <App>: v2/x".into();
        assert_eq!(
            package_file_name(&cfg, "1.2.3.0", "x64"),
            "My--App---v2-x_1.2.3.0_x64.msix"
        );
    }

    #[test]
    fn machine_types_map_to_msix_architectures() {
        let host = fs::read(std::env::current_exe().unwrap()).unwrap();
        if cfg!(windows) {
            let expected = if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            };
            assert_eq!(processor_architecture(&host).unwrap(), expected);
        } else {
            assert!(processor_architecture(&host).is_err());
        }
    }
}
