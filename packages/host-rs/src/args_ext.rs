//! Backward-compat re-export — `ArgsExt` lives in the `tynd-host-core`
//! leaf crate (U12 phase 1 split). See [`tynd_host_core::args_ext`] for
//! the full doc + examples.
//!
//! Existing `use crate::args_ext::ArgsExt` / `use tynd_host::args_ext::ArgsExt`
//! call sites keep working through this re-export; new code in any host
//! crate should reach for `tynd_host_core::args_ext::ArgsExt` directly.

pub use tynd_host_core::args_ext::*;
