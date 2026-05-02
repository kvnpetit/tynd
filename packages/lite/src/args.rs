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

impl Args {
    pub(crate) fn parse() -> Self {
        let raw: Vec<String> = std::env::args().collect();
        let mut bundle_path = String::new();
        let mut frontend_dir = None;
        let mut dev_url = None;
        let mut debug = false;

        let mut i = 1;
        while i < raw.len() {
            match raw[i].as_str() {
                "--bundle" | "-b" => {
                    i += 1;
                    if i < raw.len() {
                        bundle_path.clone_from(&raw[i]);
                    }
                },
                "--frontend-dir" | "-f" => {
                    i += 1;
                    if i < raw.len() {
                        frontend_dir = Some(raw[i].clone());
                    }
                },
                "--dev-url" => {
                    i += 1;
                    if i < raw.len() {
                        dev_url = Some(raw[i].clone());
                    }
                },
                "--debug" | "-d" => debug = true,
                "--version" | "-V" => {
                    // Wave 4.4: smoke-test E2E entry point — `tynd-lite --version`
                    // must succeed without `--bundle` so CI can sanity-check every
                    // release binary across the 6 OS×arch matrix.
                    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                },
                "--help" | "-h" => {
                    println!(
                        "Usage: tynd-lite --bundle <path> [--frontend-dir <dir>] [--dev-url \
                         <url>] [--debug]\n\nFlags:\n  -b, --bundle <path>         Path to \
                         pre-built JS bundle\n  -f, --frontend-dir <dir>    Static assets \
                         directory\n      --dev-url <url>         Dev server URL (overrides \
                         frontend-dir)\n  -d, --debug                 Enable WebView devtools + \
                         dev scheme\n  -V, --version               Print version and exit\n  -h, \
                         --help                  Print this help"
                    );
                    std::process::exit(0);
                },
                _ => {},
            }
            i += 1;
        }

        if bundle_path.is_empty() {
            tynd_host::tynd_log!("Error: --bundle <path> is required");
            tynd_host::tynd_log!(
                "Usage: tynd-lite --bundle /path/to/bundle.js [--frontend-dir /path/to/dist] \
                 [--debug]"
            );
            std::process::exit(1);
        }

        Self {
            bundle_path,
            frontend_dir,
            dev_url,
            debug,
        }
    }
}
