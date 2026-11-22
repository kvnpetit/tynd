//! Windows backend: WinRT toast notifications.
//!
//! The toast XML is built through the `XmlDocument` DOM (escaping is the
//! DOM's job). Each toast carries `Tag = <id>` / `Group = "tynd"`, so the
//! notification center history can be listed and individual toasts removed.

use std::collections::HashSet;

use windows::{
    core::{Interface, HSTRING},
    Data::Xml::Dom::{XmlDocument, XmlElement},
    Foundation::{IPropertyValue, TypedEventHandler},
    UI::Notifications::{
        NotificationSetting, ToastActivatedEventArgs, ToastDismissalReason,
        ToastDismissedEventArgs, ToastNotification, ToastNotificationManager,
    },
};

use super::{emit_event, exe_stem, NotifEvent, NotifSpec, NotifTimeout, Urgency};

/// Toast group of every notification of the app.
const GROUP: &str = "tynd";
/// `arguments` of the reply button and id of its text input.
const REPLY: &str = "reply";
/// `launch` arguments of a body click.
const CLICK: &str = "click";
/// Prefix of an action button's `arguments`.
const ACTION_PREFIX: &str = "action:";

/// The shown toast, kept so its event handlers stay registered.
#[derive(Default)]
pub(super) struct Handle(
    #[allow(dead_code, reason = "held for its lifetime")] Option<ToastNotification>,
);

pub(super) fn check_supported(_spec: &NotifSpec, _op: &str) -> Result<(), String> {
    Ok(())
}

pub(super) fn check_delivered_supported(_op: &str) -> Result<(), String> {
    Ok(())
}

fn win_error(op: &str, error: &windows::core::Error) -> String {
    format!("notification.{op}: {error}")
}

fn app_id() -> HSTRING {
    HSTRING::from(exe_stem())
}

/// Ids of the app's toasts still in the notification center.
pub(super) fn live_ids() -> Option<HashSet<u64>> {
    let history = ToastNotificationManager::History().ok()?;
    let toasts = history.GetHistoryWithId(&app_id()).ok()?;
    let mut ids = HashSet::new();
    for index in 0..toasts.Size().ok()? {
        let Ok(toast) = toasts.GetAt(index) else {
            continue;
        };
        let group = toast.Group().map(|g| g.to_string()).unwrap_or_default();
        if group != GROUP {
            continue;
        }
        if let Some(id) = toast
            .Tag()
            .ok()
            .and_then(|tag| tag.to_string().parse().ok())
        {
            ids.insert(id);
        }
    }
    Some(ids)
}

pub(super) fn remove(id: u64, _handle: &Handle) -> Result<(), String> {
    let history = ToastNotificationManager::History().map_err(|e| win_error("remove", &e))?;
    history
        .RemoveGroupedTagWithId(
            &HSTRING::from(id.to_string()),
            &HSTRING::from(GROUP),
            &app_id(),
        )
        .map_err(|e| win_error("remove", &e))
}

pub(super) fn permission() -> &'static str {
    let setting = ToastNotificationManager::CreateToastNotifierWithId(&app_id())
        .and_then(|notifier| notifier.Setting());
    match setting {
        Ok(NotificationSetting::Enabled) | Err(_) => "granted",
        Ok(_) => "denied",
    }
}

fn element(
    doc: &XmlDocument,
    parent: &XmlElement,
    name: &str,
    attrs: &[(&str, &str)],
) -> windows::core::Result<XmlElement> {
    let el = doc.CreateElement(&HSTRING::from(name))?;
    for (key, value) in attrs {
        el.SetAttribute(&HSTRING::from(*key), &HSTRING::from(*value))?;
    }
    parent.AppendChild(&el)?;
    Ok(el)
}

fn file_uri(path: &str) -> String {
    format!("file:///{}", path.replace('\\', "/"))
}

/// Toast XML for `spec`.
fn toast_xml(spec: &NotifSpec) -> windows::core::Result<XmlDocument> {
    let doc = XmlDocument::new()?;
    let toast = doc.CreateElement(&HSTRING::from("toast"))?;
    doc.AppendChild(&toast)?;
    toast.SetAttribute(&HSTRING::from("launch"), &HSTRING::from(CLICK))?;
    let mut scenario = None;
    match spec.timeout {
        NotifTimeout::Default => {},
        NotifTimeout::Never => scenario = Some("reminder"),
        NotifTimeout::Millis(ms) => {
            let duration = if ms > 7_000 { "long" } else { "short" };
            toast.SetAttribute(&HSTRING::from("duration"), &HSTRING::from(duration))?;
        },
    }
    if spec.urgency == Urgency::Critical {
        scenario = Some("urgent");
    }
    if let Some(scenario) = scenario {
        toast.SetAttribute(&HSTRING::from("scenario"), &HSTRING::from(scenario))?;
    }

    let visual = element(&doc, &toast, "visual", &[])?;
    let binding = element(&doc, &visual, "binding", &[("template", "ToastGeneric")])?;
    element(&doc, &binding, "text", &[])?.SetInnerText(&HSTRING::from(spec.title.as_str()))?;
    if !spec.body.is_empty() {
        element(&doc, &binding, "text", &[])?.SetInnerText(&HSTRING::from(spec.body.as_str()))?;
    }
    if let Some(icon) = &spec.icon {
        element(
            &doc,
            &binding,
            "image",
            &[("placement", "appLogoOverride"), ("src", &file_uri(icon))],
        )?;
    }
    if let Some(image) = &spec.image {
        element(&doc, &binding, "image", &[("src", &file_uri(image))])?;
    }

    if spec.silent {
        element(&doc, &toast, "audio", &[("silent", "true")])?;
    } else if let Some(sound) = spec
        .sound
        .as_deref()
        .filter(|s| !s.eq_ignore_ascii_case("default"))
    {
        let src = format!("ms-winsoundevent:Notification.{sound}");
        element(&doc, &toast, "audio", &[("src", &src)])?;
    }

    append_actions(&doc, &toast, spec)?;
    Ok(doc)
}

/// `<actions>`: reply field, action buttons and the dismiss button a
/// never-expiring toast needs.
fn append_actions(
    doc: &XmlDocument,
    toast: &XmlElement,
    spec: &NotifSpec,
) -> windows::core::Result<()> {
    let needs_dismiss =
        spec.timeout == NotifTimeout::Never && spec.actions.is_empty() && spec.reply.is_none();
    if !spec.actions.is_empty() || spec.reply.is_some() || needs_dismiss {
        let actions = element(doc, toast, "actions", &[])?;
        if let Some(reply) = &spec.reply {
            element(
                doc,
                &actions,
                "input",
                &[
                    ("id", REPLY),
                    ("type", "text"),
                    (
                        "placeHolderContent",
                        reply.placeholder.as_deref().unwrap_or(""),
                    ),
                ],
            )?;
            element(
                doc,
                &actions,
                "action",
                &[
                    ("content", reply.button_label.as_deref().unwrap_or("Send")),
                    ("arguments", REPLY),
                    ("hint-inputId", REPLY),
                ],
            )?;
        }
        for action in &spec.actions {
            let arguments = format!("{ACTION_PREFIX}{}", action.id);
            element(
                doc,
                &actions,
                "action",
                &[
                    ("content", action.label.as_str()),
                    ("arguments", &arguments),
                ],
            )?;
        }
        if needs_dismiss {
            // A reminder stays on screen only while it has a button.
            element(
                doc,
                &actions,
                "action",
                &[
                    ("activationType", "system"),
                    ("arguments", "dismiss"),
                    ("content", ""),
                ],
            )?;
        }
    }
    Ok(())
}

/// Map a toast activation to an event.
fn activation_event(args: &ToastActivatedEventArgs) -> NotifEvent {
    let arguments = args.Arguments().map(|a| a.to_string()).unwrap_or_default();
    if arguments == REPLY {
        let text = args
            .UserInput()
            .and_then(|input| input.Lookup(&HSTRING::from(REPLY)))
            .and_then(|value| value.cast::<IPropertyValue>())
            .and_then(|value| value.GetString())
            .map(|text| text.to_string())
            .unwrap_or_default();
        return NotifEvent::Reply(text);
    }
    match arguments.strip_prefix(ACTION_PREFIX) {
        Some(action) => NotifEvent::Action(action.to_owned()),
        None => NotifEvent::Click,
    }
}

fn dismissal_reason(args: &ToastDismissedEventArgs) -> &'static str {
    match args.Reason() {
        Ok(ToastDismissalReason::UserCanceled) => "user",
        Ok(ToastDismissalReason::TimedOut) => "timeout",
        Ok(ToastDismissalReason::ApplicationHidden) => "app",
        _ => "unknown",
    }
}

pub(super) fn show(id: u64, spec: &NotifSpec) -> Result<Handle, String> {
    let build = || -> windows::core::Result<ToastNotification> {
        let toast = ToastNotification::CreateToastNotification(&toast_xml(spec)?)?;
        toast.SetTag(&HSTRING::from(id.to_string()))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        toast.Activated(&TypedEventHandler::new(
            move |_, args: windows::core::Ref<'_, windows::core::IInspectable>| {
                let event = args
                    .as_ref()
                    .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
                    .map_or(NotifEvent::Click, |args| activation_event(&args));
                emit_event(id, &event);
                Ok(())
            },
        ))?;
        toast.Dismissed(&TypedEventHandler::new(
            move |_, args: windows::core::Ref<'_, ToastDismissedEventArgs>| {
                let reason = args.as_ref().map_or("unknown", dismissal_reason);
                emit_event(id, &NotifEvent::Close(reason));
                Ok(())
            },
        ))?;
        ToastNotificationManager::CreateToastNotifierWithId(&app_id())?.Show(&toast)?;
        Ok(toast)
    };
    build()
        .map(|toast| Handle(Some(toast)))
        .map_err(|e| win_error("send", &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::notification::{NotifAction, NotifActionId, NotifReply};

    fn xml(spec: &NotifSpec) -> String {
        let doc = toast_xml(spec).expect("toast_xml");
        doc.GetXml().expect("GetXml").to_string()
    }

    #[test]
    fn toast_xml_carries_every_option_escaped() {
        let spec = NotifSpec {
            title: "Tom & <Jerry>".into(),
            body: "body".into(),
            image: Some(r"C:\img\a.png".into()),
            sound: Some("Mail".into()),
            urgency: Urgency::Critical,
            timeout: NotifTimeout::Millis(20_000),
            actions: vec![NotifAction {
                id: NotifActionId::try_new("ok".to_owned()).unwrap(),
                label: "OK \"now\"".into(),
            }],
            reply: Some(NotifReply {
                placeholder: Some("Type".into()),
                button_label: None,
            }),
            ..NotifSpec::default()
        };
        let xml = xml(&spec);
        for needle in [
            "launch=\"click\"",
            "duration=\"long\"",
            "scenario=\"urgent\"",
            "Tom &amp; &lt;Jerry&gt;",
            "src=\"file:///C:/img/a.png\"",
            "ms-winsoundevent:Notification.Mail",
            "<input id=\"reply\" type=\"text\" placeHolderContent=\"Type\"/>",
            "arguments=\"reply\"",
            "arguments=\"action:ok\"",
            "OK &quot;now&quot;",
        ] {
            assert!(xml.contains(needle), "{needle} missing in {xml}");
        }
    }

    #[test]
    fn silent_never_toast_gets_a_dismiss_button() {
        let spec = NotifSpec {
            title: "t".into(),
            silent: true,
            timeout: NotifTimeout::Never,
            ..NotifSpec::default()
        };
        let xml = xml(&spec);
        assert!(xml.contains("<audio silent=\"true\"/>"), "{xml}");
        assert!(xml.contains("scenario=\"reminder\""), "{xml}");
        assert!(xml.contains("activationType=\"system\""), "{xml}");
    }
}
