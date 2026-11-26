//! `tynd-bundler` — cross-platform bundler + code-signer invoked by the `tynd`
//! tooling (`@tynd/cli`). Errors land on stderr as one `error:` line.

#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

use std::{
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;
use tracing::Level;

mod bundle;
mod codesign;
mod dmg;
mod icon_set;
mod linux_packages;
mod msix;
mod pack;
mod patch;
mod pe;
mod util;

const MAX_CONFIG_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Error, Diagnostic)]
#[error("invalid bundler config")]
#[diagnostic(
    code(tynd::bundler::config),
    url("https://kvnpetit.github.io/tynd/docs/v1.0/cli/config"),
    help("check the JSON config piped in by `@tynd/cli`; the offending field is shown above")
)]
struct ConfigParseError {
    #[source_code]
    src: NamedSource<String>,
    #[label("here")]
    span: SourceSpan,
    msg: String,
}

impl ConfigParseError {
    fn from_serde(name: &str, source: String, err: &serde_json::Error) -> Self {
        let line = err.line().saturating_sub(1);
        let col = err.column().saturating_sub(1);
        // Translate (line, col) -> byte offset for SourceSpan.
        let mut offset = 0usize;
        for (i, l) in source.split_inclusive('\n').enumerate() {
            if i == line {
                offset += col.min(l.len().saturating_sub(1));
                break;
            }
            offset += l.len();
        }
        Self {
            src: NamedSource::new(name, source),
            span: (offset, 1).into(),
            msg: err.to_string(),
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "tynd-bundler",
    version,
    about = "Bundle + sign Tynd apps via tauri-bundler + apple-codesign"
)]
struct Cli {
    /// -v = debug, -vv = trace.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build installer artifacts (MSI/NSIS/DMG/.deb/.rpm/AppImage).
    Bundle {
        /// Config JSON path, or `-` for stdin.
        config: String,
    },
    /// Cross-host Mach-O / `.app` / `.pkg` / DMG sign + notarize via apple-codesign.
    Codesign {
        /// Config JSON path, or `-` for stdin.
        config: String,
        /// On notarization timeout (Apple notary kept us waiting past the
        /// 15-minute budget), warn and exit 0 instead of failing. Stapling
        /// must then be done out-of-band with `rcodesign staple`.
        ///
        /// Also honoured via env var `TYND_ALLOW_NOTARIZE_TIMEOUT=1` for
        /// callers that can't pass the flag through.
        #[arg(long, default_value_t = false)]
        allow_notarize_timeout: bool,
    },
    /// BSDIFF (+ optional zstd) patch between two shipped binaries (signed if you sign) — consumed
    /// by tynd-host's `bipatch::Reader`.
    Patch(PatchCli),
    /// Archive a signed `.app` for Tynd's authenticated updater.
    ArchiveApp {
        #[arg(long, value_name = "PATH")]
        app: PathBuf,
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
    },
    /// Build a DMG from an application that was already signed and stapled.
    DmgFromApp {
        #[arg(long, value_name = "PATH")]
        app: PathBuf,
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
        #[arg(long, value_name = "NAME")]
        volume_name: String,
    },
    /// Produce a TYNDPKG self-extracting `.exe`.
    Pack {
        /// Config JSON path, or `-` for stdin.
        config: String,
    },
    /// Render one square PNG/SVG into `.ico`, `.icns`, hicolor PNGs and MSIX assets.
    Icon {
        /// Square PNG (at least 512x512) or SVG.
        #[arg(long, value_name = "PATH")]
        source: PathBuf,
        /// Output directory; created when missing.
        #[arg(long, value_name = "DIR")]
        out_dir: PathBuf,
    },
}

#[derive(clap::Args, Debug)]
struct PatchCli {
    /// Previously released binary, exactly as shipped (signed if you sign).
    #[arg(long, value_name = "PATH")]
    old: PathBuf,
    /// Freshly built binary to upgrade to, exactly as it will ship.
    #[arg(long, value_name = "PATH")]
    new: PathBuf,
    /// Output patch file. Parent dirs are NOT auto-created.
    #[arg(long, value_name = "PATH")]
    output: PathBuf,
    /// Skip zstd compression (default: compress at level 19).
    #[arg(long, default_value_t = false)]
    no_compress: bool,
}

fn main() {
    if let Err(e) = run() {
        // One-line `error:` summary; miette report below for humans.
        eprintln!("error: {e:#}");
        if let Some(diag) = e.downcast_ref::<ConfigParseError>() {
            let report = miette::Report::new(diag.clone());
            eprintln!("\n{report:?}");
        } else if let Some(diag) = e.downcast_ref::<bundle::BundleError>() {
            let report = miette::Report::new(diag.clone());
            eprintln!("\n{report:?}");
        } else if let Some(diag) = e.downcast_ref::<codesign::CodesignError>() {
            let report = miette::Report::new(diag.clone());
            eprintln!("\n{report:?}");
        } else if let Some(diag) = e.downcast_ref::<patch::PatchError>() {
            let report = miette::Report::new(diag.clone());
            eprintln!("\n{report:?}");
        } else if let Some(diag) = e.downcast_ref::<pack::PackError>() {
            let report = miette::Report::new(diag.clone());
            eprintln!("\n{report:?}");
        }
        std::process::exit(1);
    }
}

impl Clone for ConfigParseError {
    fn clone(&self) -> Self {
        Self {
            src: NamedSource::new(self.src.name(), self.src.inner().clone()),
            span: self.span,
            msg: self.msg.clone(),
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    match cli.command {
        Command::Bundle { config } => {
            let cfg: tynd_bundle_contract::BundleConfig = read_config(&config)?;
            bundle::run(&cfg)
        },
        Command::Codesign {
            config,
            allow_notarize_timeout,
        } => {
            let cfg: tynd_bundle_contract::CodesignConfig = read_config(&config)?;
            // Flag wins over env, but env still acts as a fallback for callers
            // (CI scripts, `tynd-cli` shells out) that can't thread the flag.
            let mut opts = codesign::RunOptions::from_env();
            if allow_notarize_timeout {
                opts.allow_notarize_timeout = true;
            }
            codesign::run_with_options(&cfg, opts)
        },
        Command::Patch(args) => {
            // Same anyhow/miette boundary as `Sign`.
            let patch_args = patch::PatchArgs {
                old: args.old,
                new: args.new,
                output: args.output,
                compress: !args.no_compress,
            };
            patch::run(&patch_args).map_err(|report| match report.downcast::<patch::PatchError>() {
                Ok(typed) => anyhow::Error::new(typed),
                Err(report) => anyhow::anyhow!("{report}"),
            })
        },
        Command::ArchiveApp { app, output } => {
            let app = util::to_utf8_path(app)?;
            let output = util::to_utf8_path(output)?;
            util::zip_app_bundle(&app, &output)?;
            println!("{output}");
            Ok(())
        },
        Command::DmgFromApp {
            app,
            output,
            volume_name,
        } => {
            dmg::create_from_signed_app(&app, &output, &volume_name)?;
            println!("{}", output.display());
            Ok(())
        },
        Command::Pack { config } => {
            let cfg: tynd_bundle_contract::PackConfig = read_config(&config)?;
            pack::run(&cfg)
        },
        Command::Icon { source, out_dir } => {
            for path in icon_set::generate(&source, &out_dir)? {
                println!("{}", path.display());
            }
            Ok(())
        },
    }
}

fn read_config<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    let (name, raw) = if path == "-" {
        (
            "<stdin>".to_string(),
            read_utf8_limited(std::io::stdin(), MAX_CONFIG_BYTES)
                .context("read config from stdin")?,
        )
    } else {
        let p = PathBuf::from(path);
        let raw = read_file_utf8_limited(&p, MAX_CONFIG_BYTES)
            .with_context(|| format!("read config {}", p.display()))?;
        (p.display().to_string(), raw)
    };
    serde_json::from_str(&raw)
        .map_err(|err| anyhow::Error::new(ConfigParseError::from_serde(&name, raw.clone(), &err)))
}

fn read_file_utf8_limited(path: &Path, max_bytes: u64) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > max_bytes {
        anyhow::bail!(
            "input is {} bytes; maximum allowed size is {max_bytes} bytes",
            metadata.len()
        );
    }
    read_utf8_limited(file, max_bytes)
}

fn read_utf8_limited(reader: impl std::io::Read, max_bytes: u64) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .context("read bounded UTF-8 input")?;
    if bytes.len() as u64 > max_bytes {
        anyhow::bail!("input exceeds maximum allowed size of {max_bytes} bytes");
    }
    String::from_utf8(bytes).context("input is not valid UTF-8")
}
