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
                _ => {},
            }
            i += 1;
        }

        if bundle_path.is_empty() {
            tynd_host::tynd_log!("Error: --bundle <path> is required");
            tynd_host::tynd_log!("Usage: tynd-lite --bundle /path/to/bundle.js [--frontend-dir /path/to/dist] [--debug]");
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
