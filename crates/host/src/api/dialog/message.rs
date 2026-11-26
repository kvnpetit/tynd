//! Platform-neutral message-box request: argument validation and the
//! Electron-style button / cancel resolution shared by every backend.

/// Longest accepted title / checkbox label, in bytes.
const MAX_SHORT_TEXT: usize = 1024;
/// Longest accepted message / detail, in bytes.
const MAX_LONG_TEXT: usize = 64 * 1024;
/// Longest accepted button label, in bytes.
const MAX_BUTTON_LABEL: usize = 256;
/// Most custom buttons one dialog may show.
pub(super) const MAX_BUTTONS: usize = 16;

/// Icon / severity of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    None,
    Info,
    Warning,
    Error,
    Question,
}

impl Kind {
    fn parse(kind: Option<&str>) -> Result<Self, String> {
        match kind.unwrap_or("info") {
            "none" => Ok(Self::None),
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            "question" => Ok(Self::Question),
            other => Err(format!(
                "unknown kind '{other}' — expected none, info, warning, error or question"
            )),
        }
    }
}

/// Buttons of the dialog. The presets use the OS-localized labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Buttons {
    /// A single localized "OK" (response `0`).
    Ok,
    /// Localized "OK" (response `0`) and "Cancel" (response `1`).
    OkCancel,
    /// Caller-supplied labels, response = index.
    Custom(Vec<String>),
}

impl Buttons {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Ok => 1,
            Self::OkCancel => 2,
            Self::Custom(labels) => labels.len(),
        }
    }
}

/// Validated message-box request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MessageSpec {
    pub(super) title: String,
    pub(super) message: String,
    pub(super) detail: Option<String>,
    pub(super) kind: Kind,
    pub(super) buttons: Buttons,
    /// Index of the button focused when the dialog opens.
    pub(super) default_id: usize,
    /// Index reported when the user dismisses the dialog (Esc, close box).
    pub(super) cancel_id: usize,
    pub(super) checkbox_label: Option<String>,
    pub(super) checkbox_checked: bool,
    /// Make the dialog modal to the app's active window.
    pub(super) parent: bool,
}

/// What the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MessageOutcome {
    /// Index of the clicked button (or `cancel_id` on dismissal).
    pub(super) response: usize,
    /// Final state of the checkbox (`false` without one).
    pub(super) checkbox_checked: bool,
}

/// Raw renderer arguments of `dialog.message`.
#[derive(Debug, Default)]
pub(super) struct MessageArgs {
    pub(super) message: String,
    pub(super) title: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) buttons: Option<Vec<String>>,
    pub(super) default_id: Option<u32>,
    pub(super) cancel_id: Option<u32>,
    pub(super) checkbox_label: Option<String>,
    pub(super) checkbox_checked: Option<bool>,
    pub(super) parent: Option<bool>,
}

fn check_text(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{field} is longer than {max} bytes"));
    }
    if value.contains('\0') {
        return Err(format!("{field} must not contain NUL characters"));
    }
    Ok(())
}

fn check_index(field: &str, index: Option<u32>, count: usize) -> Result<Option<usize>, String> {
    let Some(index) = index else {
        return Ok(None);
    };
    let index = index as usize;
    if index >= count {
        return Err(format!(
            "{field} {index} is out of range for {count} button(s)"
        ));
    }
    Ok(Some(index))
}

/// Electron's default cancel button: the first one labelled "Cancel" or
/// "No" (case-insensitive), else `0`.
fn default_cancel_id(buttons: &Buttons) -> usize {
    match buttons {
        Buttons::Ok => 0,
        Buttons::OkCancel => 1,
        Buttons::Custom(labels) => labels
            .iter()
            .position(|label| {
                let label = label.trim();
                label.eq_ignore_ascii_case("cancel") || label.eq_ignore_ascii_case("no")
            })
            .unwrap_or(0),
    }
}

impl MessageSpec {
    /// Validate renderer arguments. `preset` is used when the caller passes
    /// no `buttons` (or an empty list); `default_title` when it passes no
    /// title.
    pub(super) fn parse(
        args: MessageArgs,
        preset: Buttons,
        default_title: &str,
    ) -> Result<Self, String> {
        check_text("message", &args.message, MAX_LONG_TEXT)?;
        let title = args.title.unwrap_or_else(|| default_title.to_owned());
        check_text("title", &title, MAX_SHORT_TEXT)?;
        if let Some(detail) = &args.detail {
            check_text("detail", detail, MAX_LONG_TEXT)?;
        }
        if let Some(label) = &args.checkbox_label {
            if label.trim().is_empty() {
                return Err("checkboxLabel must not be empty".into());
            }
            check_text("checkboxLabel", label, MAX_SHORT_TEXT)?;
        }
        let kind = Kind::parse(args.kind.as_deref())?;

        let buttons = match args.buttons {
            Some(labels) if !labels.is_empty() => {
                if labels.len() > MAX_BUTTONS {
                    return Err(format!(
                        "at most {MAX_BUTTONS} buttons are supported, got {}",
                        labels.len()
                    ));
                }
                for (index, label) in labels.iter().enumerate() {
                    if label.trim().is_empty() {
                        return Err(format!("buttons[{index}] must not be empty"));
                    }
                    check_text(&format!("buttons[{index}]"), label, MAX_BUTTON_LABEL)?;
                }
                Buttons::Custom(labels)
            },
            _ => preset,
        };
        let count = buttons.len();
        let default_id = check_index("defaultId", args.default_id, count)?.unwrap_or(0);
        let cancel_id = check_index("cancelId", args.cancel_id, count)?
            .unwrap_or_else(|| default_cancel_id(&buttons));

        Ok(Self {
            title,
            message: args.message,
            detail: args.detail.filter(|detail| !detail.is_empty()),
            kind,
            buttons,
            default_id,
            cancel_id,
            checkbox_checked: args.checkbox_checked.unwrap_or(false)
                && args.checkbox_label.is_some(),
            checkbox_label: args.checkbox_label,
            parent: args.parent.unwrap_or(true),
        })
    }

    /// Labels of every button, presets included (English for the presets:
    /// used only by backends without localized stock buttons).
    #[cfg_attr(
        not(target_os = "macos"),
        allow(
            dead_code,
            reason = "only the AppKit backend needs literal preset labels"
        )
    )]
    pub(super) fn labels(&self) -> Vec<&str> {
        match &self.buttons {
            Buttons::Ok => vec!["OK"],
            Buttons::OkCancel => vec!["OK", "Cancel"],
            Buttons::Custom(labels) => labels.iter().map(String::as_str).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(message: &str) -> MessageArgs {
        MessageArgs {
            message: message.into(),
            ..MessageArgs::default()
        }
    }

    #[test]
    fn defaults_to_an_info_ok_box_modal_to_the_active_window() {
        let spec = MessageSpec::parse(args("Hi"), Buttons::Ok, "Message").expect("valid");
        assert_eq!(spec.kind, Kind::Info);
        assert_eq!(spec.buttons, Buttons::Ok);
        assert_eq!(spec.title, "Message");
        assert_eq!((spec.default_id, spec.cancel_id), (0, 0));
        assert!(spec.parent);
        assert!(!spec.checkbox_checked);
    }

    #[test]
    fn custom_buttons_pick_cancel_or_no_as_the_cancel_button() {
        let mut a = args("Save?");
        a.buttons = Some(vec!["Save".into(), "Don't save".into(), "Cancel".into()]);
        let spec = MessageSpec::parse(a, Buttons::Ok, "Message").expect("valid");
        assert_eq!(spec.cancel_id, 2);

        let mut a = args("Quit?");
        a.buttons = Some(vec!["Yes".into(), "no".into()]);
        assert_eq!(
            MessageSpec::parse(a, Buttons::Ok, "Message")
                .expect("valid")
                .cancel_id,
            1
        );

        let mut a = args("Pick");
        a.buttons = Some(vec!["A".into(), "B".into()]);
        assert_eq!(
            MessageSpec::parse(a, Buttons::Ok, "Message")
                .expect("valid")
                .cancel_id,
            0
        );
    }

    #[test]
    fn empty_buttons_fall_back_to_the_preset() {
        let mut a = args("Sure?");
        a.buttons = Some(Vec::new());
        let spec = MessageSpec::parse(a, Buttons::OkCancel, "Confirm").expect("valid");
        assert_eq!(spec.buttons, Buttons::OkCancel);
        assert_eq!(spec.cancel_id, 1);
        assert_eq!(spec.labels(), vec!["OK", "Cancel"]);
    }

    #[test]
    fn explicit_ids_are_range_checked() {
        let mut a = args("x");
        a.buttons = Some(vec!["A".into(), "B".into()]);
        a.default_id = Some(1);
        a.cancel_id = Some(0);
        let spec = MessageSpec::parse(a, Buttons::Ok, "Message").expect("valid");
        assert_eq!((spec.default_id, spec.cancel_id), (1, 0));

        let mut a = args("x");
        a.buttons = Some(vec!["A".into()]);
        a.default_id = Some(1);
        let err = MessageSpec::parse(a, Buttons::Ok, "Message").expect_err("out of range");
        assert!(err.contains("defaultId 1"), "{err}");

        let mut a = args("x");
        a.cancel_id = Some(3);
        assert!(MessageSpec::parse(a, Buttons::OkCancel, "Confirm").is_err());
    }

    #[test]
    fn rejects_bad_text_and_kinds() {
        let mut a = args("x");
        a.kind = Some("fatal".into());
        assert!(MessageSpec::parse(a, Buttons::Ok, "M").is_err());

        assert!(MessageSpec::parse(args("a\0b"), Buttons::Ok, "M").is_err());
        assert!(
            MessageSpec::parse(args(&"x".repeat(MAX_LONG_TEXT + 1)), Buttons::Ok, "M").is_err()
        );

        let mut a = args("x");
        a.buttons = Some(vec!["OK".into(), "  ".into()]);
        assert!(MessageSpec::parse(a, Buttons::Ok, "M").is_err());

        let mut a = args("x");
        a.buttons = Some(vec!["b".into(); MAX_BUTTONS + 1]);
        assert!(MessageSpec::parse(a, Buttons::Ok, "M").is_err());

        let mut a = args("x");
        a.checkbox_label = Some(String::new());
        assert!(MessageSpec::parse(a, Buttons::Ok, "M").is_err());
    }

    #[test]
    fn checkbox_state_needs_a_label() {
        let mut a = args("x");
        a.checkbox_checked = Some(true);
        assert!(
            !MessageSpec::parse(a, Buttons::Ok, "M")
                .expect("valid")
                .checkbox_checked
        );

        let mut a = args("x");
        a.checkbox_label = Some("Don't ask again".into());
        a.checkbox_checked = Some(true);
        let spec = MessageSpec::parse(a, Buttons::Ok, "M").expect("valid");
        assert!(spec.checkbox_checked);
    }

    #[test]
    fn parent_can_be_disabled_and_empty_detail_is_dropped() {
        let mut a = args("x");
        a.parent = Some(false);
        a.detail = Some(String::new());
        let spec = MessageSpec::parse(a, Buttons::Ok, "M").expect("valid");
        assert!(!spec.parent);
        assert_eq!(spec.detail, None);
    }
}
