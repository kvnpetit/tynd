//! Local LLM inference. Wraps [`llama-cpp-2`] to load GGUF checkpoints
//! and stream tokens back to the frontend. Models stay resident in a
//! [`Registry`] keyed by `u64`.
//!
//! Building llama.cpp pulls a heavy C++ toolchain (5-15 min cold). The
//! real backend lives behind the `inference` Cargo feature; without it
//! every dispatch arm returns a "rebuild with --features inference"
//! hint. Hardware-accelerated builds (`cuda`, `metal`, `vulkan`) layer
//! on top of `inference`.
//!
//! `generate` returns a `{ stream }` id immediately; tokens arrive as
//! `llm:token { stream, token }` events with a final
//! `llm:end { stream, ok, error? }`. The client wraps the bus
//! subscription into an `AsyncIterable<string>`.

#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    tynd_plugin::{plugin, plugin_impl},
    GatedLimits, GatedStreams, PluginCx, PluginError, PluginResult, Registry,
    DEFAULT_SHUTDOWN_DEADLINE,
};

const MAX_MODELS: usize = 16;
/// Upper bounds on untrusted wire arguments (a huge context allocates a huge
/// KV cache; a huge prompt or stop list makes every token scan expensive).
const MAX_CONTEXT_SIZE: u32 = 1 << 20;
const MAX_GPU_LAYERS: u32 = 1024;
const MAX_PROMPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_STOP_SEQUENCES: usize = 16;
const MAX_STOP_BYTES: usize = 1024;
/// Logical batch size of the llama context and the prompt chunk size. The
/// prompt is decoded in chunks of at most this many tokens, so a prompt longer
/// than llama.cpp's default `n_batch` (2048) never reaches `llama_decode` as one
/// oversized batch (which errors, or hits `GGML_ASSERT` on older builds).
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
const PROMPT_BATCH: usize = 512;

// Two `backend` impls picked at compile time via the `inference` feature.
// Both expose `load` / `generate_stream` / `LlmModel` so dispatch is
// backend-agnostic.

#[cfg(not(feature = "inference"))]
mod backend {
    use std::path::{Path, PathBuf};

    use tynd_plugin_sdk::{PluginError, PluginResult};

    use crate::GenerateOptions;

    const FEATURE_HINT: &str = "llm: rebuild with --features inference to enable local inference \
                                (llama.cpp must be available on the toolchain)";

    /// Marker handle so capacity / registry behaviour can be exercised
    /// without dragging in llama.cpp.
    pub(crate) struct LlmModel {
        pub(crate) path: PathBuf,
        pub(crate) context_size: u32,
    }

    /// Test-only constructor (the stub cannot load real models).
    #[cfg(test)]
    pub(crate) fn test_model(path: &str, context_size: u32) -> LlmModel {
        LlmModel {
            path: PathBuf::from(path),
            context_size,
        }
    }

    pub(crate) fn load(path: &Path, context_size: u32, gpu_layers: u32) -> PluginResult<LlmModel> {
        let _ = (path, context_size, gpu_layers);
        Err(PluginError::Custom(FEATURE_HINT.into()))
    }

    pub(crate) fn generate_stream<F>(
        _model: &LlmModel,
        _prompt: &str,
        _opts: &GenerateOptions,
        _on_token: F,
    ) -> PluginResult<()>
    where
        F: FnMut(&str) -> bool,
    {
        Err(PluginError::Custom(FEATURE_HINT.into()))
    }
}

#[cfg(feature = "inference")]
mod backend {
    //! `LlamaBackend::init()` is process-global; lazy-init once and share
    //! the `Arc` across loaded models. Stop sequences match against the
    //! cumulative generated text so multi-token stops still trigger.

    use std::{path::PathBuf, sync::Arc};

    use llama_cpp_2::{
        context::params::LlamaContextParams,
        llama_backend::LlamaBackend,
        llama_batch::LlamaBatch,
        model::{params::LlamaModelParams, AddBos, LlamaModel as RawModel, Special},
        sampling::LlamaSampler,
    };
    use parking_lot::Mutex;
    use tynd_plugin_sdk::{PluginError, PluginResult};

    use crate::GenerateOptions;

    /// Process-wide handle on `llama_backend_init` (non-cloneable global).
    static BACKEND: Mutex<Option<Arc<LlamaBackend>>> = Mutex::new(None);

    fn get_backend() -> PluginResult<Arc<LlamaBackend>> {
        let mut guard = BACKEND.lock();
        if guard.is_none() {
            let backend = LlamaBackend::init()
                .map_err(|e| PluginError::Custom(format!("llm: backend init failed: {e}")))?;
            *guard = Some(Arc::new(backend));
        }
        Ok(guard.as_ref().expect("backend just set").clone())
    }

    pub(crate) struct LlmModel {
        pub(crate) model: Arc<RawModel>,
        pub(crate) path: PathBuf,
        pub(crate) context_size: u32,
    }

    pub(crate) fn load(
        path: &std::path::Path,
        context_size: u32,
        gpu_layers: u32,
    ) -> PluginResult<LlmModel> {
        let backend = get_backend()?;
        let mut params = LlamaModelParams::default();
        if gpu_layers > 0 {
            params = params.with_n_gpu_layers(gpu_layers as i32);
        }
        let model = RawModel::load_from_file(&backend, path, &params)
            .map_err(|e| PluginError::Custom(format!("llm: model load failed: {e}")))?;
        Ok(LlmModel {
            model: Arc::new(model),
            path: path.to_path_buf(),
            context_size,
        })
    }

    /// `on_token` is called once per detokenised piece; returning `false`
    /// aborts the loop early.
    pub(crate) fn generate_stream<F>(
        model: &LlmModel,
        prompt: &str,
        opts: &GenerateOptions,
        mut on_token: F,
    ) -> PluginResult<()>
    where
        F: FnMut(&str) -> bool,
    {
        let backend = get_backend()?;

        let n_ctx = std::num::NonZeroU32::new(model.context_size.max(1))
            .expect("context_size >= 1 enforced via .max(1)");
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(n_ctx))
            .with_n_batch(crate::PROMPT_BATCH as u32);
        let mut ctx = model
            .model
            .new_context(&backend, ctx_params)
            .map_err(|e| PluginError::Custom(format!("llm: context create: {e}")))?;

        // `AddBos::Always` — most chat templates expect a leading <s>.
        let tokens = model
            .model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| PluginError::Custom(format!("llm: tokenize prompt: {e}")))?;

        let n_prompt = tokens.len() as i32;
        let n_ctx_i = i32::try_from(model.context_size).unwrap_or(i32::MAX);
        // `saturating_add` prevents wrap-around for huge `max_tokens`.
        let max_t = i32::try_from(opts.max_tokens).unwrap_or(i32::MAX);
        if n_prompt.saturating_add(max_t) > n_ctx_i {
            return Err(PluginError::Custom(format!(
                "llm: prompt ({n_prompt} tok) + max_tokens ({}) exceeds context window ({n_ctx_i})",
                opts.max_tokens
            )));
        }

        // Decode the prompt in chunks of at most `PROMPT_BATCH` tokens; only the
        // final prompt token requests logits to start sampling.
        let mut batch = LlamaBatch::new(tokens.len().clamp(1, crate::PROMPT_BATCH), 1);
        let last_index = tokens.len().saturating_sub(1);
        for (chunk_no, chunk) in tokens.chunks(crate::PROMPT_BATCH).enumerate() {
            batch.clear();
            let offset = chunk_no * crate::PROMPT_BATCH;
            for (i, tok) in chunk.iter().enumerate() {
                let index = offset + i;
                batch
                    .add(*tok, index as i32, &[0], index == last_index)
                    .map_err(|e| PluginError::Custom(format!("llm: batch.add: {e}")))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| PluginError::Custom(format!("llm: prompt decode: {e}")))?;
        }

        // `dist` consumes a `u32` seed; clamp `u64` callers to `u32::MAX`
        // and warn so a 64-bit seed isn't silently truncated to its low
        // 32 bits (which would map two distinct callers to the same RNG
        // stream).
        let seed: u32 = opts
            .seed
            .map(|s| {
                if s > u64::from(u32::MAX) {
                    tracing::warn!(
                        seed = s,
                        "llm.generate: seed > u32::MAX, clamping to u32::MAX (sampler accepts u32)"
                    );
                    u32::MAX
                } else {
                    s as u32
                }
            })
            .unwrap_or_else(rand::random::<u32>);
        // Sanitise sampler inputs — NaN propagates through softmax and
        // collapses output to a deterministic edge token; Infinity in
        // `top_p` disables nucleus filtering. Clamp to spec-valid
        // ranges and fall back to library defaults on non-finite.
        let temperature = if opts.temperature.is_finite() {
            opts.temperature.clamp(0.0, 5.0)
        } else {
            0.7
        };
        let top_p = if opts.top_p.is_finite() {
            opts.top_p.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let top_k = i32::try_from(opts.top_k).unwrap_or(i32::MAX);
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature),
            LlamaSampler::top_k(top_k),
            LlamaSampler::top_p(top_p, 1),
            LlamaSampler::dist(seed),
        ]);

        // sample → stop filter → emit → re-feed as 1-token batch. The filter
        // holds back text that may be the start of a stop sequence and never
        // emits a matched stop sequence (or anything after it).
        let mut n_cur = n_prompt;
        let mut stops = crate::StopFilter::new(&opts.stop);
        let mut utf8 = crate::Utf8Stream::default();
        for _ in 0..opts.max_tokens {
            let new_token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(new_token);

            if model.model.is_eog_token(new_token) {
                break;
            }

            // Byte-level vocabularies split one character across tokens, so
            // decode incrementally and hold back an incomplete tail.
            let bytes = model
                .model
                .token_to_bytes(new_token, Special::Tokenize)
                .map_err(|e| PluginError::Custom(format!("llm: detokenize: {e}")))?;
            let piece = utf8.push(&bytes);

            let (out, stopped) = stops.push(&piece);
            if !out.is_empty() && !on_token(&out) {
                return Ok(());
            }
            if stopped {
                return Ok(());
            }

            batch.clear();
            batch
                .add(new_token, n_cur, &[0], true)
                .map_err(|e| PluginError::Custom(format!("llm: batch.add (gen): {e}")))?;
            n_cur += 1;
            ctx.decode(&mut batch)
                .map_err(|e| PluginError::Custom(format!("llm: gen decode: {e}")))?;
        }

        // Natural end (EOG or `max_tokens`): release the held-back tails.
        let tail = utf8.finish();
        let (mut rest, stopped) = stops.push(&tail);
        if !stopped {
            rest.push_str(&stops.finish());
        }
        if !rest.is_empty() {
            on_token(&rest);
        }
        Ok(())
    }
}

/// Incremental UTF-8 decoder for token byte pieces. A character may be split
/// across tokens (byte-level BPE, `<0xE4>` byte fallback), so the incomplete
/// tail is kept for the next push instead of becoming U+FFFD; only bytes that
/// can never form a valid sequence are replaced.
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

#[cfg_attr(not(feature = "inference"), allow(dead_code))]
impl Utf8Stream {
    /// Returns the text completed by `bytes`; an incomplete tail is retained.
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        let mut start = 0;
        loop {
            match std::str::from_utf8(&self.pending[start..]) {
                Ok(valid) => {
                    out.push_str(valid);
                    start = self.pending.len();
                    break;
                },
                Err(error) => {
                    let valid_end = start + error.valid_up_to();
                    out.push_str(&String::from_utf8_lossy(&self.pending[start..valid_end]));
                    let Some(len) = error.error_len() else {
                        // Incomplete sequence at the end: keep for later.
                        start = valid_end;
                        break;
                    };
                    out.push(char::REPLACEMENT_CHARACTER);
                    start = valid_end + len;
                },
            }
        }
        self.pending.drain(..start);
        out
    }

    /// Flushes any retained incomplete bytes lossily at end of generation.
    fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}

/// Streaming stop-sequence matcher. Text that could still turn into a stop
/// sequence (a suffix equal to a proper prefix of one) is held back until the
/// next piece decides it; a completed stop sequence is dropped with everything
/// after it, so it never reaches the consumer.
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
struct StopFilter<'a> {
    stops: Vec<&'a str>,
    pending: String,
}

#[cfg_attr(not(feature = "inference"), allow(dead_code))]
impl<'a> StopFilter<'a> {
    fn new(stops: &'a [String]) -> Self {
        Self {
            stops: stops
                .iter()
                .map(String::as_str)
                .filter(|s| !s.is_empty())
                .collect(),
            pending: String::new(),
        }
    }

    /// Returns the text safe to emit and whether a stop sequence matched.
    fn push(&mut self, piece: &str) -> (String, bool) {
        if self.stops.is_empty() {
            return (piece.to_owned(), false);
        }
        self.pending.push_str(piece);
        let earliest = self
            .stops
            .iter()
            .filter_map(|stop| self.pending.find(stop))
            .min();
        if let Some(index) = earliest {
            let out = self.pending[..index].to_owned();
            self.pending.clear();
            return (out, true);
        }
        let hold = self
            .stops
            .iter()
            .filter_map(|stop| {
                (1..stop.len())
                    .rev()
                    .filter(|&k| stop.is_char_boundary(k))
                    .find(|&k| self.pending.ends_with(&stop[..k]))
            })
            .max()
            .unwrap_or(0);
        let split = self.pending.len() - hold;
        let out = self.pending[..split].to_owned();
        self.pending.drain(..split);
        (out, false)
    }

    /// Held-back text to release when generation ends without a match.
    fn finish(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

/// Per-call generation knobs. Conservative defaults (low-temp).
#[derive(Debug, Clone, Serialize, Deserialize, Type, bon::Builder)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct GenerateOptions {
    #[serde(default = "default_max_tokens")]
    #[builder(default = default_max_tokens())]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    #[builder(default = default_temperature())]
    pub temperature: f32,
    #[serde(default = "default_top_k")]
    #[builder(default = default_top_k())]
    pub top_k: u32,
    #[serde(default = "default_top_p")]
    #[builder(default = default_top_p())]
    pub top_p: f32,
    #[serde(default)]
    #[specta(type = Option<f64>)]
    pub seed: Option<u64>,
    #[serde(default)]
    #[builder(default)]
    pub stop: Vec<String>,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            top_k: default_top_k(),
            top_p: default_top_p(),
            seed: None,
            stop: Vec::new(),
        }
    }
}

const fn default_max_tokens() -> u32 {
    512
}
fn default_temperature() -> f32 {
    0.7
}
const fn default_top_k() -> u32 {
    40
}
fn default_top_p() -> f32 {
    0.9
}

/// `id` rendered as JS `number` (`MAX_MODELS` keeps it well below
/// `Number.MAX_SAFE_INTEGER`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmModel {
    #[specta(type = f64)]
    pub id: u64,
    pub path: String,
    pub context_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenerateStream {
    #[specta(type = f64)]
    pub stream: u64,
}

#[plugin(name = "llm")]
pub struct LlmPlugin {
    /// Loaded models. Entries are `Arc`s so inference clones the handle out
    /// and runs with no registry lock held.
    models: Arc<Registry<Arc<backend::LlmModel>>>,
    /// Bounded permits (at most `available_parallelism()` concurrent runs,
    /// timed acquisition), cancel flags and start gates of every stream.
    streams: Arc<GatedStreams>,
}

impl std::fmt::Debug for LlmPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmPlugin").finish_non_exhaustive()
    }
}

impl Default for LlmPlugin {
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

/// Resolve and bound `load` arguments (defaults: 2048 tokens, CPU only).
fn validate_load_args(
    context_size: Option<u32>,
    gpu_layers: Option<u32>,
) -> PluginResult<(u32, u32)> {
    let context_size = context_size.unwrap_or(2048);
    if context_size == 0 || context_size > MAX_CONTEXT_SIZE {
        return Err(PluginError::InvalidArg {
            key: "contextSize".into(),
            detail: format!("llm.load: contextSize must be in 1..={MAX_CONTEXT_SIZE}"),
        });
    }
    let gpu_layers = gpu_layers.unwrap_or(0);
    if gpu_layers > MAX_GPU_LAYERS {
        return Err(PluginError::InvalidArg {
            key: "gpuLayers".into(),
            detail: format!("llm.load: gpuLayers must be at most {MAX_GPU_LAYERS}"),
        });
    }
    Ok((context_size, gpu_layers))
}

/// Reject prompts / options the backend cannot handle before taking a permit.
fn validate_generate_args(
    operation: &str,
    prompt: &str,
    opts: &GenerateOptions,
) -> PluginResult<()> {
    if prompt.is_empty() {
        return Err(PluginError::InvalidArg {
            key: "prompt".into(),
            detail: format!("{operation}: prompt must not be empty"),
        });
    }
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(PluginError::InvalidArg {
            key: "prompt".into(),
            detail: format!("{operation}: prompt exceeds {MAX_PROMPT_BYTES} bytes"),
        });
    }
    if opts.stop.len() > MAX_STOP_SEQUENCES || opts.stop.iter().any(|s| s.len() > MAX_STOP_BYTES) {
        return Err(PluginError::InvalidArg {
            key: "opts.stop".into(),
            detail: format!(
                "{operation}: at most {MAX_STOP_SEQUENCES} stop sequences of {MAX_STOP_BYTES} bytes"
            ),
        });
    }
    Ok(())
}

#[plugin_impl]
impl LlmPlugin {
    #[command]
    pub fn load(
        &self,
        cx: &PluginCx,
        path: String,
        context_size: Option<u32>,
        gpu_layers: Option<u32>,
    ) -> PluginResult<LlmModel> {
        let (context_size, gpu_layers) = validate_load_args(context_size, gpu_layers)?;
        let authorized_path = cx.authorize_fs_path(&path, "read")?;
        let model = backend::load(&PathBuf::from(&authorized_path), context_size, gpu_layers)?;
        let id = self.models.insert(Arc::new(model))?;
        // Report the resolved path, the same one `list` returns.
        Ok(LlmModel {
            id,
            path: authorized_path,
            context_size,
        })
    }

    /// Streaming generation. Returns a `stream` id; tokens arrive as
    /// `llm:token { stream, token }` events with a final
    /// `llm:end { stream, ok, error? }`. The stream must be released with
    /// `startStream`; otherwise it expires and ends with `ok: false`.
    #[command]
    pub fn generate(
        &self,
        cx: &PluginCx,
        model_id: u64,
        prompt: String,
        opts: Option<GenerateOptions>,
    ) -> PluginResult<GenerateStream> {
        let opts = opts.unwrap_or_default();
        validate_generate_args("llm.generate", &prompt, &opts)?;
        let model = self
            .models
            .get_arc_arg(model_id, "modelId", "llm.generate")?;

        let emit = cx.emitter();
        let token_emit = Arc::clone(&emit);
        // Explicit event-subscription barrier: the worker emits nothing until
        // the frontend installed its listeners and called `startStream`.
        let stream = self.streams.spawn_stream(
            "llm.generate",
            "tynd-llm",
            emit,
            "llm:end",
            move |stream_id, cancel| {
                backend::generate_stream(&model, &prompt, &opts, |tok| {
                    if cancel.load(Ordering::Acquire) {
                        return false;
                    }
                    token_emit("llm:token", json!({ "stream": stream_id, "token": tok }));
                    true
                })
            },
        )?;
        Ok(GenerateStream { stream })
    }

    /// Release a reserved generation worker after frontend event listeners
    /// are installed. Internal handshake used by the TypeScript client.
    #[command]
    pub fn start_stream(&self, stream_id: u64) -> PluginResult<()> {
        self.streams.start("llm.startStream", stream_id)
    }

    /// Cancel an in-flight `generate(...)` stream. The inference thread
    /// notices on the next token and emits its terminating `llm:end`.
    /// Idempotent — unknown / already-finished ids are a no-op.
    #[command]
    pub fn cancel(&self, stream_id: u64) -> PluginResult<()> {
        self.streams.cancel(stream_id);
        Ok(())
    }

    /// Synchronous one-shot; collects tokens into a single string.
    ///
    /// Takes a permit from the bounded pool (timed: fails with "busy" rather
    /// than blocking a call-pool worker) and runs with no registry lock held.
    #[command]
    pub fn complete(
        &self,
        model_id: u64,
        prompt: String,
        opts: Option<GenerateOptions>,
    ) -> PluginResult<String> {
        let opts = opts.unwrap_or_default();
        validate_generate_args("llm.complete", &prompt, &opts)?;
        let model = self
            .models
            .get_arc_arg(model_id, "modelId", "llm.complete")?;

        self.streams.run_sync("llm.complete", |cancel| {
            let mut out = String::new();
            backend::generate_stream(&model, &prompt, &opts, |tok| {
                out.push_str(tok);
                !cancel.load(Ordering::Acquire)
            })?;
            Ok(out)
        })
    }

    #[command]
    pub fn unload(&self, model_id: u64) -> PluginResult<()> {
        self.models.remove(model_id);
        Ok(())
    }

    #[command]
    pub fn list(&self) -> PluginResult<Vec<LlmModel>> {
        Ok(self.models.snapshot(|id, m| {
            Ok(LlmModel {
                id,
                path: m.path.to_string_lossy().into_owned(),
                context_size: m.context_size,
            })
        }))
    }

    /// Host shutdown: the host never drops plugin state, so cancel every
    /// stream, wake gated workers, wait (bounded) for inference to stop and
    /// release the models explicitly.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        if !self.streams.shutdown(DEFAULT_SHUTDOWN_DEADLINE) {
            tracing::warn!("llm: inference still running at shutdown deadline");
        }
        for id in self.models.ids() {
            self.models.remove(id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_stream_joins_a_character_split_across_tokens() {
        let mut d = Utf8Stream::default();
        let mut out = String::new();
        for b in "你😀".as_bytes() {
            out.push_str(&d.push(std::slice::from_ref(b)));
        }
        assert_eq!(out, "你😀");
        assert_eq!(d.finish(), "");
    }

    #[test]
    fn utf8_stream_holds_incomplete_tail_and_flushes_it_lossily() {
        let mut d = Utf8Stream::default();
        assert_eq!(d.push(b"ab\xE4\xB8"), "ab");
        assert_eq!(d.push(b"\xAD!"), "中!");
        assert_eq!(d.push(b"\xE4"), "");
        assert_eq!(d.finish(), "\u{FFFD}");
    }

    #[test]
    fn utf8_stream_replaces_only_invalid_bytes() {
        let mut d = Utf8Stream::default();
        assert_eq!(d.push(b"a\xFFb\xE4"), "a\u{FFFD}b");
        assert_eq!(d.push(b"x"), "\u{FFFD}x");
    }

    fn run_filter(stops: &[&str], pieces: &[&str]) -> (String, bool) {
        let stops: Vec<String> = stops.iter().map(|s| (*s).to_owned()).collect();
        let mut filter = StopFilter::new(&stops);
        let mut out = String::new();
        for piece in pieces {
            let (emit, stopped) = filter.push(piece);
            out.push_str(&emit);
            if stopped {
                return (out, true);
            }
        }
        out.push_str(&filter.finish());
        (out, false)
    }

    #[test]
    fn stop_text_is_never_emitted() {
        // Stop split across pieces: nothing of it leaks.
        assert_eq!(
            run_filter(&["User:"], &["Hello", "\nUs", "er:", " hi"]),
            ("Hello\n".to_owned(), true)
        );
        // Stop inside a single piece, text after it is dropped.
        assert_eq!(
            run_filter(&["</s>"], &["abc</s>def"]),
            ("abc".to_owned(), true)
        );
        // Earliest of several stops wins.
        assert_eq!(
            run_filter(&["zz", "b"], &["aabzz"]),
            ("aa".to_owned(), true)
        );
    }

    #[test]
    fn held_back_prefix_is_released_when_no_stop_follows() {
        assert_eq!(
            run_filter(&["User:"], &["Hi Us", "ers"]),
            ("Hi Users".to_owned(), false)
        );
        // Generation ends mid-prefix: the tail is flushed.
        assert_eq!(
            run_filter(&["User:"], &["Hi Us"]),
            ("Hi Us".to_owned(), false)
        );
        // No stops: pass-through; empty stops are ignored.
        assert_eq!(run_filter(&[], &["a", "b"]), ("ab".to_owned(), false));
        assert_eq!(run_filter(&[""], &["a", "b"]), ("ab".to_owned(), false));
    }

    #[test]
    fn stop_filter_respects_char_boundaries() {
        assert_eq!(
            run_filter(&["éé!"], &["x", "é", "éé", "!"]),
            ("xé".to_owned(), true)
        );
    }

    #[test]
    fn load_args_default_and_bounds() {
        assert_eq!(validate_load_args(None, None).unwrap(), (2048, 0));
        assert_eq!(validate_load_args(Some(512), Some(8)).unwrap(), (512, 8));
        assert_eq!(
            validate_load_args(Some(MAX_CONTEXT_SIZE), Some(MAX_GPU_LAYERS)).unwrap(),
            (MAX_CONTEXT_SIZE, MAX_GPU_LAYERS)
        );
        for bad in [
            validate_load_args(Some(0), None),
            validate_load_args(Some(MAX_CONTEXT_SIZE + 1), None),
            validate_load_args(None, Some(MAX_GPU_LAYERS + 1)),
        ] {
            assert!(matches!(bad, Err(PluginError::InvalidArg { .. })));
        }
    }

    #[test]
    fn generate_args_validation() {
        let opts = GenerateOptions::default();
        assert!(validate_generate_args("t", "hello", &opts).is_ok());
        assert!(validate_generate_args("t", "", &opts).is_err());
        let huge = "x".repeat(MAX_PROMPT_BYTES + 1);
        assert!(validate_generate_args("t", &huge, &opts).is_err());

        let too_many = GenerateOptions {
            stop: vec!["a".into(); MAX_STOP_SEQUENCES + 1],
            ..GenerateOptions::default()
        };
        assert!(validate_generate_args("t", "hi", &too_many).is_err());
        let too_long = GenerateOptions {
            stop: vec!["a".repeat(MAX_STOP_BYTES + 1)],
            ..GenerateOptions::default()
        };
        assert!(validate_generate_args("t", "hi", &too_long).is_err());
    }

    #[test]
    fn unknown_model_is_rejected_before_taking_a_permit() {
        let plugin = LlmPlugin::default();
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        let generate = plugin.generate(&cx, 42, "hi".into(), None).unwrap_err();
        assert!(matches!(generate, PluginError::InvalidArg { ref key, .. } if key == "modelId"));
        let complete = plugin.complete(42, "hi".into(), None).unwrap_err();
        assert!(matches!(complete, PluginError::InvalidArg { ref key, .. } if key == "modelId"));
        // Empty prompt is reported as such, ahead of the model lookup.
        let empty = plugin.complete(42, String::new(), None).unwrap_err();
        assert!(matches!(empty, PluginError::InvalidArg { ref key, .. } if key == "prompt"));
    }

    #[test]
    fn start_cancel_and_shutdown_are_safe_on_unknown_streams() {
        let plugin = LlmPlugin::default();
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        plugin.start_stream(9_999).unwrap();
        plugin.cancel(9_999).unwrap();
        plugin.unload(9_999).unwrap();
        assert!(plugin.list().unwrap().is_empty());
        plugin.plugin_shutdown(&cx).unwrap();
        // After shutdown the plugin refuses new inference.
        assert!(plugin.streams.run_sync("llm.complete", |_| Ok(())).is_err());
    }

    #[cfg(not(feature = "inference"))]
    #[test]
    fn list_reports_loaded_models_and_shutdown_releases_them() {
        let plugin = LlmPlugin::default();
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        let id = plugin
            .models
            .insert(Arc::new(backend::test_model("/models/a.gguf", 4096)))
            .unwrap();
        let listed = plugin.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].context_size, 4096);
        assert!(listed[0].path.ends_with("a.gguf"));
        plugin.plugin_shutdown(&cx).unwrap();
        assert!(plugin.list().unwrap().is_empty());
    }
}
