//! Native menu facade: shared builders, popup context menus, and application menu bar.
//!
//! This is the `api-menu` build of the module. Without the feature,
//! `menu_disabled.rs` supplies the same crate-private surface as stubs (and
//! only the `builder` submodule when `api-tray` still needs it), so the event
//! loop keeps one code path without linking `muda`.

mod bar;
pub(crate) mod builder;
mod context;

pub(crate) use bar::{build_bar, dispatch, init_bar};
pub(crate) use context::show_context_menu;

/// Handle that keeps the application menu bar alive for the event loop's lifetime.
pub(crate) type AppMenu = muda::Menu;
