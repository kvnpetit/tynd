//! Render-pass orchestration helpers.
//!
//! Today this module is mostly a capability probe: [`available_backends`]
//! returns the list of `wgpu::Backend`s the host has actually compiled in,
//! which `available()` echoes back to the JS side so the frontend can
//! pre-flight a `createSurface` call.
//!
//! Once the IPC bridge in [`super::bridge`] lands, the bulk of the per-
//! frame work — building `RenderPassDescriptor`s from JSON command
//! batches, managing pipeline / bind-group caches, recycling staging
//! buffers — moves here. Keeping it in its own file from day one means
//! the dispatch-side (`lib.rs`) doesn't need to grow when that work
//! happens.

#![cfg(feature = "native")]

/// List the `wgpu::Backend`s baked into this build. Used by the
/// `available()` IPC method so the frontend can show "Metal / DX12 /
/// Vulkan available" before the user clicks "Start render".
///
/// Today the list is a static reflection of the Cargo features set on
/// the `wgpu` dependency in `Cargo.toml`. A future improvement would be
/// to actually probe `wgpu::Instance::enumerate_adapters` to filter out
/// backends that compiled in but have no working driver on the host
/// (e.g. Vulkan on a Mac without MoltenVK installed).
pub(crate) fn available_backends() -> Vec<String> {
    let mut out = Vec::with_capacity(4);
    // Mirror the `features = [...]` block on `wgpu` in Cargo.toml. If
    // someone trims that list, this should follow — keeps the JS-side
    // capability check honest.
    if cfg!(target_os = "macos") || cfg!(target_os = "ios") {
        out.push("metal".into());
    }
    if cfg!(target_os = "windows") {
        out.push("dx12".into());
    }
    out.push("vulkan".into());
    out.push("gl".into());
    out
}
