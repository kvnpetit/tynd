/// Parsed CLI arguments for the webview host.
pub struct Args {
    pub title:               String,
    pub width:               u32,
    pub height:              u32,
    pub resizable:           bool,
    pub debug:               bool,
    pub frameless:           bool,
    pub transparent:         bool,
    pub icon:                Option<String>,
    pub vibrancy:            Option<String>,
    pub static_dir:          Option<String>,
    pub show_minimize_button:  bool,
    pub show_maximize_button:  bool,
    pub show_close_button:     bool,
    pub hardware_acceleration: bool,
}

impl Args {
    pub fn parse() -> Self {
        let mut args = Self {
            title:               "Bunview App".into(),
            width:               900,
            height:              600,
            resizable:           true,
            debug:               false,
            frameless:           false,
            transparent:         false,
            icon:                None,
            vibrancy:            None,
            static_dir:          None,
            show_minimize_button:  true,
            show_maximize_button:  true,
            show_close_button:     true,
            hardware_acceleration: false,
        };

        for raw in std::env::args().skip(1) {
            if let Some(v) = raw.strip_prefix("--title=") {
                args.title = v.to_string();
            } else if let Some(v) = raw.strip_prefix("--width=") {
                args.width = v.parse().unwrap_or(900);
            } else if let Some(v) = raw.strip_prefix("--height=") {
                args.height = v.parse().unwrap_or(600);
            } else if raw == "--debug" {
                args.debug = true;
            } else if raw == "--frameless" {
                args.frameless = true;
            } else if raw == "--transparent" {
                args.transparent = true;
            } else if raw == "--resizable=false" {
                args.resizable = false;
            } else if let Some(v) = raw.strip_prefix("--icon=") {
                args.icon = Some(v.to_string());
            } else if let Some(v) = raw.strip_prefix("--vibrancy=") {
                args.vibrancy = Some(v.to_string());
            } else if let Some(v) = raw.strip_prefix("--static-dir=") {
                args.static_dir = Some(v.to_string());
            } else if raw == "--no-minimize-btn" {
                args.show_minimize_button = false;
            } else if raw == "--no-maximize-btn" {
                args.show_maximize_button = false;
            } else if raw == "--no-close-btn" {
                args.show_close_button = false;
            } else if raw == "--no-hw-accel" {
                args.hardware_acceleration = false;
            }
        }

        args
    }
}
