//! OS helpers shared by several modules; none is a host API.

pub(crate) mod color_scheme;
pub(crate) mod icon;
#[cfg(any(feature = "api-single-instance", feature = "api-deep-link"))]
pub(crate) mod local_listener;
// Taskbar overlay badge (`window.setBadge` on Windows).
#[cfg(windows)]
pub(crate) mod taskbar_badge;
// Window materials (`window.setEffects`).
pub(crate) mod window_effects;
// Owner-only DACL builder for named pipes, used by `single_instance` and
// `deep_link::forwarding` on Windows; both already require `interprocess`,
// which exposes the `SecurityDescriptor` type it builds.
#[cfg(all(
    windows,
    any(feature = "api-single-instance", feature = "api-deep-link")
))]
pub(crate) mod win_security;
