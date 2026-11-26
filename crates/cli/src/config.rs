mod edit;
mod environment;
mod extends;
mod load;
mod model;
mod resolve;
mod schema;
mod shared;
mod validate;

pub(crate) fn is_valid_permission_selector(selector: &str) -> bool {
    permission_api(selector).is_some()
}
pub(crate) use edit::{mutate_config_source, resolve_editable_config, ConfigMutation};
pub(crate) use load::{
    find_config_in_directory, find_config_path, find_project_root, load, load_typed,
};
pub(crate) use model::permission_api;
pub(crate) use model::*;
pub(crate) use resolve::{
    resolve_backend_plugins, resolve_effective_host_plan, resolve_observability,
    resolve_observability_json, resolve_plugins, resolve_updater_config, resolve_updater_network,
    updater_pubkey_pin, DEFAULT_UPDATER_CHANNEL,
};
