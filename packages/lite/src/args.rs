/// CLI arguments for the tynd-lite host binary.
/// Unlike tynd-full, window config comes from the JS bundle itself
/// (globalThis.__tynd_config__), but frontend path is a CLI arg because
/// import.meta.dir is not available in a browser-target IIFE bundle.
pub(crate) struct Args {
    /// Path to the pre-built JS bundle (IIFE, sets globalThis.__tynd_mod__)
    pub bundle_path: String,
    /// Optional: directory of static frontend files to serve via tynd://
    pub frontend_dir: Option<String>,
    /// Optional: dev server URL to load instead of static files
    pub dev_url: Option<String>,
    /// Enable WebView devtools
    pub debug: bool,
}

const HELP: &str = "\
Usage: tynd-lite --bundle <path> [--frontend-dir <dir>] [--dev-url <url>] [--debug]

Flags:
  -b, --bundle <path>         Path to pre-built JS bundle
  -f, --frontend-dir <dir>    Static assets directory
      --dev-url <url>         Dev server URL (overrides frontend-dir)
  -d, --debug                 Enable WebView devtools + dev scheme
  -V, --version               Print version and exit
  -h, --help                  Print this help";

impl Args {
    pub(crate) fn parse() -> Self {
        // pico-args replaces a hand-rolled `while i < raw.len()` loop —
        // same flag set, 1 KB binary cost, ergonomic API.
        let mut pa = pico_args::Arguments::from_env();

        // Wave 4.4: smoke-test E2E entry point — `tynd-lite --version`
        // must succeed without `--bundle` so CI can sanity-check every
        // release binary across the 6 OS×arch matrix.
        if pa.contains(["-V", "--version"]) {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        if pa.contains(["-h", "--help"]) {
            println!("{HELP}");
            std::process::exit(0);
        }

        let debug = pa.contains(["-d", "--debug"]);
        let bundle_path: Option<String> =
            pa.opt_value_from_str(["-b", "--bundle"]).unwrap_or(None);
        let frontend_dir: Option<String> = pa
            .opt_value_from_str(["-f", "--frontend-dir"])
            .unwrap_or(None);
        // `--dev-url` has no short form — `pico_args::Keys` accepts a
        // single `&str` for that case.
        let dev_url: Option<String> = pa.opt_value_from_str("--dev-url").unwrap_or(None);

        let Some(bundle_path) = bundle_path.filter(|s| !s.is_empty()) else {
            tynd_host::tynd_log!("Error: --bundle <path> is required");
            tynd_host::tynd_log!(
                "Usage: tynd-lite --bundle /path/to/bundle.js [--frontend-dir /path/to/dist] \
                 [--debug]"
            );
            std::process::exit(1);
        };

        Self {
            bundle_path,
            frontend_dir,
            dev_url,
            debug,
        }
    }
}
