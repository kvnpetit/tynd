---
title: Observability
description: "Production observability — structured logs, Sentry crash reporting, and OpenTelemetry traces. Opt-in through tynd.config.ts, zero-cost when off."
---

# Observability

Tynd ships an always-on structured logger and two **opt-in** pipelines: Sentry panic reporting and OpenTelemetry (OTLP/gRPC) trace export. Both are off by default; a build that does not use them contains no telemetry code and makes no network calls.

Observability is switched on in two places, and both must agree:

1. **Config** — the `observability` block in `tynd.config.ts`.
2. **Build** — the Cargo feature `obs-sentry` and/or `obs-otel` on the host.

`tynd build` and `tynd dev` enable the features for you from the config when they compile the host. A config without `observability` compiles neither feature, so the binary size is unchanged.

## What's wired by default

Every Tynd build installs a `tracing` stderr layer at startup. You get:

- **Structured spans** for IPC calls, plugin dispatch, and scheme handlers.
- **Per-target filtering** via `TYND_LOG` (or `RUST_LOG` as fallback).
- **Panic capture** routed to `target = "tynd::panic"` with a backtrace, even when `RUST_BACKTRACE` is unset.

```bash
# Debug logs from the host, info+ from your app:
TYND_LOG="tynd_host=debug,my_app=info" my-app
```

## Configure

```ts filename="tynd.config.ts"
import { defineConfig } from "@tynd/cli";

export default defineConfig({
  backend: "backend/main.ts",
  observability: {
    sentry: {
      dsn: "https://<key>@o0.ingest.sentry.io/<project>",
      environment: "production",
      release: "1.4.0",
      sampleRate: 1,
    },
    otel: {
      endpoint: "https://otlp.example.com:4317",
      serviceName: "my-app",
    },
  },
});
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `sentry.dsn` | `string` | — | Required inside `sentry`. A blank value disables Sentry |
| `sentry.environment` | `string` | — | Environment tag |
| `sentry.release` | `string` | — | Release tag |
| `sentry.sampleRate` | `number` | `1` | Between 0 and 1. Validation rejects values outside the range |
| `otel.endpoint` | `string` | — | Required inside `otel`. OTLP/gRPC collector URL. A blank value disables OTel |
| `otel.serviceName` | `string` | executable name | `service.name` resource attribute |

An absent `observability` block, or an absent `sentry` / `otel` object, means that pipeline is disabled with no initialization cost. The serialized block must stay under 4096 bytes. `tynd validate` reports invalid values.

## How the config reaches the app

| Run mode | Source |
|---|---|
| `tynd build` (packaged app) | The config is written into the pack as `observability.json`; the entry is omitted when the field is absent. **Packaged apps ignore the environment variable on purpose**, so a user cannot redirect an app's telemetry |
| `tynd dev` | `tynd` exports `TYND_OBSERVABILITY=<same JSON>` to the runtime. It is read only when there is no pack |

Initialization happens in `tynd_runtime::run()`, before the Bun backend is spawned and before the event loop starts. Invalid JSON is logged (without echoing values, since they may hold a DSN) and ignored: observability stays disabled and the app still boots.

## Enable at build

When the config sets `sentry` or `otel`, `tynd` compiles the host with the matching feature on `tynd-host`:

| Config | Cargo feature |
|---|---|
| `observability.sentry` | `obs-sentry` |
| `observability.otel` | `obs-otel` |

A host compiled without the requested feature (for example by hand, outside `tynd`) logs a warning for Sentry or an error for OTel at runtime, then continues without that pipeline. Both features increase the release artifact; measure the signed output per target.

## Sentry

Sentry reports **Rust panics** in the host. The Sentry panic integration captures them, a chained hook waits up to 2 seconds to flush, and the uncaught-panic path in `run()` flushes again. `tracing` events from the host become breadcrumbs.

Privacy defaults:

- `send_default_pii` is always `false`; the config has no way to turn it on.
- Breadcrumbs and messages are scrubbed: emails become `[email]`, `Bearer <token>` becomes `Bearer [token]`, and long messages are truncated (about 512 characters).

<Callout type="warning">
  **The scrubber is defense in depth, not a sanitizer.** Every `tracing::info!` / `warn!` / `error!` from your app and its dependencies can become a breadcrumb. Log structured, bounded fields; never raw user input, request bodies, secrets, session cookies, or URLs with credentials in the query string. Add scrubbing rules in your Sentry project settings as well.
</Callout>

What Sentry does **not** cover: native crashes (no minidump or signal capture, only Rust panics), and frontend or Bun-side errors. Report those from your own code if you need them.

## OpenTelemetry

Spans from `tracing` are exported over OTLP/gRPC to a collector you control (Honeycomb, Grafana Tempo, Datadog, Jaeger, a self-hosted collector). The exporter runs on a private single-thread Tokio runtime, so it does not depend on anything in your app. OTel resource attributes carry `service.name` plus whatever fields your spans add.

Add spans in Rust with `#[tracing::instrument]` or `tracing::info_span!`; they flow to the exporter once initialized.

If `otel` is configured but the host was built without `obs-otel`, the host logs an error and continues without traces.

## Behavior matrix

| `sentry` config | `otel` config | `obs-sentry` | `obs-otel` | Result |
|---|---|---|---|---|
| absent | absent | any | any | no-op, no network |
| set | absent | on | any | Sentry installed |
| set | absent | **off** | any | warning logged, Sentry skipped |
| absent | set | any | on | OTel installed |
| absent | set | any | **off** | error logged, OTel skipped |
| set | set | on | on | both installed |

## Profiling

Tynd does not bundle profilers. For local profiling use the platform profiler on the built host (Instruments on macOS, Windows Performance Recorder/Analyzer on Windows, `heaptrack` or Valgrind on Linux), or `tynd profile` for CPU, Bun-memory, and IPC sessions. For continuous production timing, the OTel pipeline gives span-level latency for every IPC call.

## Recommended backends

| Signal | Self-hosted | Managed |
|---|---|---|
| Traces | Jaeger, Grafana Tempo, OpenTelemetry Collector | Honeycomb, Lightstep, Datadog APM |
| Crashes | GlitchTip (Sentry-compatible) | Sentry |

Any OTLP/gRPC-compatible collector works for OTel; only `endpoint` changes. The Sentry `dsn` identifies the destination project.

## Related

- [`tynd.config.ts`](https://tynd-apps.github.io/tynd/docs/v1.0/cli/config#observability)
- [Debugging guide](https://tynd-apps.github.io/tynd/docs/v1.0/guides/debugging)
- [Production checklist](https://tynd-apps.github.io/tynd/docs/v1.0/production-checklist)
