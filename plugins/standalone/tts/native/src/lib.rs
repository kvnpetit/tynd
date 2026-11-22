//! Text-to-speech via the cross-platform [`tts`] crate.
//!
//! Backends: SAPI 5 (Windows), `AVSpeechSynthesizer` (macOS/iOS),
//! speech-dispatcher / espeak (Linux).
//!
//! The `synthesis` Cargo feature pulls in the `tts` crate. Default builds
//! compile without it; the dispatch surface still exists but returns a
//! deterministic error / empty list / `false`.
//!
//! When `synthesis` is on, the backend keeps a single process-wide `Tts`
//! handle behind a `Mutex` — speech is serial on every supported platform.
//!
//! [`tts`]: https://crates.io/crates/tts

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    tynd_plugin::{plugin, plugin_impl},
    PluginCx, PluginError, PluginResult,
};

/// Hard cap on `speak()` text — SAPI / AVSpeechSynthesizer / espeak all
/// have their own internal limits (much smaller in practice), but a
/// runaway caller would still allocate the full `String` host-side.
const SPEAK_TEXT_MAX_BYTES: usize = 64 * 1024;

/// Per-call synthesis knobs. All fields optional — omitted = backend default.
///
/// The numeric knobs use the same units on every OS and are mapped onto the
/// backend's native range (which differs wildly: speech-dispatcher uses
/// `-100..=100`, AVFoundation `0.1..=2` with `0.5` as normal, WinRT `0.5..=6`):
/// - `rate`, `pitch`: `0..=2`, `1.0` = the backend's normal value, `0` = its
///   minimum, `2` = its maximum (clamped).
/// - `volume`: `0..=1`, `0` = the backend's minimum, `1` = its maximum (clamped).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeakOpts {
    pub voice: Option<String>,
    pub rate: Option<f32>,
    pub pitch: Option<f32>,
    pub volume: Option<f32>,
}

/// Rejects non-finite numeric knobs before anything touches the synthesizer.
fn validate_speak_opts(opts: &SpeakOpts) -> PluginResult<()> {
    for (key, value) in [
        ("opts.rate", opts.rate),
        ("opts.pitch", opts.pitch),
        ("opts.volume", opts.volume),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(PluginError::InvalidArg {
                key: key.into(),
                detail: format!("tts.speak: {key} must be a finite number"),
            });
        }
    }
    Ok(())
}

/// Piecewise-linear map of a rate/pitch knob (`0..=2`, `1` = normal) onto a
/// backend range. Out-of-range input is clamped.
#[cfg_attr(not(feature = "synthesis"), allow(dead_code))]
fn scale_around_normal(value: f32, min: f32, normal: f32, max: f32) -> f32 {
    let value = value.clamp(0.0, 2.0);
    if value <= 1.0 {
        min + (normal - min) * value
    } else {
        normal + (max - normal) * (value - 1.0)
    }
}

/// Linear map of a `0..=1` knob onto a backend range. Out-of-range input is
/// clamped.
#[cfg_attr(not(feature = "synthesis"), allow(dead_code))]
fn scale_linear(value: f32, min: f32, max: f32) -> f32 {
    min + (max - min) * value.clamp(0.0, 1.0)
}

/// One installed system voice as reported by the active backend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
    /// BCP-47 language tag (e.g. `"en-US"`, `"fr-FR"`).
    pub language: String,
    /// `"male"` / `"female"` / `None` (when the OS doesn't report it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
}

#[plugin(name = "tts")]
#[derive(Default)]
pub struct TtsPlugin;

impl std::fmt::Debug for TtsPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsPlugin").finish_non_exhaustive()
    }
}

// `#[command]` requires `&self`; platform speech services own command state.
#[allow(clippy::unused_self)]
#[plugin_impl]
impl TtsPlugin {
    /// Empty without `--features synthesis`.
    #[command]
    pub fn list_voices(&self, _cx: &PluginCx) -> PluginResult<Vec<VoiceInfo>> {
        backend::list_voices()
    }

    /// Queues the utterance behind any speech in progress; resolves on
    /// enqueue, not playback end — poll `isSpeaking()`. Call `stop()` first to
    /// interrupt instead.
    #[command]
    pub fn speak(&self, cx: &PluginCx, text: String, opts: Option<SpeakOpts>) -> PluginResult<()> {
        if text.len() > SPEAK_TEXT_MAX_BYTES {
            return Err(PluginError::InvalidArg {
                key: "text".into(),
                detail: format!(
                    "text must be at most {SPEAK_TEXT_MAX_BYTES} bytes (got {} bytes)",
                    text.len()
                ),
            });
        }
        let opts = opts.unwrap_or_default();
        validate_speak_opts(&opts)?;
        // Fail closed when the host cannot enforce audio-output policy.
        cx.os_call(
            "security",
            "checkAudioOutput",
            json!({ "voice": &opts.voice }),
        )?;
        backend::speak(&text, opts)
    }

    /// Idempotent.
    #[command]
    pub fn stop(&self, _cx: &PluginCx) -> PluginResult<()> {
        backend::stop()
    }

    #[command]
    pub fn is_speaking(&self, _cx: &PluginCx) -> PluginResult<bool> {
        backend::is_speaking()
    }

    /// Native host lifecycle hook. Unlike the public `stop` command this is a
    /// no-op when synthesis was never initialized.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        backend::shutdown()
    }

    // `synthesize` (text → WAV) intentionally absent: the `tts` crate has
    // no uniform render-to-file path. See README "Limitations" for the
    // audio-capture workaround.
}

#[cfg(feature = "synthesis")]
mod backend {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    use parking_lot::Mutex;
    use tts::{Gender, Tts, Voice};

    use super::{PluginError, PluginResult, SpeakOpts, VoiceInfo};

    static SYNTH: OnceLock<Mutex<Tts>> = OnceLock::new();
    /// Voice the synthesizer started with, when the backend can report it;
    /// restored when `speak` omits `voice`.
    static DEFAULT_VOICE: OnceLock<Option<Voice>> = OnceLock::new();
    /// A call explicitly set a voice that has not been undone yet. Only
    /// consulted when the backend cannot report its current voice.
    static VOICE_OVERRIDDEN: AtomicBool = AtomicBool::new(false);
    // Serialises the *construction* of `Tts::default()` — SAPI / AVSpeech
    // / espeak don't all tolerate two live handles in one process. Uses
    // `std::sync::Mutex` because its `new` is `const`.
    static INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn synth() -> PluginResult<&'static Mutex<Tts>> {
        if let Some(s) = SYNTH.get() {
            return Ok(s);
        }
        let _guard = INIT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(s) = SYNTH.get() {
            return Ok(s);
        }
        let t = Tts::default().map_err(|e| PluginError::Custom(format!("tts init failed: {e}")))?;
        let _ = DEFAULT_VOICE.set(initial_voice(&t));
        SYNTH
            .set(Mutex::new(t))
            .map_err(|_| ())
            .expect("INIT_LOCK guarantees we're the unique initialiser");
        Ok(SYNTH.get().expect("SYNTH was just set under INIT_LOCK"))
    }

    /// Voice to restore when `speak` omits `voice`, or `None` when the backend
    /// cannot report its current voice (Linux speech-dispatcher). The first
    /// listed voice is never a valid stand-in: speech-dispatcher lists voices by
    /// language code, so it is not the user's configured default.
    fn initial_voice(t: &Tts) -> Option<Voice> {
        t.voice().ok().flatten()
    }

    /// Index of the language tag that best matches `locale` (a POSIX locale such
    /// as `fr_FR.UTF-8`): an exact tag match first, then the same primary
    /// language. `None` for the `C`/`POSIX` locales or when nothing matches.
    pub(super) fn locale_match(locale: &str, languages: &[String]) -> Option<usize> {
        let base = locale.split(['.', '@']).next().unwrap_or_default();
        let tag = base.replace('_', "-").to_ascii_lowercase();
        let primary = tag.split('-').next().unwrap_or_default();
        if primary.is_empty() || primary == "c" || primary == "posix" {
            return None;
        }
        let lower: Vec<String> = languages.iter().map(|l| l.to_ascii_lowercase()).collect();
        lower.iter().position(|l| *l == tag).or_else(|| {
            lower
                .iter()
                .position(|l| l.split('-').next() == Some(primary))
        })
    }

    fn system_locale() -> Option<String> {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
    }

    /// Best-effort undo of an explicit voice when the previous one is unknown:
    /// the tts API has no "reset to default", so use the voice matching the
    /// system locale. Without a match the explicit voice stays selected.
    fn restore_unknown_default(synth: &mut Tts) {
        if !VOICE_OVERRIDDEN.load(Ordering::Relaxed) {
            return;
        }
        let Some(locale) = system_locale() else {
            return;
        };
        let Ok(voices) = synth.voices() else {
            return;
        };
        let languages: Vec<String> = voices.iter().map(|v| v.language().to_string()).collect();
        let Some(voice) = locale_match(&locale, &languages).and_then(|i| voices.get(i)) else {
            return;
        };
        if synth.set_voice(voice).is_ok() {
            VOICE_OVERRIDDEN.store(false, Ordering::Relaxed);
        }
    }

    /// The voice to apply: the one named by `voice_id`, else the startup
    /// default. Unknown voice ids surface as `InvalidArg` instead of the text
    /// being read in an unexpected default voice; `voices()` failure bubbles
    /// up as well.
    fn resolve_voice(synth: &Tts, voice_id: Option<&str>) -> PluginResult<Option<Voice>> {
        let Some(voice_id) = voice_id else {
            return Ok(DEFAULT_VOICE.get().cloned().flatten());
        };
        let voices = synth
            .voices()
            .map_err(|e| PluginError::Custom(format!("tts.voices: {e}")))?;
        let found = voices
            .into_iter()
            .find(|v| v.id() == voice_id)
            .ok_or_else(|| PluginError::InvalidArg {
                key: "voice".into(),
                detail: format!("unknown voice id: {voice_id}"),
            })?;
        Ok(Some(found))
    }

    /// Applies the per-call options. Every knob is reset to its default when
    /// omitted so settings never leak from one `speak` into the next; explicit
    /// values must be accepted by the backend or the call fails. The voice is
    /// resolved first so an unknown id leaves the synthesizer untouched.
    fn apply_opts(synth: &mut Tts, opts: &SpeakOpts) -> PluginResult<()> {
        let voice = resolve_voice(synth, opts.voice.as_deref())?;
        if let Some(rate) = opts.rate {
            let rate = crate::scale_around_normal(
                rate,
                synth.min_rate(),
                synth.normal_rate(),
                synth.max_rate(),
            );
            synth
                .set_rate(rate)
                .map_err(|e| PluginError::Custom(format!("tts.speak(rate): {e}")))?;
        } else {
            let normal = synth.normal_rate();
            let _ = synth.set_rate(normal);
        }
        if let Some(pitch) = opts.pitch {
            let pitch = crate::scale_around_normal(
                pitch,
                synth.min_pitch(),
                synth.normal_pitch(),
                synth.max_pitch(),
            );
            synth
                .set_pitch(pitch)
                .map_err(|e| PluginError::Custom(format!("tts.speak(pitch): {e}")))?;
        } else {
            let normal = synth.normal_pitch();
            let _ = synth.set_pitch(normal);
        }
        if let Some(volume) = opts.volume {
            let volume = crate::scale_linear(volume, synth.min_volume(), synth.max_volume());
            synth
                .set_volume(volume)
                .map_err(|e| PluginError::Custom(format!("tts.speak(volume): {e}")))?;
        } else {
            let normal = synth.normal_volume();
            let _ = synth.set_volume(normal);
        }
        match (voice, opts.voice.is_some()) {
            (Some(voice), true) => {
                synth
                    .set_voice(&voice)
                    .map_err(|e| PluginError::Custom(format!("tts.speak(voice): {e}")))?;
                VOICE_OVERRIDDEN.store(true, Ordering::Relaxed);
            },
            (Some(voice), false) => {
                let _ = synth.set_voice(&voice);
            },
            (None, false) => restore_unknown_default(synth),
            (None, true) => {},
        }
        Ok(())
    }

    pub(crate) fn list_voices() -> PluginResult<Vec<VoiceInfo>> {
        let synth = synth()?;
        let synth = synth.lock();
        let voices = synth
            .voices()
            .map_err(|e| PluginError::Custom(format!("tts.list_voices: {e}")))?;
        Ok(voices
            .into_iter()
            .map(|v| VoiceInfo {
                id: v.id(),
                name: v.name(),
                language: v.language().to_string(),
                gender: match v.gender() {
                    Some(Gender::Male) => Some("male".into()),
                    Some(Gender::Female) => Some("female".into()),
                    _ => None,
                },
            })
            .collect())
    }

    pub(crate) fn speak(text: &str, opts: SpeakOpts) -> PluginResult<()> {
        let synth = synth()?;
        let mut synth = synth.lock();
        apply_opts(&mut synth, &opts)?;
        // `interrupt = false`: queue behind the current utterance (all three
        // backends queue); `stop()` is the way to interrupt.
        synth
            .speak(text, false)
            .map_err(|e| PluginError::Custom(format!("tts.speak: {e}")))?;
        Ok(())
    }

    pub(crate) fn stop() -> PluginResult<()> {
        let synth = synth()?;
        let mut synth = synth.lock();
        synth
            .stop()
            .map_err(|e| PluginError::Custom(format!("tts.stop: {e}")))?;
        Ok(())
    }

    pub(crate) fn is_speaking() -> PluginResult<bool> {
        let synth = synth()?;
        synth
            .lock()
            .is_speaking()
            .map_err(|e| PluginError::Custom(format!("tts.is_speaking: {e}")))
    }

    pub(crate) fn shutdown() -> PluginResult<()> {
        let Some(synth) = SYNTH.get() else {
            return Ok(());
        };
        synth
            .lock()
            .stop()
            .map_err(|e| PluginError::Custom(format!("tts.shutdown: {e}")))?;
        Ok(())
    }
}

#[cfg(all(test, feature = "synthesis"))]
mod backend_tests {
    use super::backend::locale_match;

    fn langs(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn locale_match_prefers_exact_tag_then_primary_language() {
        let l = langs(&["af", "en-GB", "fr-CA", "fr-FR", "fr"]);
        assert_eq!(locale_match("fr_FR.UTF-8", &l), Some(3));
        assert_eq!(locale_match("fr_BE.UTF-8", &l), Some(2));
        assert_eq!(locale_match("en_US", &l), Some(1));
        assert_eq!(locale_match("de_DE@euro", &l), None);
    }

    #[test]
    fn locale_match_ignores_c_and_posix_and_never_picks_the_first_voice() {
        let l = langs(&["af", "en"]);
        assert_eq!(locale_match("C", &l), None);
        assert_eq!(locale_match("POSIX", &l), None);
        assert_eq!(locale_match("C.UTF-8", &l), None);
        assert_eq!(locale_match("", &l), None);
    }
}

#[cfg(not(feature = "synthesis"))]
mod backend {
    use super::{PluginError, PluginResult, SpeakOpts, VoiceInfo};

    pub(crate) fn list_voices() -> PluginResult<Vec<VoiceInfo>> {
        Ok(Vec::new())
    }

    pub(crate) fn speak(_text: &str, _opts: SpeakOpts) -> PluginResult<()> {
        Err(PluginError::Custom(
            "tts.speak: rebuild with --features synthesis".into(),
        ))
    }

    pub(crate) fn stop() -> PluginResult<()> {
        Ok(())
    }

    pub(crate) fn is_speaking() -> PluginResult<bool> {
        Ok(false)
    }

    pub(crate) fn shutdown() -> PluginResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_pitch_map_around_the_backend_normal() {
        // speech-dispatcher: -100..100, normal 0.
        assert!((scale_around_normal(1.0, -100.0, 0.0, 100.0)).abs() < 1e-4);
        assert!((scale_around_normal(2.0, -100.0, 0.0, 100.0) - 100.0).abs() < 1e-4);
        assert!((scale_around_normal(0.0, -100.0, 0.0, 100.0) + 100.0).abs() < 1e-4);
        // AVFoundation: 0.1..2, normal 0.5 -- 1.0 must not be the maximum.
        assert!((scale_around_normal(1.0, 0.1, 0.5, 2.0) - 0.5).abs() < 1e-4);
        assert!((scale_around_normal(1.5, 0.1, 0.5, 2.0) - 1.25).abs() < 1e-4);
        // WinRT: 0.5..6, normal 1.
        assert!((scale_around_normal(2.0, 0.5, 1.0, 6.0) - 6.0).abs() < 1e-4);
        // Clamped.
        assert!((scale_around_normal(9.0, 0.5, 1.0, 6.0) - 6.0).abs() < 1e-4);
        assert!((scale_around_normal(-3.0, 0.5, 1.0, 6.0) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn volume_maps_onto_the_backend_range() {
        // speech-dispatcher: -100..100, so 0.8 is 80% of the range, not 0.8.
        assert!((scale_linear(0.8, -100.0, 100.0) - 60.0).abs() < 1e-3);
        assert!((scale_linear(1.0, -100.0, 100.0) - 100.0).abs() < 1e-4);
        assert!((scale_linear(0.0, 0.0, 1.0)).abs() < 1e-6);
        assert!((scale_linear(5.0, 0.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((scale_linear(-1.0, 0.0, 1.0)).abs() < 1e-6);
    }

    #[test]
    fn finite_opts_are_accepted() {
        let opts = SpeakOpts {
            voice: None,
            rate: Some(1.5),
            pitch: Some(0.0),
            volume: Some(1.0),
        };
        assert!(validate_speak_opts(&opts).is_ok());
        assert!(validate_speak_opts(&SpeakOpts::default()).is_ok());
    }

    #[test]
    fn non_finite_opts_are_rejected_with_the_offending_key() {
        for (rate, pitch, volume, key) in [
            (Some(f32::NAN), None, None, "opts.rate"),
            (None, Some(f32::INFINITY), None, "opts.pitch"),
            (None, None, Some(f32::NEG_INFINITY), "opts.volume"),
        ] {
            let opts = SpeakOpts {
                voice: None,
                rate,
                pitch,
                volume,
            };
            match validate_speak_opts(&opts) {
                Err(PluginError::InvalidArg { key: got, .. }) => assert_eq!(got, key),
                other => panic!("expected InvalidArg for {key}, got {other:?}"),
            }
        }
    }

    #[test]
    fn oversized_text_is_rejected_before_policy_check() {
        let plugin = TtsPlugin;
        let cx = PluginCx::test();
        let text = "a".repeat(SPEAK_TEXT_MAX_BYTES + 1);
        assert!(matches!(
            plugin.speak(&cx, text, None),
            Err(PluginError::InvalidArg { .. })
        ));
    }

    #[test]
    fn speak_fails_closed_without_security_dispatcher() {
        let plugin = TtsPlugin;
        let cx = PluginCx::test();
        assert!(plugin.speak(&cx, "hello".into(), None).is_err());
    }
}
