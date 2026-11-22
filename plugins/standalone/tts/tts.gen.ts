export type IsSpeakingArgs = null;

// Empty without `--features synthesis`.
export type ListVoicesArgs = null;

/**
 *  Queues the utterance behind any speech in progress; resolves on
 *  enqueue, not playback end — poll `isSpeaking()`. Call `stop()` first to
 *  interrupt instead.
 */
export type SpeakArgs = {
  text: string;
  opts: SpeakOpts | null;
};

/**
 *  Per-call synthesis knobs. All fields optional — omitted = backend default.
 *
 *  The numeric knobs use the same units on every OS and are mapped onto the
 *  backend's native range (which differs wildly: speech-dispatcher uses
 *  `-100..=100`, AVFoundation `0.1..=2` with `0.5` as normal, WinRT `0.5..=6`):
 *  - `rate`, `pitch`: `0..=2`, `1.0` = the backend's normal value, `0` = its
 *    minimum, `2` = its maximum (clamped).
 *  - `volume`: `0..=1`, `0` = the backend's minimum, `1` = its maximum (clamped).
 */
export type SpeakOpts = {
  voice: string | null;
  rate: number | null;
  pitch: number | null;
  volume: number | null;
};

// Idempotent.
export type StopArgs = null;

// One installed system voice as reported by the active backend.
export type VoiceInfo = {
  id: string;
  name: string;
  // BCP-47 language tag (e.g. `"en-US"`, `"fr-FR"`).
  language: string;
  // `"male"` / `"female"` / `None` (when the OS doesn't report it).
  gender: string | null;
};

// === Result aliases (auto from #[command] return types) ===
export type ListVoicesResult = VoiceInfo[];
export type SpeakResult = null;
export type StopResult = null;
export type IsSpeakingResult = boolean;

// === Command catalog (typed client catalog) ===
export const _commands = {
  listVoices: {
    args: undefined as unknown as ListVoicesArgs,
    result: undefined as unknown as ListVoicesResult,
  },
  speak: { args: undefined as unknown as SpeakArgs, result: undefined as unknown as SpeakResult },
  stop: { args: undefined as unknown as StopArgs, result: undefined as unknown as StopResult },
  isSpeaking: {
    args: undefined as unknown as IsSpeakingArgs,
    result: undefined as unknown as IsSpeakingResult,
  },
} as const;
export type CommandsCatalog = typeof _commands;
