//! OS helpers shared by several modules; none is a host API.

pub(crate) mod color_scheme;
pub(crate) mod icon;
#[cfg(any(feature = "api-single-instance", feature = "api-deep-link"))]
pub(crate) mod local_listener;
// Owner-only DACL builder for named pipes, used by `single_instance` and
// `deep_link::forwarding` on Windows; both already require `interprocess`,
// which exposes the `SecurityDescriptor` type it builds.
#[cfg(all(
    windows,
    any(feature = "api-single-instance", feature = "api-deep-link")
))]
pub(crate) mod win_security;
