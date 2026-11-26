//! Linux message box: `GtkMessageDialog`, run on the GTK main thread (tao's
//! event loop) and made transient for the app's active window.

use std::sync::mpsc;

use gtk::{glib, prelude::*};

use super::message::{Buttons, Kind, MessageOutcome, MessageSpec};

fn active_window() -> Option<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(GtkWindowExt::is_active)
}

fn run(spec: &MessageSpec) -> MessageOutcome {
    let parent = if spec.parent { active_window() } else { None };
    let message_type = match spec.kind {
        Kind::None => gtk::MessageType::Other,
        Kind::Info => gtk::MessageType::Info,
        Kind::Warning => gtk::MessageType::Warning,
        Kind::Error => gtk::MessageType::Error,
        Kind::Question => gtk::MessageType::Question,
    };
    let preset = match spec.buttons {
        Buttons::Ok => gtk::ButtonsType::Ok,
        Buttons::OkCancel => gtk::ButtonsType::OkCancel,
        Buttons::Custom(_) => gtk::ButtonsType::None,
    };
    let dialog = gtk::MessageDialog::new(
        parent.as_ref(),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        message_type,
        preset,
        &spec.message,
    );
    dialog.set_title(&spec.title);
    if let Some(detail) = &spec.detail {
        dialog.set_secondary_text(Some(detail));
    }
    if let Buttons::Custom(labels) = &spec.buttons {
        for (index, label) in labels.iter().enumerate() {
            let id = u16::try_from(index).unwrap_or(u16::MAX);
            dialog.add_button(label, gtk::ResponseType::Other(id));
        }
    }
    dialog.set_default_response(match spec.buttons {
        Buttons::Custom(_) => {
            gtk::ResponseType::Other(u16::try_from(spec.default_id).unwrap_or_default())
        },
        Buttons::OkCancel if spec.default_id == 1 => gtk::ResponseType::Cancel,
        Buttons::Ok | Buttons::OkCancel => gtk::ResponseType::Ok,
    });
    let checkbox = spec.checkbox_label.as_deref().map(|label| {
        let checkbox = gtk::CheckButton::with_label(label);
        checkbox.set_active(spec.checkbox_checked);
        if let Ok(area) = dialog.message_area().downcast::<gtk::Box>() {
            area.pack_start(&checkbox, false, false, 0);
        }
        checkbox.show();
        checkbox
    });

    let response = dialog.run();
    let checked = checkbox.as_ref().is_some_and(ToggleButtonExt::is_active);
    dialog.close();

    let index = match response {
        gtk::ResponseType::Other(id) if usize::from(id) < spec.buttons.len() => usize::from(id),
        gtk::ResponseType::Ok => 0,
        gtk::ResponseType::Cancel if spec.buttons == Buttons::OkCancel => 1,
        _ => spec.cancel_id,
    };
    MessageOutcome {
        response: index,
        checkbox_checked: checked,
    }
}

pub(super) fn show(spec: &MessageSpec) -> Result<MessageOutcome, String> {
    let (tx, rx) = mpsc::sync_channel::<MessageOutcome>(1);
    let owned = spec.clone();
    glib::MainContext::default().invoke(move || {
        let _ = tx.send(run(&owned));
    });
    rx.recv()
        .map_err(|_| "the dialog was closed without a response".to_owned())
}
