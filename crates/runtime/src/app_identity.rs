//! Boot-time app identifier (`bundle.identifier`), which names the app's own
//! directories (`tynd_host::app_dirs`).
//!
//! Sources, by build kind:
//! - packaged app: the `app.identifier` pack entry only, fixed at build time
//!   (the environment is ignored so a user cannot point an app at another
//!   app's data);
//! - dev (`tynd dev` / `tynd start`, no pack): the `TYND_APP_IDENTIFIER` env
//!   var set by the CLI from the project config.
//!
//! Absent or invalid means the executable-name fallback. Never blocks boot.

/// Dev-mode env var carrying the identifier.
pub(crate) const ENV_VAR: &str = "TYND_APP_IDENTIFIER";

/// Pick the identifier for this build kind; blank counts as absent.
fn select_source(packed: Option<String>, env: Option<String>, packaged: bool) -> Option<String> {
    if packaged { packed } else { env }.filter(|identifier| !identifier.trim().is_empty())
}

/// Fix the app identifier from `packed` (the embedded `app.identifier`
/// entry) or the environment, according to `packaged`.
pub(crate) fn init(packed: Option<String>, packaged: bool) {
    let env = std::env::var(ENV_VAR).ok();
    let Some(identifier) = select_source(packed, env, packaged) else {
        return;
    };
    if let Err(error) = tynd_host::app_dirs::set_identifier(identifier.trim()) {
        tynd_host::tynd_error!("{error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_ignores_env_and_dev_ignores_pack() {
        let packed = Some("com.example.packed".to_string());
        let env = Some("com.example.env".to_string());
        assert_eq!(
            select_source(packed.clone(), env.clone(), true).as_deref(),
            Some("com.example.packed")
        );
        assert_eq!(
            select_source(packed, env, false).as_deref(),
            Some("com.example.env")
        );
        assert_eq!(
            select_source(None, Some("com.example.env".into()), true),
            None
        );
    }

    #[test]
    fn blank_counts_as_absent() {
        assert_eq!(select_source(Some("  ".into()), None, true), None);
        assert_eq!(select_source(None, Some(String::new()), false), None);
    }
}
