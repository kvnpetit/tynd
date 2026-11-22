use super::*;

fn is_sorted_unique(list: &[&str]) -> bool {
    list.windows(2).all(|pair| pair[0] < pair[1])
}

#[test]
fn catalog_lists_are_sorted_and_consistent() {
    assert!(is_sorted_unique(KNOWN_APIS));
    assert!(is_sorted_unique(CORE_HOST_APIS));
    assert!(is_sorted_unique(PLUGIN_FIRST_APIS));
    for api in CORE_HOST_APIS.iter().chain(PLUGIN_FIRST_APIS) {
        assert!(KNOWN_APIS.contains(api), "{api} must be a known API");
    }
    assert!(
        CORE_HOST_APIS
            .iter()
            .all(|api| !PLUGIN_FIRST_APIS.contains(api)),
        "an API is either preset or plugin-first"
    );
    assert_eq!(
        KNOWN_APIS.len(),
        CORE_HOST_APIS.len() + PLUGIN_FIRST_APIS.len()
    );
}

#[test]
fn plugin_packages_cover_every_plugin_first_api() {
    let apis: Vec<_> = API_PLUGIN_PACKAGES.iter().map(|(api, _)| *api).collect();
    assert_eq!(apis, PLUGIN_FIRST_APIS);
    assert!(API_PLUGIN_PACKAGES
        .iter()
        .all(|(api, package)| *package == format!("@tynd/plugin-{api}")));
}

#[test]
fn core_is_minimal_and_ui_apis_are_plugin_first() {
    assert_eq!(CORE_HOST_APIS, ["acl", "log"]);
    for api in ["menu", "tray", "multiwindow"] {
        assert!(KNOWN_APIS.contains(&api));
        assert!(PLUGIN_FIRST_APIS.contains(&api));
    }
    assert!(!KNOWN_APIS.contains(&"debug-screenshot"));
}

#[test]
fn permission_selectors_accept_new_apis() {
    assert_eq!(permission_api("menu:*"), Some("menu"));
    assert_eq!(permission_api("tray:setIcon"), Some("tray"));
    assert_eq!(permission_api("multiwindow:create"), Some("multiwindow"));
    assert_eq!(permission_api("network-public:fetch"), Some("network"));
    assert_eq!(permission_api("bogus:create"), None);
}
