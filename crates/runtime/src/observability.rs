//! Boot-time observability wiring (Sentry crash reporting + OTel traces).
//!
//! The app's `observability` config (`tynd.config.ts`) reaches the runtime as
//! a small JSON object, camelCase:
//!
//! ```json
//! { "sentry": { "dsn": "…", "environment": "…", "release": "…", "sampleRate": 1 },
//!   "otel":   { "endpoint": "…", "serviceName": "…" } }
//! ```
//!
//! Sources, by build kind:
//! - packaged app: the `observability.json` pack entry only (the environment
//!   is ignored so a user cannot redirect an app's telemetry);
//! - dev (`tynd dev`, no pack): the `TYND_OBSERVABILITY` env var.
//!
//! Absent or invalid config means disabled. Invalid config never blocks boot.

use tynd_host::observability::{self, ObservabilityConfig};

/// Pack entry carrying the JSON object above.
pub(crate) const PACK_ENTRY: &str = "observability.json";
/// Dev-mode env var carrying the same JSON object.
pub(crate) const ENV_VAR: &str = "TYND_OBSERVABILITY";

/// Pick the raw config text for this build kind.
fn select_source(packed: Option<String>, env: Option<String>, packaged: bool) -> Option<String> {
    if packaged {
        packed
    } else {
        env
    }
}

/// Parse raw config text; absent, blank or invalid input yields the disabled
/// config. Errors are logged without echoing values (they may hold a DSN).
fn parse(raw: Option<&str>) -> ObservabilityConfig {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return ObservabilityConfig::default();
    };
    match ObservabilityConfig::from_app_json(raw) {
        Ok(config) => config,
        Err(error) => {
            tynd_host::tynd_error!(
                "Ignoring invalid observability config ({:?} at line {} column {}); \
                 observability stays disabled",
                error.classify(),
                error.line(),
                error.column()
            );
            ObservabilityConfig::default()
        },
    }
}

/// Start observability if configured. `packed` is the embedded
/// `observability.json` entry (packaged builds); `packaged` selects the source.
/// Returns `true` when at least one pipeline started. No-op when disabled.
pub(crate) fn init(packed: Option<String>, packaged: bool) -> bool {
    let env = std::env::var(ENV_VAR).ok();
    init_with(parse(select_source(packed, env, packaged).as_deref()))
}

fn init_with(config: ObservabilityConfig) -> bool {
    if config.is_empty() {
        return false;
    }
    match observability::init_global(config) {
        Ok(started) => started,
        Err(error) => {
            tynd_host::tynd_error!("Observability init failed; continuing without it: {error}");
            false
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_ignores_env_and_dev_ignores_pack() {
        let packed = Some("P".to_string());
        let env = Some("E".to_string());
        assert_eq!(
            select_source(packed.clone(), env.clone(), true).as_deref(),
            Some("P")
        );
        assert_eq!(select_source(packed, env, false).as_deref(), Some("E"));
        assert_eq!(select_source(None, Some("E".into()), true), None);
    }

    #[test]
    fn absent_blank_and_invalid_are_disabled() {
        assert!(parse(None).is_empty());
        assert!(parse(Some("  \n")).is_empty());
        assert!(parse(Some("{broken")).is_empty());
        assert!(parse(Some(r#"{"sentry":{"dsn":1}}"#)).is_empty());
    }

    #[test]
    fn valid_config_is_parsed() {
        let config = parse(Some(
            r#"{"sentry":{"dsn":"https://k@o.example/1","environment":"prod"}}"#,
        ));
        assert_eq!(config.sentry.unwrap().environment.as_deref(), Some("prod"));
        assert!(config.otel.is_none());
    }

    #[test]
    fn init_is_noop_when_disabled() {
        assert!(!init_with(ObservabilityConfig::default()));
        assert!(!init(None, true));
        assert!(!init(Some("{}".into()), true));
        assert!(!init(Some("null".into()), true));
    }
}
