use std::cell::RefCell;

/// Lifecycle and navigation events the host broadcasts to every initialized
/// plugin through [`crate::TyndPlugin::on_event`].
///
/// The enum is `#[non_exhaustive]`: always keep a `_ => {}` arm so new events
/// do not break existing plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostEvent {
    /// The primary webview finished loading and the app is interactive.
    AppReady,
    /// The app is shutting down; fired before any plugin `shutdown` hook.
    AppExit,
    /// A native window was created.
    WindowCreated {
        /// Window label.
        label: String,
    },
    /// A native window was closed.
    WindowClosed {
        /// Window label.
        label: String,
    },
    /// A native window gained or lost focus.
    WindowFocus {
        /// Window label.
        label: String,
        /// `true` when the window gained focus.
        focused: bool,
    },
    /// A webview started navigating to `url`.
    Navigation {
        /// Native window label owning the webview.
        window: String,
        /// Webview label.
        webview: String,
        /// Target URL.
        url: String,
    },
}

/// Identity of the renderer that issued the call being dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerInfo {
    /// Native window label.
    pub window: String,
    /// Webview label (equals `window` for top-level webviews).
    pub webview: String,
    /// Page origin when the call comes from a remote origin, `None` for the
    /// bundled app.
    pub origin: Option<String>,
}

thread_local! {
    static CALLER: RefCell<Option<CallerInfo>> = const { RefCell::new(None) };
}

/// Run `f` with `caller` visible to [`crate::PluginCx::caller`] on this thread.
///
/// Called by the host around a renderer-originated plugin dispatch; the
/// previous value is restored afterwards, even if `f` panics. Threads spawned
/// by a plugin do not inherit the caller.
pub fn with_caller<R>(caller: Option<CallerInfo>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<CallerInfo>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CALLER.with(|slot| *slot.borrow_mut() = previous);
        }
    }
    let previous = CALLER.with(|slot| slot.replace(caller));
    let _restore = Restore(previous);
    f()
}

/// Caller of the plugin dispatch running on this thread, if any. Host use; plugins call [`crate::PluginCx::caller`].
pub fn current_caller() -> Option<CallerInfo> {
    CALLER.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(window: &str) -> CallerInfo {
        CallerInfo {
            window: window.to_string(),
            webview: window.to_string(),
            origin: None,
        }
    }

    #[test]
    fn caller_is_scoped_and_restored() {
        assert_eq!(current_caller(), None);
        with_caller(Some(caller("a")), || {
            assert_eq!(current_caller(), Some(caller("a")));
            with_caller(Some(caller("b")), || {
                assert_eq!(current_caller(), Some(caller("b")));
            });
            assert_eq!(current_caller(), Some(caller("a")));
        });
        assert_eq!(current_caller(), None);
    }

    #[test]
    fn caller_is_restored_after_panic() {
        let result = std::panic::catch_unwind(|| {
            with_caller(Some(caller("a")), || panic!("boom"));
        });
        assert!(result.is_err());
        assert_eq!(current_caller(), None);
    }

    #[test]
    fn caller_does_not_leak_to_other_threads() {
        with_caller(Some(caller("a")), || {
            let seen = std::thread::spawn(current_caller).join().unwrap();
            assert_eq!(seen, None);
        });
    }
}
