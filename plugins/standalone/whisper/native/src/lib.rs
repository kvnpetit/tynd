//! Local speech-to-text via [`whisper-rs`] ([`whisper.cpp`]). Loads
//! GGML / GGUF Whisper checkpoints; models stay resident in a
//! [`Registry`] keyed by `u64`.
//!
//! Building whisper.cpp pulls a C++ toolchain (3–5 min cold). The real
//! backend lives behind the `inference` feature; without it every
//! dispatch arm returns a "rebuild with --features inference" hint.
//! Hardware-accelerated builds (`cuda`, `metal`, `vulkan`, `hipblas`,
//! `openblas`) layer on top.
//!
//! `transcribe` returns a `{ stream }` id; segments arrive as
//! `whisper:segment { stream, segment }` with a final `whisper:end`. The
//! buffer is decoded in one pass, so segments are emitted together once the
//! decode finishes; `cancel` aborts the decode itself.
//! The client wraps it into an `AsyncIterable<TranscriptSegment>`.
//!
//! Whisper expects 16 kHz mono f32 PCM. Samples cross the wire as
//! base64 little-endian f32.
//!
//! [`whisper-rs`]: https://crates.io/crates/whisper-rs
//! [`whisper.cpp`]: https://github.com/ggerganov/whisper.cpp

#![deny(unsafe_op_in_unsafe_fn)]

use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    tynd_plugin::{plugin, plugin_impl},
    GatedLimits, GatedStreams, PluginCx, PluginError, PluginResult, Registry,
    DEFAULT_SHUTDOWN_DEADLINE,
};

const MAX_MODELS: usize = 8;
/// Bounds on free-form option strings coming from the renderer.
const MAX_LANGUAGE_BYTES: usize = 16;
const MAX_PROMPT_BYTES: usize = 8 * 1024;

/// Conservative defaults: greedy decode, auto language, no translation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TranscribeOptions {
    /// `None` / `"auto"` / `""` ⇒ detect; else BCP-47 short code.
    #[serde(default)]
    pub language: Option<String>,
    /// Translate to English while transcribing.
    #[serde(default)]
    pub translate: bool,
    /// `0` ⇒ `available_parallelism()`; larger values are clamped to it.
    #[serde(default)]
    pub threads: u32,
    /// Initial prompt to bias vocabulary / formatting.
    #[serde(default)]
    pub prompt: Option<String>,
    /// `0.0` ⇒ greedy.
    #[serde(default)]
    pub temperature: f32,
    /// Max segment length in chars. `None` ⇒ whisper default.
    #[serde(default)]
    pub max_len: Option<u32>,
    /// Per-token timestamps + confidences.
    #[serde(default)]
    pub word_timestamps: bool,
}

/// One finalised transcript segment. Emitted once per `whisper:segment`
/// event during streaming `transcribe()`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    #[specta(type = f64)]
    pub start_ms: i64,
    #[specta(type = f64)]
    pub end_ms: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordTimestamp>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WordTimestamp {
    pub text: String,
    #[specta(type = f64)]
    pub start_ms: i64,
    #[specta(type = f64)]
    pub end_ms: i64,
    pub confidence: f32,
}

/// `id` rendered as JS `number`; `MAX_MODELS` keeps it well below
/// `Number.MAX_SAFE_INTEGER`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WhisperModel {
    #[specta(type = f64)]
    pub id: u64,
    pub path: String,
    /// Always `16000`.
    pub sample_rate: u32,
    /// `"multi"` for multilingual; BCP-47 short code for monolingual.
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeStream {
    #[specta(type = f64)]
    pub stream: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DetectLanguageResult {
    pub language: String,
    pub confidence: f32,
}

// Two `backend` impls picked at compile time via the `inference` feature.

mod backend;

#[plugin(name = "whisper")]
pub struct WhisperPlugin {
    /// Loaded models. Entries are `Arc`s so inference clones the handle out
    /// and runs with no registry lock held.
    models: Arc<Registry<Arc<backend::WhisperModel>>>,
    /// Bounded permits (at most `available_parallelism()` concurrent runs,
    /// timed acquisition), cancel flags and start gates of every stream.
    streams: Arc<GatedStreams>,
}

impl std::fmt::Debug for WhisperPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperPlugin").finish_non_exhaustive()
    }
}

impl Default for WhisperPlugin {
    fn default() -> Self {
        let max_parallel =
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self {
            models: Arc::new(Registry::new(MAX_MODELS)),
            streams: Arc::new(GatedStreams::new(
                max_parallel,
                "inference slots",
                GatedLimits::default(),
            )),
        }
    }
}

#[plugin_impl]
impl WhisperPlugin {
    #[command]
    pub fn load(
        &self,
        cx: &PluginCx,
        path: String,
        gpu: Option<bool>,
    ) -> PluginResult<WhisperModel> {
        let authorized_path = cx.authorize_fs_path(&path, "read")?;
        let gpu = gpu.unwrap_or(true);
        let model = backend::load(&PathBuf::from(&authorized_path), gpu)?;
        let id = self.models.insert(Arc::new(model))?;
        // Report the resolved path, the same one `list` returns.
        Ok(WhisperModel {
            id,
            path: authorized_path,
            sample_rate: 16_000,
            language: "multi".into(),
        })
    }

    /// Streaming transcription. Returns a `stream` id; segments arrive as
    /// `whisper:segment { stream, segment }` events with a final
    /// `whisper:end { stream, ok, error? }`. The stream must be released with
    /// `startStream`; otherwise it expires and ends with `ok: false`.
    #[command]
    pub fn transcribe(
        &self,
        cx: &PluginCx,
        model_id: u64,
        samples: String,
        sample_rate: Option<u32>,
        channels: Option<u32>,
        opts: Option<TranscribeOptions>,
    ) -> PluginResult<TranscribeStream> {
        validate_audio_format(sample_rate, channels)?;
        let opts = opts.unwrap_or_default();
        validate_options(&opts)?;
        let samples = decode_samples_str(&samples)?;
        let model = self
            .models
            .get_arc_arg(model_id, "modelId", "whisper.transcribe")?;

        let emit = cx.emitter();
        let segment_emit = Arc::clone(&emit);
        // Explicit event-subscription barrier: the worker emits nothing until
        // the frontend installed its listeners and called `startStream`.
        let stream = self.streams.spawn_stream(
            "whisper.transcribe",
            "tynd-whisper",
            emit,
            "whisper:end",
            move |stream_id, cancel| {
                backend::transcribe(&model, &samples, &opts, cancel, |seg| {
                    if cancel.load(Ordering::Acquire) {
                        return false;
                    }
                    segment_emit(
                        "whisper:segment",
                        json!({ "stream": stream_id, "segment": seg }),
                    );
                    true
                })
            },
        )?;
        Ok(TranscribeStream { stream })
    }

    /// Release a reserved transcription worker after frontend event
    /// listeners are installed. Internal handshake used by TypeScript.
    #[command]
    pub fn start_stream(&self, stream_id: u64) -> PluginResult<()> {
        self.streams.start("whisper.startStream", stream_id)
    }

    /// Cancel an in-flight `transcribe(...)` stream. The decode polls the
    /// cancel flag while it runs, so it stops within a fraction of a second;
    /// segments are only emitted once the decode of the buffer finishes, so a
    /// cancelled run emits none. Idempotent — unknown / already-finished ids are a no-op.
    #[command]
    pub fn cancel(&self, stream_id: u64) -> PluginResult<()> {
        self.streams.cancel(stream_id);
        Ok(())
    }

    /// Synchronous one-shot; concatenates segment text.
    ///
    /// Takes a permit from the bounded pool (timed: fails with "busy" rather
    /// than blocking a call-pool worker) and runs with no registry lock held.
    #[command]
    pub fn transcribe_all(
        &self,
        model_id: u64,
        samples: String,
        sample_rate: Option<u32>,
        channels: Option<u32>,
        opts: Option<TranscribeOptions>,
    ) -> PluginResult<String> {
        validate_audio_format(sample_rate, channels)?;
        let opts = opts.unwrap_or_default();
        validate_options(&opts)?;
        let samples = decode_samples_str(&samples)?;
        let model = self
            .models
            .get_arc_arg(model_id, "modelId", "whisper.transcribeAll")?;

        self.streams.run_sync("whisper.transcribeAll", |cancel| {
            let mut out = String::new();
            backend::transcribe(&model, &samples, &opts, cancel, |seg| {
                out.push_str(&seg.text);
                !cancel.load(Ordering::Acquire)
            })?;
            Ok(out)
        })
    }

    #[command]
    pub fn detect_language(
        &self,
        model_id: u64,
        samples: String,
        sample_rate: Option<u32>,
        channels: Option<u32>,
    ) -> PluginResult<DetectLanguageResult> {
        validate_audio_format(sample_rate, channels)?;
        let samples = decode_samples_str(&samples)?;
        let model = self
            .models
            .get_arc_arg(model_id, "modelId", "whisper.detectLanguage")?;

        let (language, confidence) = self.streams.run_sync("whisper.detectLanguage", |_| {
            backend::detect_language(&model, &samples)
        })?;
        Ok(DetectLanguageResult {
            language,
            confidence,
        })
    }

    #[command]
    pub fn unload(&self, model_id: u64) -> PluginResult<()> {
        self.models.remove(model_id);
        Ok(())
    }

    #[command]
    pub fn list(&self) -> PluginResult<Vec<WhisperModel>> {
        Ok(self.models.snapshot(|id, m| {
            Ok(WhisperModel {
                id,
                path: m.path.to_string_lossy().into_owned(),
                sample_rate: 16_000,
                language: "multi".into(),
            })
        }))
    }

    /// Host shutdown: the host never drops plugin state, so cancel every
    /// stream, wake gated workers, wait (bounded) for inference to stop and
    /// release the models explicitly.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        if !self.streams.shutdown(DEFAULT_SHUTDOWN_DEADLINE) {
            tracing::warn!("whisper: inference still running at shutdown deadline");
        }
        for id in self.models.ids() {
            self.models.remove(id);
        }
        Ok(())
    }
}

/// Language argument handed to whisper.cpp. `None` / `""` / `"auto"` map to
/// `"auto"` (detect, then transcribe). Never use whisper.cpp's `detect_language`
/// flag for this: it makes the run return after detection with no segments.
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
fn whisper_language(language: Option<&str>) -> &str {
    match language {
        None | Some("" | "auto") => "auto",
        Some(language) => language,
    }
}

/// Thread count handed to whisper.cpp: `0` means "all cores", anything else is
/// clamped to `available` so a hostile `threads: 100000` cannot make
/// whisper.cpp spawn 100k threads. Always at least 1.
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
fn effective_threads(requested: u32, available: usize) -> i32 {
    let available = u32::try_from(available.max(1)).unwrap_or(u32::MAX);
    let threads = if requested == 0 {
        available
    } else {
        requested.min(available)
    };
    i32::try_from(threads).unwrap_or(i32::MAX)
}

/// Bound the free-form option strings before they reach whisper.cpp.
fn validate_options(opts: &TranscribeOptions) -> PluginResult<()> {
    if let Some(language) = opts.language.as_deref() {
        let valid = language.len() <= MAX_LANGUAGE_BYTES
            && language
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !valid {
            return Err(PluginError::InvalidArg {
                key: "opts.language".into(),
                detail: format!(
                    "language must be \"auto\" or a short code of at most {MAX_LANGUAGE_BYTES} \
                     ASCII letters / digits / '-'"
                ),
            });
        }
    }
    if opts
        .prompt
        .as_deref()
        .is_some_and(|p| p.len() > MAX_PROMPT_BYTES)
    {
        return Err(PluginError::InvalidArg {
            key: "opts.prompt".into(),
            detail: format!("prompt exceeds {MAX_PROMPT_BYTES} bytes"),
        });
    }
    Ok(())
}

/// Decode either base64 LE f32 (default) or, when the string starts with `[`,
/// a JSON array of numbers (tests and quick experiments).
fn decode_samples_str(raw: &str) -> PluginResult<Vec<f32>> {
    // Cap the encoded length first so a 1+ GB JS-supplied base64 can't
    // double host RSS via the `decode` allocation. 256 MiB ≈ 23 min of
    // 16 kHz mono f32 PCM — chunk longer audio JS-side.
    const MAX_AUDIO_BYTES: usize = 256 * 1024 * 1024;
    if raw.len() > MAX_AUDIO_BYTES {
        return Err(PluginError::InvalidArg {
            key: "samples".into(),
            detail: format!("audio exceeds {MAX_AUDIO_BYTES}-byte cap; chunk on the JS side"),
        });
    }
    let samples = if raw.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<f32>>(raw).map_err(|e| PluginError::InvalidArg {
            key: "samples".into(),
            detail: format!("JSON array of numbers expected: {e}"),
        })?
    } else {
        decode_base64_f32(raw)?
    };
    if samples.is_empty() {
        return Err(PluginError::InvalidArg {
            key: "samples".into(),
            detail: "audio is empty".into(),
        });
    }
    if samples.iter().any(|s| !s.is_finite()) {
        return Err(PluginError::InvalidArg {
            key: "samples".into(),
            detail: "audio contains NaN or infinite samples".into(),
        });
    }
    Ok(samples)
}

fn decode_base64_f32(raw: &str) -> PluginResult<Vec<f32>> {
    let bytes = STANDARD.decode(raw).map_err(|e| PluginError::InvalidArg {
        key: "samples".into(),
        detail: format!("base64 decode: {e}"),
    })?;
    if bytes.len() % 4 != 0 {
        return Err(PluginError::InvalidArg {
            key: "samples".into(),
            detail: format!(
                "expected base64 of f32 PCM (multiple of 4 bytes), got {}",
                bytes.len()
            ),
        });
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.as_chunks::<4>().0 {
        out.push(f32::from_le_bytes(*chunk));
    }
    Ok(out)
}

/// Whisper assumes 16 kHz mono f32 PCM; wrong values yield gibberish.
fn validate_audio_format(sample_rate: Option<u32>, channels: Option<u32>) -> PluginResult<()> {
    let sample_rate = sample_rate.unwrap_or(16_000);
    let channels = channels.unwrap_or(1);
    if sample_rate != 16_000 {
        return Err(PluginError::InvalidArg {
            key: "sampleRate".into(),
            detail: format!(
                "whisper requires 16000 Hz, got {sample_rate} — resample on the JS side or \
                 configure the capture pipeline accordingly"
            ),
        });
    }
    if channels != 1 {
        return Err(PluginError::InvalidArg {
            key: "channels".into(),
            detail: format!(
                "whisper requires mono (1 channel), got {channels} — downmix on the JS side \
                 before forwarding samples"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(samples: &[f32]) -> String {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        STANDARD.encode(bytes)
    }

    fn invalid_key(result: PluginResult<impl std::fmt::Debug>) -> String {
        match result {
            Err(PluginError::InvalidArg { key, .. }) => key,
            other => panic!("expected InvalidArg, got {other:?}"),
        }
    }

    #[test]
    fn threads_are_clamped_to_available_parallelism() {
        assert_eq!(effective_threads(0, 8), 8);
        assert_eq!(effective_threads(4, 8), 4);
        assert_eq!(effective_threads(100_000, 8), 8);
        assert_eq!(effective_threads(u32::MAX, 8), 8);
        assert_eq!(effective_threads(0, 0), 1);
        assert_eq!(effective_threads(3, 0), 1);
    }

    #[test]
    fn auto_language_maps_to_auto_not_detect_only() {
        assert_eq!(whisper_language(None), "auto");
        assert_eq!(whisper_language(Some("")), "auto");
        assert_eq!(whisper_language(Some("auto")), "auto");
        assert_eq!(whisper_language(Some("fr")), "fr");
    }

    #[test]
    fn audio_format_defaults_and_rejections() {
        assert!(validate_audio_format(None, None).is_ok());
        assert!(validate_audio_format(Some(16_000), Some(1)).is_ok());
        assert_eq!(
            invalid_key(validate_audio_format(Some(44_100), None)),
            "sampleRate"
        );
        assert_eq!(
            invalid_key(validate_audio_format(None, Some(2))),
            "channels"
        );
    }

    #[test]
    fn decodes_base64_f32_round_trip() {
        let samples = [0.0_f32, 0.5, -0.25, 1.0];
        assert_eq!(decode_samples_str(&b64(&samples)).unwrap(), samples);
    }

    #[test]
    fn decodes_json_array_mode() {
        assert_eq!(
            decode_samples_str("[0.0, 0.5, -1]").unwrap(),
            vec![0.0, 0.5, -1.0]
        );
        assert_eq!(invalid_key(decode_samples_str("[0.1, \"x\"]")), "samples");
        assert_eq!(invalid_key(decode_samples_str("[1, 2")), "samples");
    }

    #[test]
    fn rejects_malformed_audio() {
        assert_eq!(invalid_key(decode_samples_str("not base64!!")), "samples");
        // 3 bytes is not a whole f32.
        assert_eq!(
            invalid_key(decode_samples_str(&STANDARD.encode([1, 2, 3]))),
            "samples"
        );
        assert_eq!(invalid_key(decode_samples_str("")), "samples");
        assert_eq!(invalid_key(decode_samples_str("[]")), "samples");
        assert_eq!(
            invalid_key(decode_samples_str(&b64(&[f32::NAN]))),
            "samples"
        );
        assert_eq!(
            invalid_key(decode_samples_str(&b64(&[f32::INFINITY]))),
            "samples"
        );
    }

    #[test]
    fn option_strings_are_bounded() {
        let ok = |language: Option<&str>| {
            validate_options(&TranscribeOptions {
                language: language.map(str::to_owned),
                ..TranscribeOptions::default()
            })
        };
        assert!(ok(None).is_ok());
        assert!(ok(Some("auto")).is_ok());
        assert!(ok(Some("")).is_ok());
        assert!(ok(Some("en")).is_ok());
        assert!(ok(Some("zh-CN")).is_ok());
        assert!(ok(Some("english; DROP")).is_err());
        assert!(ok(Some(&"a".repeat(MAX_LANGUAGE_BYTES + 1))).is_err());
        let long_prompt = TranscribeOptions {
            prompt: Some("p".repeat(MAX_PROMPT_BYTES + 1)),
            ..TranscribeOptions::default()
        };
        assert_eq!(invalid_key(validate_options(&long_prompt)), "opts.prompt");
    }

    #[test]
    fn unknown_model_is_rejected_before_taking_a_permit() {
        let plugin = WhisperPlugin::default();
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        let audio = b64(&[0.0; 160]);
        assert_eq!(
            invalid_key(plugin.transcribe(&cx, 7, audio.clone(), None, None, None)),
            "modelId"
        );
        assert_eq!(
            invalid_key(plugin.transcribe_all(7, audio.clone(), None, None, None)),
            "modelId"
        );
        assert_eq!(
            invalid_key(plugin.detect_language(7, audio.clone(), None, None)),
            "modelId"
        );
        // Format errors come first and never reach the registry.
        assert_eq!(
            invalid_key(plugin.transcribe(&cx, 7, audio, Some(8_000), None, None)),
            "sampleRate"
        );
    }

    #[test]
    fn start_cancel_and_shutdown_are_safe_on_unknown_streams() {
        let plugin = WhisperPlugin::default();
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        plugin.start_stream(9_999).unwrap();
        plugin.cancel(9_999).unwrap();
        plugin.unload(9_999).unwrap();
        assert!(plugin.list().unwrap().is_empty());
        plugin.plugin_shutdown(&cx).unwrap();
        assert!(plugin.streams.run_sync("whisper.x", |_| Ok(())).is_err());
    }

    #[cfg(not(feature = "inference"))]
    #[test]
    fn list_reports_loaded_models_and_shutdown_releases_them() {
        let plugin = WhisperPlugin::default();
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        let id = plugin
            .models
            .insert(Arc::new(backend::test_model("/models/ggml-base.bin")))
            .unwrap();
        let listed = plugin.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert!(listed[0].path.ends_with("ggml-base.bin"));
        plugin.plugin_shutdown(&cx).unwrap();
        assert!(plugin.list().unwrap().is_empty());
    }
}
