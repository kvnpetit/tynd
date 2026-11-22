//! Pure capability, ACL, and identifier contracts shared across Tynd crates.
//!
//! This crate intentionally owns data shapes and deterministic matching helpers
//! only. Runtime state, disk persistence, IPC dispatch, and OS integration stay
//! in `tynd-host`.

#![forbid(unsafe_code)]

mod identifiers;
mod manifest;
mod model;
mod resolver;
mod schema;

pub use identifiers::{
    ApiName, CapabilityId, Command, CommandParseError, EventName, MethodName, PluginName,
    WindowLabel,
};
pub use manifest::{
    command_lookup_keys, manifest_command_key, manifest_from_plugin_commands, plugin_command_key,
    validate_plugin_name,
};
pub use model::{
    Capability, CapabilityFile, Commands, Manifest, Permission, PermissionEntry, RemoteCapability,
    Target,
};
pub use resolver::{platforms_match, split_permission_ref, Resolved, APP_ACL_KEY};
pub use schema::{capability_schema, validate_capability_shape};

#[cfg(test)]
mod tests;
