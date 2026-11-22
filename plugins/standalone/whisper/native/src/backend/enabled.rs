use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use tynd_plugin_sdk::{PluginError, PluginResult};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{TranscribeOptions, TranscriptSegment, WordTimestamp};

pub(crate) struct WhisperModel {
    pub(crate) ctx: WhisperContext,
    pub(crate) path: PathBuf,
}

pub(crate) fn load(path: &Path, gpu: bool) -> PluginResult<WhisperModel> {
    let mut params = WhisperContextParameters::default();
    params.use_gpu(gpu);

    let path_str = path
        .to_str()
        .ok_or_else(|| PluginError::Custom(format!("whisper.load: non-utf8 path {path:?}")))?;
    let ctx = WhisperContext::new_with_params(path_str, params)
        .map_err(|e| PluginError::Custom(format!("whisper.load: {e}")))?;

    Ok(WhisperModel {
        ctx,
        path: path.to_path_buf(),
    })
}

/// `0` or an over-large request resolves to at most `available_parallelism()`.
fn thread_count(requested: u32) -> i32 {
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    crate::effective_threads(requested, available)
}

/// C abort hook: `data` points at the stream's `AtomicBool` cancel flag.
unsafe extern "C" fn abort_on_cancel(data: *mut c_void) -> bool {
    // SAFETY: `data` is the `&AtomicBool` passed by `transcribe`, which
    // outlives the synchronous `state.full` call this hook runs inside.
    unsafe { &*data.cast::<AtomicBool>() }.load(Ordering::Acquire)
}

fn build_params(options: &TranscribeOptions) -> FullParams<'_> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    params.set_n_threads(thread_count(options.threads));

    // `detect_language = true` makes whisper.cpp return right after language
    // detection without decoding anything, so auto-detect must go through the
    // `"auto"` language instead (which detects, then transcribes). whisper-rs
    // defaults the language to `"en"`, so `None` would not detect either.
    params.set_language(Some(crate::whisper_language(options.language.as_deref())));
    params.set_translate(options.translate);

    if let Some(prompt) = options.prompt.as_deref() {
        params.set_initial_prompt(prompt);
    }
    params.set_token_timestamps(options.word_timestamps);

    let temperature = if options.temperature.is_finite() {
        options.temperature.clamp(0.0, 5.0)
    } else {
        0.0
    };
    if temperature > 0.0 {
        params.set_temperature(temperature);
    }

    if let Some(max_len) = options.max_len {
        params.set_max_len(i32::try_from(max_len).unwrap_or(i32::MAX));
    }

    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params
}

pub(crate) fn transcribe<F>(
    model: &WhisperModel,
    samples: &[f32],
    options: &TranscribeOptions,
    cancel: &AtomicBool,
    mut callback: F,
) -> PluginResult<()>
where
    F: FnMut(TranscriptSegment) -> bool,
{
    let mut state = model
        .ctx
        .create_state()
        .map_err(|e| PluginError::Custom(format!("whisper.transcribe: state: {e}")))?;
    let mut params = build_params(options);
    // SAFETY: `cancel` outlives the `state.full` call below and the hook only
    // performs an atomic load. Whisper polls it between graph nodes, so cancel
    // latency is a fraction of a second rather than the full decode time.
    unsafe {
        params.set_abort_callback(Some(abort_on_cancel));
        params.set_abort_callback_user_data(std::ptr::from_ref(cancel).cast_mut().cast());
    }
    if let Err(e) = state.full(params, samples) {
        // An aborted decode surfaces as an error code; a cancel is not a failure.
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        return Err(PluginError::Custom(format!(
            "whisper.transcribe: full: {e}"
        )));
    }

    let segment_count = state
        .full_n_segments()
        .map_err(|e| PluginError::Custom(format!("whisper.transcribe: segments: {e}")))?;

    for segment in 0..segment_count {
        let text = state
            .full_get_segment_text_lossy(segment)
            .map_err(|e| PluginError::Custom(format!("whisper.transcribe: segment text: {e}")))?;
        let (start, end) = match (
            state.full_get_segment_t0(segment),
            state.full_get_segment_t1(segment),
        ) {
            (Ok(start), Ok(end)) => (start, end),
            (start, end) => {
                tracing::warn!(
                    segment,
                    ?start,
                    ?end,
                    "whisper: failed to read segment timestamps"
                );
                continue;
            },
        };

        let words = if options.word_timestamps {
            let token_count = state.full_n_tokens(segment).unwrap_or(0).clamp(0, 10_000);
            let mut words = Vec::with_capacity(token_count as usize);
            for token in 0..token_count {
                let text = state
                    .full_get_token_text_lossy(segment, token)
                    .unwrap_or_default();
                let data = state
                    .full_get_token_data(segment, token)
                    .unwrap_or_default();
                words.push(WordTimestamp {
                    text,
                    start_ms: data.t0 * 10,
                    end_ms: data.t1 * 10,
                    confidence: data.p,
                });
            }
            Some(words)
        } else {
            None
        };

        let transcript_segment = TranscriptSegment {
            start_ms: start * 10,
            end_ms: end * 10,
            text,
            words,
        };
        if !callback(transcript_segment) {
            break;
        }
    }

    Ok(())
}

pub(crate) fn detect_language(
    model: &WhisperModel,
    samples: &[f32],
) -> PluginResult<(String, f32)> {
    let mut state = model
        .ctx
        .create_state()
        .map_err(|e| PluginError::Custom(format!("whisper.detectLanguage: state: {e}")))?;
    let threads = thread_count(0);
    let threads_usize = usize::try_from(threads).unwrap_or(1);
    state
        .pcm_to_mel(samples, threads_usize)
        .map_err(|e| PluginError::Custom(format!("whisper.detectLanguage: mel: {e}")))?;
    // `lang_detect` returns the winning id and the probability of every language.
    let (language_id, probabilities) = state
        .lang_detect(0, threads_usize)
        .map_err(|e| PluginError::Custom(format!("whisper.detectLanguage: language: {e}")))?;
    let language = whisper_rs::get_lang_str(language_id)
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let confidence = usize::try_from(language_id)
        .ok()
        .and_then(|index| probabilities.get(index))
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    Ok((language, confidence))
}
