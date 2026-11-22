use std::{
    mem::size_of,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Sample, SampleFormat, Stream, StreamConfig, SupportedStreamConfig,
};
use crossbeam_channel::Sender;
use tynd_plugin_sdk::{EmitFn, PluginError, PluginResult};

use super::{
    control::{start_playback, PlaybackRequest},
    device::pick_supported_config,
};
use crate::{PlayFileOpts, PlaySamplesOpts, PlaybackSlot};
/// Hard cap on a `playFile` WAV decode. The decoder allocates
/// `Vec<f32>` for the entire file up-front (so we can return an
/// exact duration before the playback starts); a 256 MiB cap stops
/// a hostile / mistakenly-pointed file from OOM-ing the host.
const MAX_WAV_BYTES: u64 = 256 * 1024 * 1024;

struct DecodedWav {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
    duration_ms: u64,
}

#[derive(Clone)]
struct PlaybackCompletion {
    id: u64,
    sender: Sender<u64>,
    sent: Arc<AtomicBool>,
}

impl PlaybackCompletion {
    fn new(id: u64, sender: Sender<u64>) -> Self {
        Self {
            id,
            sender,
            sent: Arc::new(AtomicBool::new(false)),
        }
    }

    fn notify(&self) {
        if self
            .sent
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = self.sender.send(self.id);
        }
    }
}

pub(crate) fn play_file(
    id: u64,
    path: &str,
    opts: PlayFileOpts,
    _emit: EmitFn,
    slot: PlaybackSlot,
) -> PluginResult<(u64, u64)> {
    let decoded = decode_wav(path)?;
    let duration_ms = decoded.duration_ms;
    start_playback(
        PlaybackRequest {
            id,
            samples: decoded.samples,
            sample_rate: decoded.sample_rate,
            channels: decoded.channels,
            volume: normalize_volume(opts.volume),
            device: opts.device_id,
            slot,
        },
        "audio.playFile",
    )?;
    Ok((id, duration_ms))
}

fn decode_wav(path: &str) -> PluginResult<DecodedWav> {
    // Bounded decode: refuse > `MAX_WAV_BYTES` before allocating a
    // `Vec<f32>` for the entire file. Open the file *once* and read
    // metadata + length from the live handle so a swap between stat
    // and open cannot bypass the cap (TOCTOU).
    let file = std::fs::File::open(Path::new(path))
        .map_err(|error| PluginError::Custom(format!("audio.playFile(open '{path}'): {error}")))?;
    let file_meta = file
        .metadata()
        .map_err(|error| PluginError::Custom(format!("audio.playFile(stat '{path}'): {error}")))?;
    if file_meta.len() > MAX_WAV_BYTES {
        return Err(PluginError::InvalidArg {
            key: "path".into(),
            detail: format!(
                "audio.playFile: WAV file '{path}' is {} bytes, max {MAX_WAV_BYTES} bytes",
                file_meta.len()
            ),
        });
    }

    // Decode upfront so duration is exact and file I/O stays off the
    // control thread.
    let mut reader = hound::WavReader::new(std::io::BufReader::new(file))
        .map_err(|error| PluginError::Custom(format!("audio.playFile(parse '{path}'): {error}")))?;
    let spec = reader.spec();
    let projected_bytes = (reader.duration() as u64)
        .saturating_mul(u64::from(spec.channels))
        .saturating_mul(size_of::<f32>() as u64);
    if projected_bytes > MAX_WAV_BYTES {
        return Err(PluginError::InvalidArg {
            key: "path".into(),
            detail: format!(
                "audio.playFile: WAV '{path}' decodes to {projected_bytes} bytes, max \
                 {MAX_WAV_BYTES}"
            ),
        });
    }

    let samples = read_wav_samples(&mut reader).map_err(|error| {
        PluginError::Custom(format!("audio.playFile(decode '{path}'): {error}"))
    })?;
    let duration_ms = wav_duration_ms(samples.len() as u64, spec.sample_rate, spec.channels);
    Ok(DecodedWav {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        duration_ms,
    })
}

fn wav_duration_ms(total_samples: u64, sample_rate: u32, channels: u16) -> u64 {
    let frames = if channels == 0 {
        0
    } else {
        total_samples / u64::from(channels)
    };
    if sample_rate == 0 {
        0
    } else {
        (frames * 1000) / u64::from(sample_rate)
    }
}

fn normalize_volume(volume: Option<f32>) -> f32 {
    // `f32::clamp(NaN, …)` returns NaN. Keep non-finite input away
    // from callbacks and integer backends.
    let raw = volume.unwrap_or(1.0);
    if raw.is_finite() {
        raw.clamp(0.0, 4.0)
    } else {
        1.0
    }
}

pub(crate) fn play_samples(
    id: u64,
    samples: Vec<f32>,
    opts: PlaySamplesOpts,
    _emit: EmitFn,
    slot: PlaybackSlot,
) -> PluginResult<u64> {
    start_playback(
        PlaybackRequest {
            id,
            samples,
            sample_rate: opts.sample_rate,
            channels: opts.channels,
            volume: normalize_volume(opts.volume),
            device: None,
            slot,
        },
        "audio.playSamples",
    )?;
    Ok(id)
}
fn pick_output_device(name: Option<&str>) -> PluginResult<Device> {
    let host = cpal::default_host();
    if let Some(want) = name {
        let mut devices = host
            .output_devices()
            .map_err(|e| PluginError::Custom(format!("audio.playback(output_devices): {e}")))?;
        return devices
            .find(|d| d.description().is_ok_and(|desc| desc.name() == want))
            .ok_or_else(|| {
                PluginError::Custom(format!("audio.playback: output device '{want}' not found"))
            });
    }
    host.default_output_device()
        .ok_or_else(|| PluginError::Custom("audio.playback: no default output device".into()))
}

fn negotiate_output_config(
    device: &Device,
    want_rate: u32,
    want_channels: u16,
) -> PluginResult<SupportedStreamConfig> {
    if want_rate == 0 || want_channels == 0 {
        return Err(PluginError::InvalidArg {
            key: "opts".into(),
            detail: "audio.playback: sampleRate and channels must be greater than zero".into(),
        });
    }
    let configs = device
        .supported_output_configs()
        .map_err(|e| PluginError::Custom(format!("audio.playback(supported configs): {e}")))?;
    pick_supported_config(configs, want_rate, want_channels)
        .filter(|config| config.sample_rate() == want_rate && config.channels() == want_channels)
        .ok_or_else(|| PluginError::InvalidArg {
            key: "opts".into(),
            detail: format!(
                "audio.playback: output device does not support exact {want_rate} Hz / \
                 {want_channels} channel configuration; resample or remix before playback"
            ),
        })
}

pub(super) fn build_playback_stream(
    id: u64,
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    channels: u16,
    volume: f32,
    device_name: Option<String>,
    finished: Sender<u64>,
) -> PluginResult<Stream> {
    let device = pick_output_device(device_name.as_deref())?;
    let supported = negotiate_output_config(&device, sample_rate, channels)?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();

    let completion = PlaybackCompletion::new(id, finished);
    let error_completion = completion.clone();
    let err_fn = move |e: cpal::Error| {
        tracing::error!(error = %e, "audio.playback stream error");
        error_completion.notify();
    };

    let stream = match format {
        SampleFormat::F32 => {
            build_output_stream::<f32>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::I16 => {
            build_output_stream::<i16>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::U16 => {
            build_output_stream::<u16>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::I8 => {
            build_output_stream::<i8>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::I32 => {
            build_output_stream::<i32>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::I64 => {
            build_output_stream::<i64>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::U8 => {
            build_output_stream::<u8>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::U32 => {
            build_output_stream::<u32>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::U64 => {
            build_output_stream::<u64>(&device, &config, samples, volume, completion, err_fn)
        },
        SampleFormat::F64 => {
            build_output_stream::<f64>(&device, &config, samples, volume, completion, err_fn)
        },
        other => Err(PluginError::Custom(format!(
            "audio.playback: unsupported sample format {other:?}"
        )))?,
    }?;

    stream
        .play()
        .map_err(|e| PluginError::Custom(format!("audio.playback(play): {e}")))?;
    Ok(stream)
}

fn build_output_stream<S>(
    device: &Device,
    config: &StreamConfig,
    samples: Arc<Vec<f32>>,
    volume: f32,
    completion: PlaybackCompletion,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> PluginResult<Stream>
where
    S: Sample + cpal::SizedSample + FromSample<f32> + Send + 'static,
{
    // cpal callbacks are `FnMut` and serialised per stream — the
    // cursor only needs to live across invocations, not across
    // threads. A captured `usize` drops the per-callback `Mutex`
    // lock/unlock that previously sat on the real-time audio path.
    let mut idx: usize = 0;
    device
        .build_output_stream(
            *config,
            move |data: &mut [S], _info: &cpal::OutputCallbackInfo| {
                // Do not drop the stream from the same callback that copied
                // the final samples. The device has only accepted that
                // buffer; it has not necessarily rendered it yet. A second
                // callback proves the previous buffer was consumed.
                let drained_before_callback = idx >= samples.len();
                for slot in data.iter_mut() {
                    if idx < samples.len() {
                        let raw = samples[idx] * volume;
                        // NaN-laced input would clamp to NaN and
                        // emit raw NaN bytes to the device — flatten
                        // to silence instead.
                        let v = if raw.is_finite() {
                            raw.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        };
                        *slot = S::from_sample(v);
                        idx += 1;
                    } else {
                        *slot = S::EQUILIBRIUM;
                    }
                }
                if drained_before_callback {
                    completion.notify();
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| PluginError::Custom(format!("audio.playback(build_output_stream): {e}")))
}

/// Decode any WAV (int or float, any bit depth) into interleaved f32
/// in the [-1.0, 1.0] range.
fn read_wav_samples(
    reader: &mut hound::WavReader<std::io::BufReader<std::fs::File>>,
) -> hound::Result<Vec<f32>> {
    let spec = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>(),
        hound::SampleFormat::Int => {
            let scale = match spec.bits_per_sample {
                8 => i32::from(i8::MAX) as f32,
                16 => i32::from(i16::MAX) as f32,
                24 => 8_388_607.0_f32,
                32 => i32::MAX as f32,
                _ => return Err(hound::Error::Unsupported),
            };
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_clamped_and_non_finite_defaults_to_unity() {
        assert!((normalize_volume(None) - 1.0).abs() < f32::EPSILON);
        assert!((normalize_volume(Some(-3.0))).abs() < f32::EPSILON);
        assert!((normalize_volume(Some(99.0)) - 4.0).abs() < f32::EPSILON);
        assert!((normalize_volume(Some(f32::NAN)) - 1.0).abs() < f32::EPSILON);
        assert!((normalize_volume(Some(f32::INFINITY)) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn wav_duration_handles_degenerate_formats() {
        assert_eq!(wav_duration_ms(88_200, 44_100, 2), 1000);
        assert_eq!(wav_duration_ms(44_100, 44_100, 1), 1000);
        assert_eq!(wav_duration_ms(100, 0, 2), 0);
        assert_eq!(wav_duration_ms(100, 44_100, 0), 0);
        assert_eq!(wav_duration_ms(0, 44_100, 2), 0);
    }
}
