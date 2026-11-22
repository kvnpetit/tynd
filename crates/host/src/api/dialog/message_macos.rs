//! macOS message box: `NSAlert`, run on the main thread. With `parent` and a
//! key window it is shown as a sheet of that window, otherwise app-modal.

use std::sync::mpsc;

use block2::RcBlock;
use dispatch2::run_on_main;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSControlStateValueOff,
    NSControlStateValueOn, NSModalResponse,
};
use objc2_foundation::NSString;

use super::message::{Kind, MessageOutcome, MessageSpec};

/// Build the alert. Buttons are added in order, so button `i` answers
/// `NSAlertFirstButtonReturn + i`.
fn build(spec: &MessageSpec, mtm: MainThreadMarker) -> Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&spec.message));
    if let Some(detail) = &spec.detail {
        alert.setInformativeText(&NSString::from_str(detail));
    }
    alert.setAlertStyle(match spec.kind {
        Kind::Warning => NSAlertStyle::Warning,
        Kind::Error => NSAlertStyle::Critical,
        Kind::None | Kind::Info | Kind::Question => NSAlertStyle::Informational,
    });
    let labels = spec.labels();
    for (index, label) in labels.iter().enumerate() {
        let button = alert.addButtonWithTitle(&NSString::from_str(label));
        // AppKit makes the first button the Return key and a "Cancel"
        // button the Escape key; re-assign both to the requested buttons.
        let key = if index == spec.default_id {
            "\r"
        } else if index == spec.cancel_id {
            "\u{1b}"
        } else {
            ""
        };
        button.setKeyEquivalent(&NSString::from_str(key));
    }
    if let Some(label) = &spec.checkbox_label {
        alert.setShowsSuppressionButton(true);
        if let Some(checkbox) = alert.suppressionButton() {
            checkbox.setTitle(&NSString::from_str(label));
            checkbox.setState(if spec.checkbox_checked {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }
    alert
}

fn outcome(spec: &MessageSpec, alert: &NSAlert, response: NSModalResponse) -> MessageOutcome {
    let index = usize::try_from(response - NSAlertFirstButtonReturn)
        .ok()
        .filter(|index| *index < spec.buttons.len())
        .unwrap_or(spec.cancel_id);
    let checked = spec.checkbox_label.is_some()
        && alert
            .suppressionButton()
            .is_some_and(|checkbox| checkbox.state() == NSControlStateValueOn);
    MessageOutcome {
        response: index,
        checkbox_checked: checked,
    }
}

pub(super) fn show(spec: &MessageSpec) -> Result<MessageOutcome, String> {
    let (tx, rx) = mpsc::sync_channel::<MessageOutcome>(1);
    let owned = spec.clone();
    run_on_main(move |mtm| {
        let alert = build(&owned, mtm);
        let sheet_window = owned
            .parent
            .then(|| NSApplication::sharedApplication(mtm).keyWindow())
            .flatten();
        match sheet_window {
            Some(window) => {
                let retained = alert.clone();
                let handler = RcBlock::new(move |response: NSModalResponse| {
                    let _ = tx.send(outcome(&owned, &retained, response));
                });
                alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
            },
            None => {
                let response = alert.runModal();
                let _ = tx.send(outcome(&owned, &alert, response));
            },
        }
    });
    rx.recv()
        .map_err(|_| "the alert was closed without a response".to_owned())
}
