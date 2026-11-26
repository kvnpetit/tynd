//! Production observability — OTel traces export + Sentry crash reporting.
//!
//! Strictly opt-in: compile-time via `obs-otel` / `obs-sentry` features, AND
//! runtime via non-`None` fields in [`ObservabilityConfig`]. Default builds
//! ship neither.
//!
//! `tynd_runtime::run()` wires this up automatically: it reads the app's
//! `observability` config ([`ObservabilityConfig::from_app_json`]) and calls
//! [`init_global`], which keeps the Sentry guard alive for the process lifetime
//! and chains a panic hook that flushes pending events. Embedders without
//! `tynd_runtime` can call [`init`] themselves AFTER [`crate::init_tracing`]
//! (which installs the global dispatcher) and BEFORE `app.start(...)`, holding
//! the returned guard for the process lifetime — its `Drop` flushes events.
//!
//! Privacy: Sentry's `send_default_pii` defaults to `false`; OTel only sends
//! `service.name` plus user-supplied span fields.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Top-level observability config. All-`None` is the "off" state.
#[derive(Serialize, Deserialize, Clone, Debug, Default, bon::Builder)]
pub struct ObservabilityConfig {
    /// OpenTelemetry traces export. Requires the `obs-otel` feature.
    pub otel: Option<OtelConfig>,
    /// Sentry crash reporter. Requires the `obs-sentry` feature.
    pub sentry: Option<SentryConfig>,
}

/// OTLP/gRPC exporter config.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OtelConfig {
    /// OTLP/gRPC endpoint, e.g. `https://otlp.honeycomb.io:443`.
    pub endpoint: String,
    /// `service.name` resource attribute.
    pub service_name: String,
    /// Head-based sample rate in `[0.0, 1.0]`. Defaults to `1.0`.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
}

const fn default_sample_rate() -> f64 {
    1.0
}

/// Sentry SDK config.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SentryConfig {
    /// Project DSN, e.g. `https://<key>@sentry.io/<project_id>`.
    pub dsn: String,
    /// Environment tag (`"production"` / `"staging"` / `"dev"`).
    pub environment: Option<String>,
    /// Release tag (typically version or git SHA).
    pub release: Option<String>,
    /// Send the default PII bundle (user IP, username, cookies). Default false.
    #[serde(default = "default_no_pii")]
    pub send_default_pii: bool,
    /// Error sample rate in `[0.0, 1.0]`. Default `1.0`.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
}

const fn default_no_pii() -> bool {
    false
}

/// Service name used when the app config does not set `otel.serviceName`.
const DEFAULT_SERVICE_NAME: &str = "tynd-app";

/// App-facing (`tynd.config.ts`) shape, camelCase:
/// `{ sentry?: { dsn, environment?, release?, sampleRate? },
///    otel?: { endpoint, serviceName? } }`.
/// Unknown keys are ignored so newer configs still load on older hosts.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AppObservability {
    sentry: Option<AppSentry>,
    otel: Option<AppOtel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSentry {
    dsn: String,
    environment: Option<String>,
    release: Option<String>,
    sample_rate: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppOtel {
    endpoint: String,
    service_name: Option<String>,
}

/// Clamp a user-supplied rate into `[0.0, 1.0]`; non-finite values fall back
/// to the default (`1.0`).
fn sanitize_rate(rate: Option<f64>) -> f64 {
    match rate {
        Some(r) if r.is_finite() => r.clamp(0.0, 1.0),
        _ => default_sample_rate(),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn default_service_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string())
}

impl ObservabilityConfig {
    /// `true` when neither Sentry nor OTel is configured (the "off" state).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.otel.is_none() && self.sentry.is_none()
    }

    /// Parse the app's `observability` config object (camelCase JSON).
    ///
    /// An absent/`null` section, or an empty `dsn` / `endpoint`, yields the
    /// disabled state for that pipeline. Sample rates are clamped to `[0, 1]`.
    ///
    /// # Errors
    /// Returns the JSON error when `raw` is not valid JSON or a field has the
    /// wrong type; callers should treat that as "disabled".
    pub fn from_app_json(raw: &str) -> Result<Self, serde_json::Error> {
        let app: Option<AppObservability> = serde_json::from_str(raw)?;
        let app = app.unwrap_or_default();

        let sentry = app.sentry.and_then(|s| {
            Some(SentryConfig {
                dsn: non_empty(Some(s.dsn))?,
                environment: non_empty(s.environment),
                release: non_empty(s.release),
                send_default_pii: false,
                sample_rate: sanitize_rate(s.sample_rate),
            })
        });
        let otel = app.otel.and_then(|o| {
            Some(OtelConfig {
                endpoint: non_empty(Some(o.endpoint))?,
                service_name: non_empty(o.service_name).unwrap_or_else(default_service_name),
                sample_rate: default_sample_rate(),
            })
        });
        Ok(Self { otel, sentry })
    }
}

/// Boxed OTel layer, attached to the process-wide subscriber after startup.
#[cfg(feature = "obs-otel")]
type OtelLayer = Box<dyn tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync>;
#[cfg(feature = "obs-otel")]
type OtelSlot = tracing_subscriber::reload::Handle<Option<OtelLayer>, tracing_subscriber::Registry>;

/// Handle to the empty OTel slot installed by [`install_tracing_subscriber`].
#[cfg(feature = "obs-otel")]
static OTEL_SLOT: std::sync::OnceLock<OtelSlot> = std::sync::OnceLock::new();

#[cfg(feature = "obs-otel")]
fn env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_env("TYND_LOG")
        .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
}

/// Fill the OTel slot. Fails when the subscriber that owns the slot is gone,
/// so callers never report OTel as started without an attached layer.
#[cfg(feature = "obs-otel")]
fn attach_otel_layer(slot: &OtelSlot, layer: OtelLayer) -> anyhow::Result<()> {
    slot.reload(Some(layer))
        .map_err(|error| anyhow::anyhow!("cannot attach the OpenTelemetry layer: {error}"))
}

/// Install the process-wide `tracing` subscriber used by `init_tracing`.
///
/// With `obs-otel` the registry carries an empty reloadable slot that
/// [`init_otel`] fills later, since the observability config is only known
/// after the subscriber is already installed. Idempotent against an already
/// installed subscriber (`try_init` is a no-op then).
pub(crate) fn install_tracing_subscriber(filter: tracing_subscriber::EnvFilter) {
    #[cfg(feature = "obs-otel")]
    {
        use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt};
        let (slot_layer, handle) = reload::Layer::new(None::<OtelLayer>);
        let installed = tracing_subscriber::registry()
            .with(slot_layer)
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_writer(std::io::stderr)
                    .compact(),
            )
            .try_init()
            .is_ok();
        if installed {
            let _ = OTEL_SLOT.set(handle);
        }
    }
    #[cfg(not(feature = "obs-otel"))]
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_writer(std::io::stderr)
            .compact()
            .try_init();
    }
}

/// Initialize the OpenTelemetry pipeline against `config.endpoint`.
/// Requires a Tokio runtime context (OTLP gRPC exporter spawns background
/// tasks via `opentelemetry_sdk::runtime::Tokio`).
///
/// # Errors
/// Returns `Err` if the OTLP exporter fails to build or registering the
/// global tracing dispatcher fails.
#[cfg(feature = "obs-otel")]
pub(crate) fn init_otel(config: &OtelConfig) -> anyhow::Result<()> {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};
    use opentelemetry_sdk::{
        trace::{Sampler, SdkTracerProvider},
        Resource,
    };
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(Sampler::TraceIdRatioBased(config.sample_rate))
        .with_resource(
            Resource::builder()
                .with_service_name(config.service_name.clone())
                .build(),
        )
        .build();

    let tracer = provider.tracer("tynd");
    let layer: OtelLayer = Box::new(tracing_opentelemetry::layer().with_tracer(tracer));

    // The layer must really be attached, otherwise no span is ever exported
    // while the caller believes OTel is running. `init_tracing` installs a
    // reloadable slot for it; without that slot (an embedder skipped
    // `init_tracing`) install a fresh subscriber, and fail if one is
    // already set.
    if let Some(handle) = OTEL_SLOT.get() {
        attach_otel_layer(handle, layer)?;
    } else {
        use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
        tracing_subscriber::registry()
            .with(layer)
            .with(env_filter())
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .try_init()
            .map_err(|error| {
                anyhow::anyhow!(
                    "cannot attach the OpenTelemetry layer: a tracing subscriber is already \
                     installed without an OTel slot ({error})"
                )
            })?;
    }

    // Register globally so flush works on shutdown.
    opentelemetry::global::set_tracer_provider(provider);

    Ok(())
}

/// Stub when `obs-otel` is not compiled in.
#[cfg(not(feature = "obs-otel"))]
pub(crate) fn init_otel(_config: &OtelConfig) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "OpenTelemetry support not compiled in. Rebuild with --features obs-otel"
    ))
}

/// Initialize the Sentry SDK. The returned `ClientInitGuard` MUST be held
/// for the lifetime of the process — its `Drop` flushes pending events.
/// Returns `None` when the DSN fails to parse — telemetry is then OFF
/// rather than silently swallowed.
#[cfg(feature = "obs-sentry")]
pub(crate) fn init_sentry(config: &SentryConfig) -> Option<sentry::ClientInitGuard> {
    let dsn = match config.dsn.parse::<sentry::types::Dsn>() {
        Ok(d) => d,
        Err(e) => {
            // Sentry DSNs may carry a secret-key segment in the legacy
            // format — skip logging the raw value, the parse error names
            // the malformed component without it.
            tracing::error!(
                target: "tynd::observability",
                error = %e,
                "Sentry DSN parse failed; crash reporting disabled"
            );
            return None;
        },
    };
    // `ClientOptions` is `#[non_exhaustive]` (no struct literal); the setters
    // panic on a rate outside 0..=1, so clamp what a caller may have put in
    // the public config.
    let sample_rate = if config.sample_rate.is_finite() {
        config.sample_rate.clamp(0.0, 1.0) as f32
    } else {
        1.0
    };
    let mut options = sentry::ClientOptions::new()
        .sample_rate(sample_rate)
        .traces_sample_rate(0.1)
        .send_default_pii(config.send_default_pii)
        .attach_stacktrace(true)
        .before_breadcrumb(scrub_breadcrumb)
        .before_send(scrub_event);
    options.dsn = Some(dsn);
    options.environment = config.environment.clone().map(Into::into);
    options.release = config.release.clone().map(Into::into);
    Some(sentry::init(options))
}

#[cfg(feature = "obs-sentry")]
fn scrub_event(
    mut event: sentry::protocol::Event<'static>,
) -> Option<sentry::protocol::Event<'static>> {
    if let Some(message) = event.message.as_mut() {
        *message = scrub_pii(message);
    }
    for exception in &mut event.exception.values {
        exception.value = exception.value.as_deref().map(scrub_pii);
    }
    Some(event)
}

/// Scrub `tracing`-derived breadcrumbs before they reach Sentry.
///
/// `tracing` events flow into Sentry breadcrumbs verbatim, so any log line
/// from the embedding app's code can leak user input. This best-effort
/// scrubber drops empty crumbs, masks obvious PII (emails, bearer tokens),
/// and truncates oversized messages. Apps that log untrusted input should
/// still avoid doing so — see `docs/concepts/observability.mdx`.
#[cfg(feature = "obs-sentry")]
fn scrub_breadcrumb(mut crumb: sentry::Breadcrumb) -> Option<sentry::Breadcrumb> {
    const MAX_LEN: usize = 512;
    const TRUNCATED_SUFFIX: &str = "[...truncated]";
    let Some(message) = crumb.message.as_ref() else {
        return Some(crumb);
    };
    if message.trim().is_empty() {
        return None;
    }
    let mut scrubbed = scrub_pii(message);
    if scrubbed.len() > MAX_LEN {
        scrubbed.truncate(MAX_LEN);
        scrubbed.push_str(TRUNCATED_SUFFIX);
    }
    crumb.message = Some(scrubbed);
    Some(crumb)
}

/// Mask likely-PII tokens (emails, `Bearer …` headers) inside `input`.
///
/// Pure-stdlib scanner — pulling `regex` just for two patterns is not
/// worth the build-time cost in a hot crash-reporting path. Returns the
/// original string unchanged when nothing matches.
#[cfg(feature = "obs-sentry")]
fn scrub_pii(input: &str) -> String {
    const EMAIL_MASK: &str = "[email]";
    const TOKEN_MASK: &str = "Bearer [token]";

    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Bearer <token>
        if bytes[i..].len() >= 7
            && bytes[i..i + 6].eq_ignore_ascii_case(b"Bearer")
            && matches!(bytes[i + 6], b' ' | b'\t')
        {
            let mut j = i + 7;
            while j < bytes.len() && is_token_char(bytes[j]) {
                j += 1;
            }
            if j > i + 7 {
                out.push_str(TOKEN_MASK);
                i = j;
                continue;
            }
        }

        // user@host
        if is_email_local_char(bytes[i]) {
            let local_start = i;
            let mut j = i;
            while j < bytes.len() && is_email_local_char(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'@' && j > local_start {
                let at = j;
                j += 1;
                let domain_start = j;
                while j < bytes.len() && is_email_domain_char(bytes[j]) {
                    j += 1;
                }
                // Require at least one `.` in the domain to avoid masking
                // `user@local` style debug tags that aren't real addresses.
                if j > domain_start && input[domain_start..j].contains('.') {
                    out.push_str(EMAIL_MASK);
                    i = j;
                    let _ = at;
                    continue;
                }
            }
        }

        // Push one UTF-8 char without splitting it.
        let ch = input[i..].chars().next().expect("non-empty by loop guard");
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

#[cfg(feature = "obs-sentry")]
const fn is_email_local_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}

#[cfg(feature = "obs-sentry")]
const fn is_email_domain_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')
}

#[cfg(feature = "obs-sentry")]
const fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

/// Init both layers based on `config`. Call from `main` BEFORE `app.start`.
/// Bind the returned guard to a `let _g =` so `Drop` flushes pending events.
///
/// | `otel` | `sentry` | Features        | Result                    |
/// |--------|----------|-----------------|---------------------------|
/// | None   | None     | any             | `Ok(None)` — no-op        |
/// | Some   | None     | `obs-otel`      | OTel installed, `Ok(None)`|
/// | Some   | None     | (missing)       | `Err(...)`                |
/// | None   | Some     | `obs-sentry`    | `Ok(Some(guard))`         |
/// | None   | Some     | (missing)       | `Ok(None)` — silent       |
/// | Some   | Some     | both            | both, `Ok(Some(guard))`   |
///
/// # Errors
/// Propagates [`init_otel`] failures. Sentry init failures never error.
pub(crate) fn init(config: ObservabilityConfig) -> anyhow::Result<Option<ObservabilityGuard>> {
    // Destructure so `sentry` is bound regardless of features compiled in.
    let ObservabilityConfig { otel, sentry } = config;

    if let Some(otel_cfg) = otel.as_ref() {
        init_otel(otel_cfg)?;
    }

    #[cfg(feature = "obs-sentry")]
    {
        if let Some(sentry_cfg) = sentry.as_ref() {
            if let Some(guard) = init_sentry(sentry_cfg) {
                return Ok(Some(ObservabilityGuard {
                    _sentry: Some(guard),
                }));
            }
            return Ok(None);
        }
    }

    // Explicit drop silences the unused-binding lint when `obs-sentry` is off.
    drop(sentry);
    Ok(None)
}

/// RAII guard from [`init`]. Owns the Sentry client guard so `Drop` flushes.
/// Hold for the lifetime of `main`; dropping early disables crash reporting.
pub(crate) struct ObservabilityGuard {
    #[cfg(feature = "obs-sentry")]
    _sentry: Option<sentry::ClientInitGuard>,
    #[cfg(not(feature = "obs-sentry"))]
    _empty: (),
}

impl std::fmt::Debug for ObservabilityGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservabilityGuard").finish()
    }
}

/// Process-lifetime home for the Sentry guard (`process::exit` skips `Drop`,
/// so a stack-held guard would never flush anyway).
static GLOBAL_GUARD: parking_lot::Mutex<Option<ObservabilityGuard>> = parking_lot::Mutex::new(None);
static GLOBAL_INIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Dedicated Tokio runtime for the OTLP batch exporter. The host has no
/// ambient runtime and the exporter must be built inside one.
#[cfg(feature = "obs-otel")]
static OTEL_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// Initialize observability for the whole process; the entry point used by
/// `tynd_runtime::run()`.
///
/// - Empty config: returns `Ok(false)` without touching any global state.
/// - Otherwise runs [`init`] (inside a private Tokio runtime for OTel), parks
///   the Sentry guard in a process-wide static, and chains a panic hook that
///   flushes pending Sentry events (Sentry's own panic integration, enabled by
///   `sentry::init`, does the capture; the previous Tynd log hook still runs).
/// - Idempotent: later calls return `Ok(false)`.
///
/// Returns `Ok(true)` when at least one pipeline was started.
///
/// # Errors
/// Propagates [`init`] failures (OTel exporter build, or OTel requested while
/// `obs-otel` is not compiled in). Callers should log and continue.
pub fn init_global(config: ObservabilityConfig) -> anyhow::Result<bool> {
    use std::sync::atomic::Ordering;

    if config.is_empty() {
        return Ok(false);
    }
    if GLOBAL_INIT.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }

    #[cfg(not(feature = "obs-sentry"))]
    if config.sentry.is_some() {
        tracing::warn!(
            target: "tynd::observability",
            "observability.sentry is configured but this build lacks the obs-sentry feature; \
             crash reporting is disabled"
        );
    }

    #[cfg(feature = "obs-otel")]
    let _otel_enter = if config.otel.is_some() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("tynd-otel")
            .enable_all()
            .build()?;
        Some(OTEL_RUNTIME.get_or_init(|| rt).enter())
    } else {
        None
    };

    // `init` errors when OTel is requested without `obs-otel`, so `Ok` means
    // the OTel pipeline (if configured) is running.
    let otel_requested = config.otel.is_some();
    let guard = init(config)?;
    let sentry_active = guard.is_some();
    if sentry_active {
        install_flush_panic_hook();
        *GLOBAL_GUARD.lock() = guard;
    }
    Ok(sentry_active || otel_requested)
}

/// Chain a hook that flushes the Sentry transport after the previous hook
/// (Sentry capture, then the Tynd structured log) has run. Without this a
/// crash racing `process::abort`/unwind would drop the queued event.
fn install_flush_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        flush(Duration::from_secs(2));
    }));
}

/// Block up to `timeout` for pending Sentry events to be delivered. No-op
/// when Sentry is not compiled in or not initialized.
pub fn flush(timeout: Duration) {
    #[cfg(feature = "obs-sentry")]
    if let Some(client) = sentry::Hub::current().client() {
        client.flush(Some(timeout));
    }
    #[cfg(not(feature = "obs-sentry"))]
    let _ = timeout;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_or_null_is_disabled() {
        for raw in ["null", "{}", r#"{"sentry":null,"otel":null}"#] {
            let cfg = ObservabilityConfig::from_app_json(raw).unwrap();
            assert!(cfg.is_empty(), "{raw}");
        }
    }

    #[test]
    fn parses_camel_case_fields() {
        let cfg = ObservabilityConfig::from_app_json(
            r#"{"sentry":{"dsn":"https://k@o.example/1","environment":"prod",
                "release":"1.2.3","sampleRate":0.25},
               "otel":{"endpoint":"https://otlp.example:443","serviceName":"my-app"}}"#,
        )
        .unwrap();
        let sentry = cfg.sentry.unwrap();
        assert_eq!(sentry.dsn, "https://k@o.example/1");
        assert_eq!(sentry.environment.as_deref(), Some("prod"));
        assert_eq!(sentry.release.as_deref(), Some("1.2.3"));
        assert!(!sentry.send_default_pii);
        assert!((sentry.sample_rate - 0.25).abs() < f64::EPSILON);
        let otel = cfg.otel.unwrap();
        assert_eq!(otel.endpoint, "https://otlp.example:443");
        assert_eq!(otel.service_name, "my-app");
        assert!((otel.sample_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn defaults_and_clamping() {
        let cfg = ObservabilityConfig::from_app_json(
            r#"{"sentry":{"dsn":"https://k@o.example/1","sampleRate":7},
               "otel":{"endpoint":"http://localhost:4317"},"future":1}"#,
        )
        .unwrap();
        let sentry = cfg.sentry.unwrap();
        assert!(sentry.environment.is_none() && sentry.release.is_none());
        assert!((sentry.sample_rate - 1.0).abs() < f64::EPSILON);
        assert_ne!(cfg.otel.unwrap().service_name, "");

        let low = ObservabilityConfig::from_app_json(
            r#"{"sentry":{"dsn":"https://k@o.example/1","sampleRate":-3}}"#,
        )
        .unwrap();
        assert!(low.sentry.unwrap().sample_rate.abs() < f64::EPSILON);
    }

    #[test]
    fn blank_dsn_or_endpoint_disables_that_pipeline() {
        let cfg =
            ObservabilityConfig::from_app_json(r#"{"sentry":{"dsn":"  "},"otel":{"endpoint":""}}"#)
                .unwrap();
        assert!(cfg.is_empty());
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(ObservabilityConfig::from_app_json("{not json").is_err());
        assert!(ObservabilityConfig::from_app_json(r#"{"sentry":{"dsn":5}}"#).is_err());
        assert!(ObservabilityConfig::from_app_json(r#"{"sentry":{}}"#).is_err());
    }

    #[test]
    fn init_is_noop_when_disabled() {
        assert!(!init_global(ObservabilityConfig::default()).unwrap());
        assert!(!GLOBAL_INIT.load(std::sync::atomic::Ordering::SeqCst));
        assert!(GLOBAL_GUARD.lock().is_none());
        assert!(init(ObservabilityConfig::default()).unwrap().is_none());
        flush(Duration::from_millis(1));
    }

    #[cfg(feature = "obs-sentry")]
    #[test]
    fn sentry_rejects_malformed_dsn() {
        let cfg = SentryConfig {
            dsn: "not a dsn".into(),
            environment: None,
            release: None,
            send_default_pii: false,
            sample_rate: 1.0,
        };
        assert!(init_sentry(&cfg).is_none());
    }

    #[cfg(feature = "obs-otel")]
    mod otel_slot {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        use tracing_subscriber::{layer::SubscriberExt, reload, Layer, Registry};

        use super::super::*;

        struct CountSpans(Arc<AtomicUsize>);

        impl Layer<Registry> for CountSpans {
            fn on_new_span(
                &self,
                _attrs: &tracing::span::Attributes<'_>,
                _id: &tracing::span::Id,
                _ctx: tracing_subscriber::layer::Context<'_, Registry>,
            ) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        #[test]
        fn attached_layer_receives_spans() {
            let seen = Arc::new(AtomicUsize::new(0));
            let (slot_layer, handle) = reload::Layer::new(None::<OtelLayer>);
            let subscriber = Registry::default().with(slot_layer);
            tracing::subscriber::with_default(subscriber, || {
                tracing::info_span!("before").in_scope(|| {});
                assert_eq!(seen.load(Ordering::SeqCst), 0);
                attach_otel_layer(&handle, Box::new(CountSpans(seen.clone()))).unwrap();
                tracing::info_span!("after").in_scope(|| {});
            });
            assert_eq!(seen.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn attach_fails_when_the_subscriber_is_gone() {
            let (slot_layer, handle) = reload::Layer::new(None::<OtelLayer>);
            drop(Registry::default().with(slot_layer));
            let layer: OtelLayer = Box::new(CountSpans(Arc::new(AtomicUsize::new(0))));
            assert!(attach_otel_layer(&handle, layer).is_err());
        }
    }
}
