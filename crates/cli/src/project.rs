mod binaries;
mod custom_host;
mod discovery;
mod host_cache;
mod platform;
mod rust_toolchain;
mod shared;

pub(crate) use binaries::{resolve_binary, TyndBinary};
pub(crate) use custom_host::{
    build_custom_host, host_flamegraph_command, host_sources_available,
    host_sources_install_command, host_sources_spec, HOST_SOURCES_PACKAGE,
};
pub(crate) use discovery::{
    detect_frontend, load_package_json, normalize_author, resolve_package_dir, BuildTool,
    CommandPlan, FrontendInfo, PackageJson,
};
pub(crate) use host_cache::{
    host_cache_dir, long_host_cache_warning, remove_host_cache, CACHE_DIR_VARIABLE,
};
pub(crate) use platform::{
    current_architecture, current_platform, find_host_binary, resolve_host_binary, Architecture,
};
pub(crate) use rust_toolchain::{ensure_rust_toolchain, HOST_MSRV, RUST_INSTALL_URL};
