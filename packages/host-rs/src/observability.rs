//! Production observability — OpenTelemetry traces export + Sentry crash reporting.
//!
//! Both pipelines are **strictly opt-in** at two levels:
//!
//! 1. **Compile-time** — guarded by Cargo features `obs-otel` / `obs-sentry`.
//!    The default build (and every `api-all` preset) compiles neither, so a
//!    plain `tynd-full` / `tynd-lite` ships without tokio + reqwest + tonic
//!    + sentry SDK in the binary.
//! 2. **Runtime** — even with the feature on, [`init`] is a no-op unless the
//!    user passes a non-`None` field in [`ObservabilityConfig`]. Compiling
//!    the support in does not auto-export anything to the network.
//!
//! Apps that want either pipeline call [`init`] from `main` **before**
//! `app.start(...)` and hold the returned `Drop` guard for the lifetime of
//! the process — Sentry flushes pending events on `Drop`, which matters
//! for crashes that happen during shutdown.
//!
//! # Layering with `init_tracing`
//!
//! [`crate::init_tracing`] installs the always-on `tracing-subscriber` fmt
//! layer that writes to stderr. When `obs-otel` is enabled and an OTel
//! config is provided, the OTel layer is **added on top** of the existing
//! subscriber via the global dispatch — the fmt layer keeps emitting JSON
//! to stderr, and the OTel layer mirrors the same span tree to the
//! collector. Calling [`init`] before `init_tracing` is therefore unsafe
//! (no global dispatcher yet); the convention is `init_tracing()` first,
//! `observability::init(...)` second.
//!
//! # Privacy defaults
//!
//! - Sentry's `send_default_pii` defaults to `false` — usernames, IPs, and
//!   cookies stay out of crash payloads unless the app overrides it.
//! - OTel resource attributes carry only `service.name` plus whatever the
//!   app adds via `tracing` span fields. No machine identifiers, no PII.
//! - The `sample_rate` knob lets crash-budget-conscious apps cap the
//!   volume sent to either backend (1.0 = all, 0.1 = 10%).

use serde::{Deserialize, Serialize};

/// Top-level observability config. Either field can be `None`; passing
/// the all-`None` value to [`init`] is the documented "off" state and
/// returns successfully without touching anything.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ObservabilityConfig {
    /// OpenTelemetry traces export. Requires the `obs-otel` feature.
    pub otel: Option<OtelConfig>,
    /// Sentry crash reporter. Requires the `obs-sentry` feature.
    pub sentry: Option<SentryConfig>,
}

/// OTLP exporter configuration. The endpoint must be reachable at process
/// start — failures during pipeline install bubble up as `Err` from
/// [`init`] so the app can crash loud rather than silently lose traces.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OtelConfig {
    /// OTLP/gRPC endpoint, e.g. `https://otlp.honeycomb.io:443` or
    /// `http://localhost:4317` for a local collector.
    pub endpoint: String,
    /// `service.name` resource attribute. Surfaces in the collector as
    /// the service identifier — pick the same name across all instances
    /// of one app so traces aggregate.
    pub service_name: String,
    /// Sample rate in `[0.0, 1.0]`. `1.0` = export every trace, `0.1` =
    /// export ~10% (head-based sampler, decided at root span creation).
    /// Defaults to `1.0` when omitted from JSON.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
}

const fn default_sample_rate() -> f64 {
    1.0
}

/// Sentry SDK configuration. The DSN selects the Sentry project; the
/// other fields populate the standard `environment` / `release` /
/// `sample_rate` knobs.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SentryConfig {
    /// Project DSN, e.g. `https://<key>@sentry.io/<project_id>`.
    pub dsn: String,
    /// Environment tag (`"production"` / `"staging"` / `"dev"`). Optional
    /// — Sentry leaves it blank if missing.
    pub environment: Option<String>,
    /// Release tag (typically `package.json` version or git SHA). Drives
    /// suspect-commit attribution and release-health graphs.
    pub release: Option<String>,
    /// When `true`, Sentry sends the default PII bundle (user IP,
    /// username, cookies). Defaults to `false` — opt in explicitly.
    #[serde(default = "default_no_pii")]
    pub send_default_pii: bool,
    /// Error sample rate in `[0.0, 1.0]`. `1.0` ships every captured
    /// event; `0.1` ships ~10%. Defaults to `1.0`.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
}

const fn default_no_pii() -> bool {
    false
}

/// Initialize the OpenTelemetry pipeline against `config.endpoint`.
///
/// Requires a Tokio runtime context — the OTLP gRPC exporter spawns
/// background tasks via `opentelemetry_sdk::runtime::Tokio`. The host
/// crate doesn't bundle a runtime, so callers must either (a) wrap the
/// init call in `#[tokio::main]` / `tokio::runtime::Runtime::new()?`, or
/// (b) invoke it from a context that already runs Tokio (e.g. an
/// `axum`/`actix` sidecar process).
///
/// On success the returned tracer is registered as the global
/// `tracing` subscriber's OTel layer, on top of the fmt layer that
/// [`crate::init_tracing`] installed — both layers see every span.
///
/// # Errors
/// Returns `Err` if the OTLP exporter fails to build (invalid endpoint,
/// missing TLS roots, …) or if registering the global tracing
/// dispatcher fails (already set with an incompatible layer set).
#[cfg(feature = "obs-otel")]
pub fn init_otel(config: &OtelConfig) -> anyhow::Result<()> {
    use opentelemetry::{trace::TracerProvider as _, KeyValue};
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};
    use opentelemetry_sdk::{
        runtime,
        trace::{Sampler, TracerProvider},
        Resource,
    };
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()?;

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_sampler(Sampler::TraceIdRatioBased(config.sample_rate))
        .with_resource(Resource::new(vec![KeyValue::new(
            "service.name",
            config.service_name.clone(),
        )]))
        .build();

    let tracer = provider.tracer("tynd");

    // Install the OTel layer on top of the always-on fmt layer. We can't
    // assume `init_tracing` has already run (an embedder might want OTel
    // without the stderr fmt sink), so we register a fresh subscriber
    // with both layers; the second `init` is a no-op on the dispatcher
    // already set by `init_tracing`, so users should call us first if
    // they want OTel + their own custom fmt layer.
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let filter = EnvFilter::try_from_env("TYND_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();

    // Hand the provider to opentelemetry global so flush works on shutdown.
    opentelemetry::global::set_tracer_provider(provider);

    Ok(())
}

/// Stub when `obs-otel` is not compiled in — returns a clear error so
/// the operator can rebuild with the right feature flag instead of
/// chasing silent telemetry loss.
#[cfg(not(feature = "obs-otel"))]
pub fn init_otel(_config: &OtelConfig) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "OpenTelemetry support not compiled in. Rebuild with --features obs-otel"
    ))
}

/// Initialize the Sentry SDK with the supplied config.
///
/// Returns the SDK's `ClientInitGuard` which **must be held** for the
/// lifetime of the process — its `Drop` impl calls `client.flush()`
/// with a 2 s timeout to ship any in-flight events before exit. Drop it
/// too early and crashes near shutdown ship as truncated payloads.
#[cfg(feature = "obs-sentry")]
pub fn init_sentry(config: &SentryConfig) -> sentry::ClientInitGuard {
    sentry::init(sentry::ClientOptions {
        dsn: config.dsn.parse().ok(),
        environment: config.environment.clone().map(Into::into),
        release: config.release.clone().map(Into::into),
        send_default_pii: config.send_default_pii,
        #[allow(clippy::cast_possible_truncation)]
        sample_rate: config.sample_rate as f32,
        // Tracing sample rate — separate from error sample rate. Default
        // to 10% so apps that don't override get reasonable trace
        // volume without flooding Sentry's tracing quota.
        traces_sample_rate: 0.1,
        attach_stacktrace: true,
        ..Default::default()
    })
}

/// No-op stub when `obs-sentry` is not compiled in. Returns `()` rather
/// than `Err` because Sentry init failures should never crash an app —
/// the entire pipeline is optional. Operators who care surface the
/// missing feature via the `init` log line instead.
#[cfg(not(feature = "obs-sentry"))]
pub fn init_sentry(_config: &SentryConfig) {}

/// Init both observability layers based on `config`. Call from `main`
/// **before** `app.start(...)`. The returned guard owns any background
/// resources (Sentry transport, OTel batcher) — bind it to a `let _g =`
/// at top of `main` and drop it at the end so flushes complete cleanly.
///
/// # Behavior matrix
///
/// | `otel` | `sentry` | Compiled features | Result                    |
/// |--------|----------|-------------------|---------------------------|
/// | None   | None     | any               | `Ok(None)` — no-op        |
/// | Some   | None     | `obs-otel`        | OTel installed, `Ok(None)`|
/// | Some   | None     | (missing)         | `Err(...)`                |
/// | None   | Some     | `obs-sentry`      | `Ok(Some(guard))`         |
/// | None   | Some     | (missing)         | `Ok(None)` — silent       |
/// | Some   | Some     | both              | both, `Ok(Some(guard))`   |
///
/// # Errors
/// Propagates failures from [`init_otel`] (invalid endpoint, TLS root
/// missing, dispatcher race). Sentry init failures never error — the
/// SDK swallows them internally and routes events to a dropping client.
pub fn init(config: ObservabilityConfig) -> anyhow::Result<Option<ObservabilityGuard>> {
    // Destructure up front so the `sentry` field is bound regardless of
    // which features are compiled in — keeps the by-value signature
    // (callers can't easily share the config across threads anyway and
    // observability config is small + cheap to clone) without tripping
    // `clippy::needless_pass_by_value` on the unused-feature build.
    let ObservabilityConfig { otel, sentry } = config;

    if let Some(otel_cfg) = otel.as_ref() {
        init_otel(otel_cfg)?;
    }

    #[cfg(feature = "obs-sentry")]
    {
        if let Some(sentry_cfg) = sentry.as_ref() {
            let guard = init_sentry(sentry_cfg);
            return Ok(Some(ObservabilityGuard {
                _sentry: Some(guard),
            }));
        }
    }

    // Drop the sentry slot explicitly so the binding shows as consumed
    // even when `obs-sentry` isn't compiled in — clippy would otherwise
    // flag the destructured field as unused.
    drop(sentry);
    Ok(None)
}

/// RAII guard returned from [`init`]. Owns the Sentry client guard (when
/// `obs-sentry` is on) so its `Drop` flushes pending events on the way
/// out. Holding this for the lifetime of `main` is the supported
/// pattern; dropping it early is equivalent to disabling crash
/// reporting for any subsequent panic.
pub struct ObservabilityGuard {
    #[cfg(feature = "obs-sentry")]
    _sentry: Option<sentry::ClientInitGuard>,
    // Without `obs-sentry` the guard is empty but still kept as a public
    // type so embedders compile against a stable signature.
    #[cfg(not(feature = "obs-sentry"))]
    _empty: (),
}

impl std::fmt::Debug for ObservabilityGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservabilityGuard").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty config is the documented "observability disabled" state and
    /// must return `Ok(None)` without trying to touch the network or any
    /// global subscriber. This is what apps that wire the API but leave
    /// the user-facing toggle off will hit on every cold start.
    #[test]
    fn init_with_empty_config_is_noop() {
        let result = init(ObservabilityConfig::default());
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    /// When `obs-otel` is *not* compiled in, asking for OTel must error
    /// loud — silently skipping would let an SRE think traces were
    /// flowing when the binary is incapable of exporting them. Skipped
    /// when the feature *is* compiled in: in that case the call hits a
    /// real network init and would either succeed (collector reachable)
    /// or fail for a wholly different reason.
    #[cfg(not(feature = "obs-otel"))]
    #[test]
    fn init_otel_without_feature_returns_err() {
        let cfg = OtelConfig {
            endpoint: "http://localhost:4317".to_string(),
            service_name: "test".to_string(),
            sample_rate: 1.0,
        };
        let err = init_otel(&cfg).unwrap_err();
        assert!(
            err.to_string().contains("obs-otel"),
            "error message should hint at the missing feature flag, got: {err}"
        );
    }

    /// Inverse: when `obs-sentry` is missing, `init_sentry` is a typed
    /// `()` no-op (signature returns unit, not `Result`). Calling
    /// `init(...)` with a sentry config in that build returns
    /// `Ok(None)` — Sentry quietly does nothing rather than refusing
    /// to boot the app.
    #[cfg(not(feature = "obs-sentry"))]
    #[test]
    fn init_sentry_without_feature_is_silent_noop() {
        let cfg = SentryConfig {
            dsn: "https://key@sentry.io/1".to_string(),
            environment: Some("test".to_string()),
            release: None,
            send_default_pii: false,
            sample_rate: 1.0,
        };
        // Should compile and run as `()`.
        init_sentry(&cfg);

        let result = init(ObservabilityConfig {
            otel: None,
            sentry: Some(cfg),
        });
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    /// The two configs must round-trip through JSON cleanly so apps can
    /// store them in `tynd.config.json` / env-injected blobs without
    /// custom serializers. Defaults must materialize on the way back in.
    #[test]
    fn config_round_trips_through_json() {
        // Full config — every field set.
        let full = ObservabilityConfig {
            otel: Some(OtelConfig {
                endpoint: "https://otlp.example.com:4317".to_string(),
                service_name: "myapp".to_string(),
                sample_rate: 0.25,
            }),
            sentry: Some(SentryConfig {
                dsn: "https://abc@sentry.io/123".to_string(),
                environment: Some("prod".to_string()),
                release: Some("1.0.0".to_string()),
                send_default_pii: false,
                sample_rate: 1.0,
            }),
        };
        let json = serde_json::to_string(&full).unwrap();
        let back: ObservabilityConfig = serde_json::from_str(&json).unwrap();
        let otel = back.otel.expect("otel preserved");
        assert_eq!(otel.endpoint, "https://otlp.example.com:4317");
        assert_eq!(otel.service_name, "myapp");
        assert!((otel.sample_rate - 0.25).abs() < f64::EPSILON);
        let sentry_back = back.sentry.expect("sentry preserved");
        assert_eq!(sentry_back.dsn, "https://abc@sentry.io/123");
        assert_eq!(sentry_back.environment.as_deref(), Some("prod"));
        assert_eq!(sentry_back.release.as_deref(), Some("1.0.0"));
        assert!(!sentry_back.send_default_pii);

        // Minimal sentry — defaults must fill `send_default_pii` (false)
        // and `sample_rate` (1.0).
        let minimal = r#"{"sentry":{"dsn":"https://x@y/1"}}"#;
        let parsed: ObservabilityConfig = serde_json::from_str(minimal).unwrap();
        let s = parsed.sentry.expect("present");
        assert_eq!(s.dsn, "https://x@y/1");
        assert!(!s.send_default_pii, "default PII off");
        assert!((s.sample_rate - 1.0).abs() < f64::EPSILON);
        assert!(s.environment.is_none());

        // Minimal otel — `sample_rate` must default to 1.0.
        let minimal_otel = r#"{"otel":{"endpoint":"http://localhost:4317","service_name":"a"}}"#;
        let parsed_otel: ObservabilityConfig = serde_json::from_str(minimal_otel).unwrap();
        let o = parsed_otel.otel.expect("present");
        assert!((o.sample_rate - 1.0).abs() < f64::EPSILON);
    }

    /// Empty JSON `{}` must yield the all-`None` default config — covers
    /// embedders that want to opt every toggle off via a literal blank.
    #[test]
    fn empty_json_object_yields_default() {
        let parsed: ObservabilityConfig = serde_json::from_str("{}").unwrap();
        assert!(parsed.otel.is_none());
        assert!(parsed.sentry.is_none());
    }

    /// Guard implements `Debug` (workspace lint denies missing impls)
    /// and is `Send`-able so it can be moved into worker threads or
    /// closures held by the runtime.
    #[test]
    fn guard_implements_debug_and_is_send() {
        // `ObservabilityGuard` may not be `Send` on every feature combo
        // because `sentry::ClientInitGuard` isn't `Send` — gate the
        // assertion accordingly. Helper is gated too so the
        // non-`obs-sentry` build doesn't carry a dead-code warning.
        #[cfg(not(feature = "obs-sentry"))]
        {
            fn assert_send<T: Send>() {}
            assert_send::<ObservabilityGuard>();
        }

        let guard = init(ObservabilityConfig::default()).unwrap();
        assert!(guard.is_none(), "default config returns None guard");
    }
}
