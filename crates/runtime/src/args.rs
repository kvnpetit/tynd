/// CLI arguments for the tynd-runtime host binary.
/// Window config comes from the backend's first stdout message (tynd:config),
/// so we only need the backend entry path here.
use std::ffi::OsString;

pub(crate) struct Args {
    /// Absolute path to the backend TypeScript entry file
    pub backend_entry: String,
    /// Enable WebView devtools
    pub debug: bool,
    /// Everything after a `--` separator, forwarded untouched to the app
    /// backend (`tynd.config` `binaryArgs`).
    pub passthrough: Vec<OsString>,
}

#[derive(Debug, PartialEq, Eq)]
enum ParseExit {
    Help,
    Version,
    MissingBackendEntry,
    InvalidArgument(String),
}

const HELP: &str = "\
Usage: tynd-runtime --backend-entry <path> [--debug] [-- <app args>...]

Flags:
  -e, --backend-entry <path>  Absolute path to backend TS entry
  -d, --debug                 Enable WebView devtools + dev scheme
  -V, --version               Print version and exit
  -h, --help                  Print this help
  --                          Pass every following argument to the app";

/// Arguments the app itself should see (`argv[1..]` of a packaged executable).
pub(crate) fn packaged_app_args() -> Vec<OsString> {
    std::env::args_os().skip(1).collect()
}

impl Args {
    /// Handle process-level informational flags before packaged assets are
    /// extracted. Only meaningful for the unpacked runtime: a packaged app owns
    /// its command line, so `-h` / `-V` must reach it instead of this runtime.
    pub(crate) fn exit_on_informational_flags() {
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        if has_flag(&args, "-V", "--version") {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        if has_flag(&args, "-h", "--help") {
            println!("{HELP}");
            std::process::exit(0);
        }
    }

    pub(crate) fn parse() -> Self {
        match Self::parse_from_iter(std::env::args_os().skip(1)) {
            Ok(args) => args,
            Err(ParseExit::Version) => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            },
            Err(ParseExit::Help) => {
                println!("{HELP}");
                std::process::exit(0);
            },
            Err(ParseExit::MissingBackendEntry) => {
                tynd_host::tynd_error!("Error: --backend-entry <path> is required");
                tynd_host::tynd_error!(
                    "Usage: tynd-runtime --backend-entry /path/to/backend/main.ts [--debug]"
                );
                std::process::exit(1);
            },
            Err(ParseExit::InvalidArgument(argument)) => {
                tynd_host::tynd_error!("Error: unsupported or duplicate argument '{argument}'");
                tynd_host::tynd_error!("{HELP}");
                std::process::exit(2);
            },
        }
    }

    fn parse_from_iter<I>(args: I) -> Result<Self, ParseExit>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut args: Vec<OsString> = args.into_iter().collect();
        // Everything after `--` belongs to the app, never to this parser.
        let app_args = match args.iter().position(|arg| arg == "--") {
            Some(separator) => {
                let rest = args.split_off(separator + 1);
                args.pop();
                rest
            },
            None => Vec::new(),
        };
        // `tynd-runtime --version` must succeed without `--backend-entry`
        // so CI can smoke-test every release binary across the OS x arch
        // matrix.
        if has_flag(&args, "-V", "--version") {
            return Err(ParseExit::Version);
        }
        if has_flag(&args, "-h", "--help") {
            return Err(ParseExit::Help);
        }

        let mut debug = false;
        let mut backend_entry: Option<String> = None;
        let mut index = 0usize;
        while index < args.len() {
            let argument = args[index].to_string_lossy();
            match argument.as_ref() {
                "-d" | "--debug" => debug = true,
                "-e" | "--backend-entry" => {
                    if backend_entry.is_some() {
                        return Err(ParseExit::InvalidArgument(argument.into_owned()));
                    }
                    let Some(value) = args.get(index + 1) else {
                        return Err(ParseExit::MissingBackendEntry);
                    };
                    let value = value.to_string_lossy();
                    if value.is_empty() || value.starts_with('-') {
                        return Err(ParseExit::MissingBackendEntry);
                    }
                    backend_entry = Some(value.into_owned());
                    index += 1;
                },
                value if value.starts_with("--backend-entry=") => {
                    if backend_entry.is_some() {
                        return Err(ParseExit::InvalidArgument("--backend-entry".into()));
                    }
                    let value = value.trim_start_matches("--backend-entry=");
                    if value.is_empty() {
                        return Err(ParseExit::MissingBackendEntry);
                    }
                    backend_entry = Some(value.to_string());
                },
                _ => return Err(ParseExit::InvalidArgument(argument.into_owned())),
            }
            index += 1;
        }

        let Some(backend_entry) = backend_entry else {
            return Err(ParseExit::MissingBackendEntry);
        };

        Ok(Self {
            backend_entry,
            debug,
            passthrough: app_args,
        })
    }
}

/// Whether `short` / `long` appears before a `--` separator.
fn has_flag(args: &[OsString], short: &str, long: &str) -> bool {
    args.iter().take_while(|arg| *arg != "--").any(|arg| {
        let arg = arg.to_string_lossy();
        (!short.is_empty() && arg == short) || arg == long
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, ParseExit> {
        Args::parse_from_iter(args.iter().map(OsString::from))
    }

    #[test]
    fn parses_dev_arguments() {
        let args = parse(&["--backend-entry", "main.ts", "--debug"]).unwrap();
        assert_eq!(args.backend_entry, "main.ts");
        assert!(args.debug);
        assert_eq!(args.passthrough, [] as [OsString; 0]);
    }

    #[test]
    fn rejects_unknown_arguments_before_the_separator() {
        assert_eq!(
            parse(&["-e", "main.ts", "--foo"]).err(),
            Some(ParseExit::InvalidArgument("--foo".into()))
        );
    }

    #[test]
    fn passes_arguments_after_the_separator_to_the_app() {
        let args = parse(&["-e", "main.ts", "--", "--foo", "-h", "-V", "--debug", "x"]).unwrap();
        assert_eq!(args.backend_entry, "main.ts");
        assert!(!args.debug);
        let forwarded: Vec<_> = args
            .passthrough
            .iter()
            .map(|a| a.to_string_lossy())
            .collect();
        assert_eq!(forwarded, ["--foo", "-h", "-V", "--debug", "x"]);
    }

    #[test]
    fn separator_without_app_arguments_is_fine() {
        let args = parse(&["-e", "main.ts", "--"]).unwrap();
        assert_eq!(args.passthrough, [] as [OsString; 0]);
    }

    #[test]
    fn informational_flags_only_count_before_the_separator() {
        assert_eq!(parse(&["-V"]).err(), Some(ParseExit::Version));
        assert_eq!(parse(&["--help"]).err(), Some(ParseExit::Help));
        assert!(parse(&["-e", "main.ts", "--", "--help"]).is_ok());
        assert_eq!(
            parse(&["--", "-e", "main.ts"]).err(),
            Some(ParseExit::MissingBackendEntry)
        );
    }
}
