/// CLI arguments for the vorn-full host binary.
/// Window config comes from the backend's first stdout message (vorn:config),
/// so we only need the backend entry path here.
pub(crate) struct Args {
    /// Absolute path to the backend TypeScript entry file
    pub backend_entry: String,
    /// Enable WebView devtools
    pub debug: bool,
}

impl Args {
    pub(crate) fn parse() -> Self {
        let raw: Vec<String> = std::env::args().collect();
        let mut backend_entry = String::new();
        let mut debug = false;

        let mut i = 1;
        while i < raw.len() {
            match raw[i].as_str() {
                "--backend-entry" | "-e" => {
                    i += 1;
                    if i < raw.len() {
                        backend_entry.clone_from(&raw[i]);
                    }
                },
                "--debug" | "-d" => debug = true,
                _ => {},
            }
            i += 1;
        }

        if backend_entry.is_empty() {
            vorn_host::vorn_log!("Error: --backend-entry <path> is required");
            vorn_host::vorn_log!(
                "Usage: vorn-full --backend-entry /path/to/backend/main.ts [--debug]"
            );
            std::process::exit(1);
        }

        Self {
            backend_entry,
            debug,
        }
    }
}
