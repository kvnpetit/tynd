//! macOS Dock: activation policy (Dock icon shown / hidden), attention
//! bounce, custom icon and the Dock menu. Every call rejects on Windows and
//! Linux, which have no Dock.
//!
//! The Dock menu is served by an `applicationDockMenu:` method added at
//! runtime to the class of tao's application delegate (tao does not
//! implement it). The method returns the `NSMenu` of the last
//! `app.setDockMenu` call, built with the shared menu registry so clicks emit
//! the usual `menu:action` event and its radio groups stay exclusive.

/// `NSApplicationActivationPolicy` as accepted by `setActivationPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Policy {
    /// Dock icon + menu bar (default).
    Regular,
    /// No Dock icon, no menu bar; windows still show (menu-bar apps).
    Accessory,
    /// No Dock icon and no windows can activate the app.
    Prohibited,
}

pub(super) fn parse_policy(op: &str, policy: &str) -> Result<Policy, String> {
    match policy {
        "regular" => Ok(Policy::Regular),
        "accessory" => Ok(Policy::Accessory),
        "prohibited" => Ok(Policy::Prohibited),
        other => Err(format!(
            "app.{op}: unknown policy '{other}'; expected \"regular\", \"accessory\" or \"prohibited\""
        )),
    }
}

/// `true` for a critical bounce (repeats until the app is activated).
pub(super) fn parse_bounce(kind: Option<&str>) -> Result<bool, String> {
    match kind.unwrap_or("informational") {
        "informational" => Ok(false),
        "critical" => Ok(true),
        other => Err(format!(
            "app.dockBounce: unknown type '{other}'; expected \"informational\" or \"critical\""
        )),
    }
}

/// Most top-level items accepted for the Dock menu.
const MAX_DOCK_MENU_ITEMS: usize = 64;

/// Parse a Dock menu template (same item shape as `menu.setApplicationMenu`).
pub(super) fn parse_menu(
    items: serde_json::Value,
) -> Result<Vec<tynd_protocol::runtime::MenuItemDef>, String> {
    if !items.is_array() {
        return Err("app.setDockMenu: 'items' must be an array".into());
    }
    let items: Vec<tynd_protocol::runtime::MenuItemDef> = serde_json::from_value(items)
        .map_err(|e| format!("app.setDockMenu: invalid items — {e}"))?;
    if items.len() > MAX_DOCK_MENU_ITEMS {
        return Err(format!(
            "app.setDockMenu: at most {MAX_DOCK_MENU_ITEMS} top-level items are allowed"
        ));
    }
    Ok(items)
}

#[cfg(all(target_os = "macos", feature = "api-menu"))]
pub(super) use mac::set_menu;
#[cfg(target_os = "macos")]
pub(super) use mac::{bounce, cancel_bounce, set_icon, set_policy};

#[cfg(all(not(target_os = "macos"), feature = "api-menu"))]
pub(super) use other::set_menu;

/// Builds without the menu API cannot build a Dock menu.
#[cfg(not(feature = "api-menu"))]
pub(super) fn set_menu(_items: Vec<tynd_protocol::runtime::MenuItemDef>) -> Result<(), String> {
    Err("this build has no menu support; enable the `menu` API".into())
}
#[cfg(not(target_os = "macos"))]
pub(super) use other::{bounce, cancel_bounce, set_icon, set_policy};

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use dispatch2::run_on_main;
    use objc2::AllocAnyThread as _;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSImage, NSRequestUserAttentionType,
    };
    use objc2_foundation::NSString;

    use super::Policy;

    pub(in crate::api::app) fn set_policy(policy: Policy) -> Result<(), String> {
        let policy = match policy {
            Policy::Regular => NSApplicationActivationPolicy::Regular,
            Policy::Accessory => NSApplicationActivationPolicy::Accessory,
            Policy::Prohibited => NSApplicationActivationPolicy::Prohibited,
        };
        let applied = run_on_main(move |mtm| {
            NSApplication::sharedApplication(mtm).setActivationPolicy(policy)
        });
        if applied {
            Ok(())
        } else {
            Err("AppKit refused the activation policy".into())
        }
    }

    pub(in crate::api::app) fn bounce(critical: bool) -> Result<i64, String> {
        let kind = if critical {
            NSRequestUserAttentionType::CriticalRequest
        } else {
            NSRequestUserAttentionType::InformationalRequest
        };
        Ok(run_on_main(move |mtm| {
            NSApplication::sharedApplication(mtm).requestUserAttention(kind) as i64
        }))
    }

    pub(in crate::api::app) fn cancel_bounce(id: i64) -> Result<(), String> {
        let Ok(id) = isize::try_from(id) else {
            return Ok(());
        };
        run_on_main(move |mtm| {
            NSApplication::sharedApplication(mtm).cancelUserAttentionRequest(id)
        });
        Ok(())
    }

    /// `None` restores the bundle icon.
    pub(in crate::api::app) fn set_icon(path: Option<PathBuf>) -> Result<(), String> {
        let path = match path {
            Some(path) => Some(
                path.to_str()
                    .ok_or_else(|| "icon path is not valid UTF-8".to_string())?
                    .to_owned(),
            ),
            None => None,
        };
        run_on_main(move |mtm| {
            let image = match &path {
                Some(path) => {
                    let image = NSImage::initWithContentsOfFile(
                        NSImage::alloc(),
                        &NSString::from_str(path),
                    )
                    .ok_or_else(|| format!("'{path}' is not an image AppKit can read"))?;
                    Some(image)
                },
                None => None,
            };
            // SAFETY: nil restores the default icon; otherwise a valid NSImage.
            unsafe {
                NSApplication::sharedApplication(mtm).setApplicationIconImage(image.as_deref());
            }
            Ok(())
        })
    }

    #[cfg(feature = "api-menu")]
    pub(in crate::api::app) use dock_menu::set_menu;

    #[cfg(feature = "api-menu")]
    mod dock_menu {
        use std::{
            cell::RefCell,
            ffi::c_void,
            rc::Rc,
            sync::{
                atomic::{AtomicPtr, Ordering},
                OnceLock,
            },
        };

        use dispatch2::run_on_main;
        use muda::ContextMenu as _;
        use objc2::{
            ffi::class_addMethod,
            runtime::{AnyClass, AnyObject, Imp, Sel},
            sel,
        };
        use objc2_app_kit::NSApplication;
        use tynd_protocol::runtime::MenuItemDef;

        use crate::api::menu::registry::{self, MenuRegistry, Scope};

        thread_local! {
            /// Main-thread owner of the current Dock menu and its item
            /// registry; replaced on every `setDockMenu`, which keeps its
            /// `NSMenu` alive.
            static MENU: RefCell<Option<Rc<MenuRegistry>>> = const { RefCell::new(None) };
        }

        /// `NSMenu*` returned to AppKit, null for "no Dock menu".
        static NS_MENU: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
        /// Outcome of adding `applicationDockMenu:`, returned on every call.
        static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

        /// `- (NSMenu *)applicationDockMenu:(NSApplication *)sender`.
        extern "C-unwind" fn application_dock_menu(
            _this: *mut AnyObject,
            _cmd: Sel,
            _sender: *mut AnyObject,
        ) -> *mut c_void {
            NS_MENU.load(Ordering::Acquire)
        }

        /// Add `applicationDockMenu:` to the delegate class once (main
        /// thread). Returns an error when there is no delegate yet (retried
        /// on the next call) or the class already defines the method (another
        /// component owns the Dock menu; kept for every later call).
        fn install(app: &NSApplication) -> Result<(), String> {
            if let Some(result) = INSTALLED.get() {
                return result.clone();
            }
            let Some(delegate) = app.delegate() else {
                return Err("the application has no delegate yet".to_string());
            };
            INSTALLED
                .get_or_init(|| {
                    let object: &AnyObject = delegate.as_ref();
                    let class: *const AnyClass = object.class();
                    // SAFETY: `application_dock_menu` matches the `@@:@` type
                    // encoding (object return, self, _cmd, one object argument);
                    // class_addMethod fails without side effects when the class
                    // already implements the selector.
                    let added = unsafe {
                        let imp: Imp = std::mem::transmute::<
                            extern "C-unwind" fn(
                                *mut AnyObject,
                                Sel,
                                *mut AnyObject,
                            ) -> *mut c_void,
                            Imp,
                        >(application_dock_menu);
                        class_addMethod(
                            class.cast_mut(),
                            sel!(applicationDockMenu:),
                            imp,
                            c"@@:@".as_ptr(),
                        )
                    };
                    if added.as_bool() {
                        Ok(())
                    } else {
                        Err("the application delegate already defines a Dock menu".into())
                    }
                })
                .clone()
        }

        pub(in crate::api::app) fn set_menu(items: Vec<MenuItemDef>) -> Result<(), String> {
            run_on_main(move |mtm| {
                install(&NSApplication::sharedApplication(mtm))?;
                if items.is_empty() {
                    NS_MENU.store(std::ptr::null_mut(), Ordering::Release);
                    MENU.with(|slot| slot.borrow_mut().take());
                    return Ok(());
                }
                let menu = Rc::new(MenuRegistry::build(Scope::DOCK, &items)?);
                NS_MENU.store(menu.root().ns_menu(), Ordering::Release);
                registry::track(&menu);
                MENU.with(|slot| *slot.borrow_mut() = Some(menu));
                Ok(())
            })
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::path::PathBuf;

    use super::Policy;

    fn unsupported() -> String {
        format!(
            "the Dock is macOS only; not supported on {}",
            std::env::consts::OS
        )
    }

    pub(in crate::api::app) fn set_policy(_policy: Policy) -> Result<(), String> {
        Err(unsupported())
    }

    pub(in crate::api::app) fn bounce(_critical: bool) -> Result<i64, String> {
        Err(unsupported())
    }

    pub(in crate::api::app) fn cancel_bounce(_id: i64) -> Result<(), String> {
        Err(unsupported())
    }

    pub(in crate::api::app) fn set_icon(_path: Option<PathBuf>) -> Result<(), String> {
        Err(unsupported())
    }

    #[cfg(feature = "api-menu")]
    pub(in crate::api::app) fn set_menu(
        _items: Vec<tynd_protocol::runtime::MenuItemDef>,
    ) -> Result<(), String> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_and_bounce_types_are_validated() {
        assert_eq!(parse_policy("op", "accessory").unwrap(), Policy::Accessory);
        assert_eq!(parse_policy("op", "regular").unwrap(), Policy::Regular);
        assert_eq!(
            parse_policy("op", "prohibited").unwrap(),
            Policy::Prohibited
        );
        assert!(parse_policy("op", "hidden")
            .unwrap_err()
            .contains("\"accessory\""));
        assert!(!parse_bounce(None).unwrap());
        assert!(parse_bounce(Some("critical")).unwrap());
        assert!(parse_bounce(Some("loud")).is_err());
    }

    #[test]
    fn dock_menu_template_is_validated() {
        assert!(parse_menu(serde_json::json!({})).is_err());
        assert!(parse_menu(serde_json::json!([{ "label": "New window", "id": "new" }])).is_ok());
        let many: Vec<_> = (0..65)
            .map(|_| serde_json::json!({ "type": "separator" }))
            .collect();
        assert!(parse_menu(serde_json::Value::Array(many))
            .unwrap_err()
            .contains("at most"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn rejects_off_macos() {
        assert!(set_policy(Policy::Accessory)
            .unwrap_err()
            .contains("not supported on"));
        assert!(bounce(false).is_err());
    }
}
