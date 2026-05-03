/// CLI arguments for the tynd-full host binary.
/// Window config comes from the backend's first stdout message (tynd:config),
/// so we only need the backend entry path here.
pub(crate) struct Args {
    /// Absolute path to the backend TypeScript entry file
    pub backend_entry: String,
    /// Enable WebView devtools
    pub debug: bool,
}

const HELP: &str = "\
Usage: tynd-full --backend-entry <path> [--debug]

Flags:
  -e, --backend-entry <path>  Absolute path to backend TS entry
  -d, --debug                 Enable WebView devtools + dev scheme
  -V, --version               Print version and exit
  -h, --help                  Print this help";

impl Args {
    pub(crate) fn parse() -> Self {
        // pico-args replaces a hand-rolled `while i < raw.len()` loop —
        // same flag set (4 flags, 0 positional), 1 KB binary cost,
        // ergonomic `value_from_str` / `contains` API.
        let mut pa = pico_args::Arguments::from_env();

        // Wave 4.4: smoke-test E2E entry point — `tynd-full --version`
        // must succeed without `--backend-entry` so CI can sanity-check
        // every release binary across the 6 OS×arch matrix.
        if pa.contains(["-V", "--version"]) {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        if pa.contains(["-h", "--help"]) {
            println!("{HELP}");
            std::process::exit(0);
        }

        let debug = pa.contains(["-d", "--debug"]);
        let backend_entry: Option<String> = pa
            .opt_value_from_str(["-e", "--backend-entry"])
            .unwrap_or(None);

        let Some(backend_entry) = backend_entry.filter(|s| !s.is_empty()) else {
            tynd_host::tynd_log!("Error: --backend-entry <path> is required");
            tynd_host::tynd_log!(
                "Usage: tynd-full --backend-entry /path/to/backend/main.ts [--debug]"
            );
            std::process::exit(1);
        };

        Self {
            backend_entry,
            debug,
        }
    }
}
