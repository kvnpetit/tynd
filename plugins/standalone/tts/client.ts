/**
 * `@tynd/plugin-tts` — single-import entry. Speech is serial, no
 * per-call handle.
 */

import { composePluginClient, defineTypedPlugin } from "@tynd/plugin-sdk";
import type { CommandsCatalog, SpeakOpts } from "./tts.gen";

const _api = defineTypedPlugin<CommandsCatalog>("tts");

/** Single OS-level voice descriptor returned by `tts.listVoices`. */
export interface Voice {
  id: string;
  name: string;
  /** BCP-47 language tag (`"en-US"`, `"fr-FR"`). */
  language: string;
  gender?: "male" | "female";
}

/** Per-utterance options for `tts.speak`. */
export interface TtsSpeakOptions {
  voice?: string;
  /** 0..2, `1` = normal speed (`0` slowest, `2` fastest). Same scale on every OS. */
  rate?: number;
  /** 0..2, `1` = normal pitch. Same scale on every OS. */
  pitch?: number;
  /** 0..1, `1` = loudest. Same scale on every OS. */
  volume?: number;
}

function toSpeakOpts(opts: TtsSpeakOptions | undefined): SpeakOpts | null {
  if (!opts) return null;
  return {
    voice: opts.voice ?? null,
    rate: opts.rate ?? null,
    pitch: opts.pitch ?? null,
    volume: opts.volume ?? null,
  };
}

/** Frontend bindings for the `@tynd/plugin-tts` plugin (system text-to-speech). */
export const tts = composePluginClient(_api, {
  /** List OS-level voices; empty without `--features synthesis`. */
  listVoices(): Promise<Voice[]> {
    return _api.listVoices() as Promise<unknown> as Promise<Voice[]>;
  },
  /** Queue an utterance behind any speech in progress; resolves on enqueue, not playback end — poll `isSpeaking()`. Call `stop()` first to interrupt. */
  speak(text: string, opts?: TtsSpeakOptions): Promise<void> {
    return _api.speak({ text, opts: toSpeakOpts(opts) }) as Promise<void>;
  },
  /** Stop any in-progress utterance and clear the queue. */
  stop(): Promise<void> {
    return _api.stop() as Promise<void>;
  },
  /** Whether an utterance is currently being spoken. */
  isSpeaking(): Promise<boolean> {
    return _api.isSpeaking() as Promise<unknown> as Promise<boolean>;
  },
});

export default tts;
