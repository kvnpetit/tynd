//! Linux keep-awake through systemd-logind. `Manager.Inhibit` hands back a file
//! descriptor; the inhibitor lives exactly as long as that fd stays open, so
//! dropping the handle releases it (and logind releases it for us if the
//! process dies). Display requests also take an `org.freedesktop.ScreenSaver`
//! inhibit on the session bus, best effort, since desktops that do not run
//! that service still honor the logind `idle` inhibitor.

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedFd,
};

const APP_NAME: &str = "Tynd";
const REASON: &str = "Tynd app requested keep-awake";
/// logind lock kinds. Only `idle` is taken: it stops the idle action (the
/// same scope as Windows `PowerRequestSystemRequired` and macOS
/// `PreventUserIdleSystemSleep`). `sleep` would also block explicit suspends
/// such as closing the lid or the session menu.
const LOGIND_WHAT: &str = "idle";

struct ScreenSaverInhibit {
    connection: Connection,
    cookie: u32,
}

impl ScreenSaverInhibit {
    fn acquire() -> Result<Self, String> {
        let connection = Connection::session().map_err(|error| format!("session bus: {error}"))?;
        let cookie = screensaver_proxy(&connection)?
            .call("Inhibit", &(APP_NAME, REASON))
            .map_err(|error| format!("ScreenSaver.Inhibit: {error}"))?;
        Ok(Self { connection, cookie })
    }
}

impl Drop for ScreenSaverInhibit {
    fn drop(&mut self) {
        if let Ok(proxy) = screensaver_proxy(&self.connection) {
            let _: Result<(), _> = proxy.call("UnInhibit", &(self.cookie));
        }
    }
}

fn screensaver_proxy(connection: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(
        connection,
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver",
    )
    .map_err(|error| format!("ScreenSaver proxy: {error}"))
}

pub(super) struct Handle {
    /// Closing the fd releases the logind inhibitor.
    _logind: OwnedFd,
    _screensaver: Option<ScreenSaverInhibit>,
}

pub(super) fn acquire(display: bool) -> Result<Handle, String> {
    let connection = Connection::system().map_err(|error| format!("system bus: {error}"))?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|error| format!("power.preventSleep: Manager proxy: {error}"))?;
    let logind: OwnedFd = manager
        .call("Inhibit", &(LOGIND_WHAT, APP_NAME, REASON, "block"))
        .map_err(|error| format!("power.preventSleep: logind Inhibit: {error}"))?;

    let screensaver = if display {
        match ScreenSaverInhibit::acquire() {
            Ok(inhibit) => Some(inhibit),
            Err(error) => {
                crate::tynd_warn!("power.preventSleep: display inhibit unavailable: {error}");
                None
            },
        }
    } else {
        None
    };
    Ok(Handle {
        _logind: logind,
        _screensaver: screensaver,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inhibits_idle_only_like_windows_and_macos() {
        assert_eq!(LOGIND_WHAT, "idle");
        assert!(!LOGIND_WHAT.contains("sleep"));
    }
}
