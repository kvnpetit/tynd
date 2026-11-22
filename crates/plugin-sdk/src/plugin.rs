use serde_json::Value;

use crate::{HostEvent, PluginCx, PluginResult};

/// Implemented by every native plugin crate.
///
/// # Lifetime and teardown
///
/// The host keeps every plugin in a process-wide static registry and never
/// drops it: **`Drop` does not run at host exit** for the plugin struct or
/// anything it owns (child processes, background threads, native handles,
/// files that need a flush). Put all teardown in [`shutdown`](Self::shutdown)
/// (the `plugin_shutdown` hook of `#[plugin]`): signal workers, drain them with
/// a bounded wait ([`GatedStreams::shutdown`](crate::GatedStreams::shutdown),
/// [`join_with_timeout`](crate::join_with_timeout),
/// [`BoundedPermitPool::wait_idle`](crate::BoundedPermitPool::wait_idle)) and
/// release OS resources explicitly.
pub trait TyndPlugin: Send + Sync + 'static {
    /// Stable identifier used in wire-format calls.
    fn name(&self) -> &'static str;

    /// Stable wire command names exposed by this plugin.
    ///
    /// `#[tynd_plugin::plugin_impl]` generates this list automatically.
    /// Hand-written implementations may override it to opt into generated
    /// renderer permission manifests; an empty list keeps named permissions
    /// disabled and requires inline capability commands.
    fn command_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// Called once when the host registers the plugin.
    fn init(&self, _cx: &PluginCx) -> PluginResult<()> {
        Ok(())
    }

    /// Handle one RPC call.
    fn dispatch(&self, cx: &PluginCx, method: &str, args: Value) -> PluginResult<Value>;

    /// Optional rkyv zero-copy dispatch. Return `None` to fall back to JSON.
    fn dispatch_rkyv(
        &self,
        _cx: &PluginCx,
        _method: &str,
        _args: &[u8],
    ) -> Option<PluginResult<Vec<u8>>> {
        None
    }

    /// Called for every host lifecycle / navigation event ([`HostEvent`]).
    ///
    /// The default implementation ignores the event. Runs on a host thread
    /// (often the UI thread): return quickly and hand slow work to a
    /// background thread. A panic is caught and logged.
    fn on_event(&self, _cx: &PluginCx, _event: &HostEvent) {}

    /// Graceful teardown, called once when the host exits. This is the only
    /// teardown hook: `Drop` never runs for a registered plugin (see the trait
    /// docs). Keep it bounded: it blocks the host's exit path.
    fn shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        Ok(())
    }
}
