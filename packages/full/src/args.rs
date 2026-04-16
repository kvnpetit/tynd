/// CLI arguments for the tynd-full host binary.
/// Window config comes from the backend's first stdout message (tynd:config),
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
            tynd_host::tynd_log!("Error: --backend-entry <path> is required");
            tynd_host::tynd_log!(
                "Usage: tynd-full --backend-entry /path/to/backend/main.ts [--debug]"
            );
            std::process::exit(1);
        }

        Self {
            backend_entry,
            debug,
        }
    }
}
