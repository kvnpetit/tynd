// All fields optional; missing values inherit platform defaults.
export type CaptureOpts = {
  sampleRate: number | null;
  channels: number | null;
  format: string | null;
  device: string | null;
};

// List every input + output device the OS audio host advertises.
export type ListDevicesArgs = null;

// Listed input/output device names exposed by the OS audio host.
export type ListDevicesResult = {
  inputs: string[];
  outputs: string[];
};

/**
 *  Decode `path` (any cpal-supported WAV variant) and play it back.
 *  Returns the playback id + an estimate of total duration.
 */
export type PlayFileArgs = {
  path: string;
  opts: PlayFileOpts | null;
};

export type PlayFileOpts = {
  volume: number | null;
  deviceId: string | null;
};

// Playback handle + duration estimate for `playFile`.
export type PlayFileResult = {
  id: number;
  durationMs: number;
};

// Play raw interleaved f32 samples through the default output device.
export type PlaySamplesArgs = {
  samples: number[];
  opts: PlaySamplesOpts;
};

export type PlaySamplesOpts = {
  sampleRate: number;
  channels: number;
  volume: number | null;
};

// Playback handle for `playSamples`.
export type PlaySamplesResult = {
  id: number;
};

// Write `samples` (interleaved f32) to `path` as a 32-bit float WAV.
export type SaveWavArgs = {
  samples: number[];
  path: string;
  opts: SaveWavOpts;
};

export type SaveWavOpts = {
  sampleRate: number;
  channels: number;
};

// Open an input stream and start emitting `audio:samples` events.
export type StartCaptureArgs = {
  opts: CaptureOpts | null;
};

// Capture handle id; userland feeds it back into `stopCapture`.
export type StartCaptureResult = {
  id: number;
};

// Idempotent — stopping an unknown id is a no-op.
export type StopCaptureArgs = {
  id: number;
};

// Idempotent — stopping an unknown id is a no-op.
export type StopPlaybackArgs = {
  id: number;
};

// === Result aliases (auto from #[command] return types) ===
export type StopCaptureResult = null;
export type StopPlaybackResult = null;
export type SaveWavResult = null;

// === Command catalog (typed client catalog) ===
export const _commands = {
  listDevices: {
    args: undefined as unknown as ListDevicesArgs,
    result: undefined as unknown as ListDevicesResult,
  },
  startCapture: {
    args: undefined as unknown as StartCaptureArgs,
    result: undefined as unknown as StartCaptureResult,
  },
  stopCapture: {
    args: undefined as unknown as StopCaptureArgs,
    result: undefined as unknown as StopCaptureResult,
  },
  playFile: {
    args: undefined as unknown as PlayFileArgs,
    result: undefined as unknown as PlayFileResult,
  },
  playSamples: {
    args: undefined as unknown as PlaySamplesArgs,
    result: undefined as unknown as PlaySamplesResult,
  },
  stopPlayback: {
    args: undefined as unknown as StopPlaybackArgs,
    result: undefined as unknown as StopPlaybackResult,
  },
  saveWav: {
    args: undefined as unknown as SaveWavArgs,
    result: undefined as unknown as SaveWavResult,
  },
} as const;
export type CommandsCatalog = typeof _commands;
