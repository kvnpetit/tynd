// `Mjpeg` always available; `H264` / `Hevc` need `gpu-encode`.
export type Codec = "mjpeg" | "h264" | "hevc";

// `Auto` walks NVENC → VideoToolbox → QSV → VA-API → libx264.
export type EncoderChoice = "auto" | "nvenc" | "videotoolbox" | "qsv" | "vaapi" | "x264" | "x265";

export type HasPermissionArgs = null;

export type ListArgs = null;

export type ListSourcesArgs = null;

export type MonitorInfo = {
  id: number;
  name: string;
  width: number;
  height: number;
  primary: boolean;
};

export type PauseArgs = {
  recordingId: number;
};

/**
 *  Maps onto the ffmpeg `-preset` family (normalised per encoder in
 *  `h264_backend`).
 */
export type Preset = "ultrafast" | "fast" | "medium" | "slow";

// `id` rendered as JS `number`; `startedAt` is unix epoch ms.
export type RecordingHandle = {
  id: number;
  startedAt: number;
  output: string;
};

// `output` is the only required field; bitrate/preset ignored for MJPEG.
export type RecordingOpts = {
  fps: number;
  region: Region | null;
  monitor: number | null;
  audio: boolean;
  // Output container; defaults to the codec's natural extension.
  format: string | null;
  output: string;
  codec: Codec;
  encoder: EncoderChoice;
  // kbps; default 8000.
  bitrate: number;
  preset: Preset;
};

export type RecordingReport = {
  durationMs: number;
  sizeBytes: number;
};

// Capture region in physical screen pixels. Origin top-left.
export type Region = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type RequestPermissionArgs = null;

export type ResumeArgs = {
  recordingId: number;
};

export type Sources = {
  monitors: MonitorInfo[];
  windows: WindowInfo[];
};

export type StartArgs = {
  opts: RecordingOpts;
};

export type StopArgs = {
  recordingId: number;
};

export type WindowInfo = {
  id: number;
  title: string;
  appName: string;
};

// === Result aliases (auto from #[command] return types) ===
export type ListSourcesResult = Sources;
export type HasPermissionResult = boolean;
export type RequestPermissionResult = boolean;
export type StartResult = RecordingHandle;
export type StopResult = RecordingReport;
export type PauseResult = null;
export type ResumeResult = null;
export type ListResult = RecordingHandle[];

// === Command catalog (typed client catalog) ===
export const _commands = {
  listSources: {
    args: undefined as unknown as ListSourcesArgs,
    result: undefined as unknown as ListSourcesResult,
  },
  hasPermission: {
    args: undefined as unknown as HasPermissionArgs,
    result: undefined as unknown as HasPermissionResult,
  },
  requestPermission: {
    args: undefined as unknown as RequestPermissionArgs,
    result: undefined as unknown as RequestPermissionResult,
  },
  start: { args: undefined as unknown as StartArgs, result: undefined as unknown as StartResult },
  stop: { args: undefined as unknown as StopArgs, result: undefined as unknown as StopResult },
  pause: { args: undefined as unknown as PauseArgs, result: undefined as unknown as PauseResult },
  resume: {
    args: undefined as unknown as ResumeArgs,
    result: undefined as unknown as ResumeResult,
  },
  list: { args: undefined as unknown as ListArgs, result: undefined as unknown as ListResult },
} as const;
export type CommandsCatalog = typeof _commands;
