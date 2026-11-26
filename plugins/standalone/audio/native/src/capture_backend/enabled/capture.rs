use std::{
    cell::RefCell,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Sample, SampleFormat, Stream, StreamConfig, SupportedStreamConfig,
};
use tynd_plugin_sdk::{sanitize_event_string, EmitFn, PluginError, PluginResult};

use super::device::pick_supported_config;
use crate::CaptureOpts;
thread_local! {
    /// Reused per-callback float scratch — avoids allocating a fresh
    /// Vec<f32> on every `data` chunk (~100 Hz × 4 allocs without it).
    static FLOAT_SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    /// Reused base64 output buffer.
    static B64_SCRATCH: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(crate) struct Devices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

pub(crate) fn list_devices() -> PluginResult<Devices> {
    let host = cpal::default_host();
    let inputs = host
        .input_devices()
        .map_err(|e| PluginError::Custom(format!("audio.listDevices(inputs): {e}")))?
        .filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned()))
        .collect();
    let outputs = host
        .output_devices()
        .map_err(|e| PluginError::Custom(format!("audio.listDevices(outputs): {e}")))?
        .filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned()))
        .collect();
    Ok(Devices { inputs, outputs })
}

/// Resolve an input device by name, falling back to host default.
fn pick_input_device(name: Option<&str>) -> PluginResult<Device> {
    let host = cpal::default_host();
    if let Some(want) = name {
        let mut devices = host
            .input_devices()
            .map_err(|e| PluginError::Custom(format!("audio.startCapture(input_devices): {e}")))?;
        return devices
            .find(|d| d.description().is_ok_and(|desc| desc.name() == want))
            .ok_or_else(|| {
                PluginError::Custom(format!(
                    "audio.startCapture: input device '{want}' not found"
                ))
            });
    }
    host.default_input_device()
        .ok_or_else(|| PluginError::Custom("audio.startCapture: no default input device".into()))
}

fn negotiate_input_config(
    device: &Device,
    want_rate: Option<u32>,
    want_channels: Option<u16>,
) -> PluginResult<SupportedStreamConfig> {
    let default = device
        .default_input_config()
        .map_err(|e| PluginError::Custom(format!("audio.startCapture(default config): {e}")))?;
    let target_rate = want_rate.unwrap_or_else(|| default.sample_rate());
    let target_channels = want_channels.unwrap_or_else(|| default.channels());
    let configs = device
        .supported_input_configs()
        .map_err(|e| PluginError::Custom(format!("audio.startCapture(supported configs): {e}")))?;
    Ok(pick_supported_config(configs, target_rate, target_channels).unwrap_or(default))
}

pub(super) fn build_capture_stream(
    id: u64,
    opts: &CaptureOpts,
    emit: EmitFn,
) -> PluginResult<Stream> {
    let device = pick_input_device(opts.device.as_deref())?;
    let supported = negotiate_input_config(&device, opts.sample_rate, opts.channels)?;
    let actual_rate = supported.sample_rate();
    if let Some(want) = opts.sample_rate {
        if want != actual_rate {
            emit(
                "audio:warn",
                serde_json::json!({
                    "id": id,
                    "kind": "sample-rate-fallback",
                    "requested": want,
                    "actual": actual_rate,
                }),
            );
        }
    }
    if let Some(want) = opts.channels {
        if want != supported.channels() {
            emit(
                "audio:warn",
                serde_json::json!({
                    "id": id,
                    "kind": "channel-fallback",
                    "requested": want,
                    "actual": supported.channels(),
                }),
            );
        }
    }

    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let err_emit = emit.clone();
    let err_id = id;
    let err_fn = move |e: cpal::Error| {
        err_emit(
            "audio:error",
            serde_json::json!({
                "id": err_id,
                "kind": "stream-error",
                "message": sanitize_event_string(&e.to_string()),
            }),
        );
    };

    let stream = match format {
        SampleFormat::F32 => build_input_stream::<f32>(&device, &config, id, emit, err_fn),
        SampleFormat::I16 => build_input_stream::<i16>(&device, &config, id, emit, err_fn),
        SampleFormat::U16 => build_input_stream::<u16>(&device, &config, id, emit, err_fn),
        SampleFormat::I8 => build_input_stream::<i8>(&device, &config, id, emit, err_fn),
        SampleFormat::I32 => build_input_stream::<i32>(&device, &config, id, emit, err_fn),
        SampleFormat::I64 => build_input_stream::<i64>(&device, &config, id, emit, err_fn),
        SampleFormat::U8 => build_input_stream::<u8>(&device, &config, id, emit, err_fn),
        SampleFormat::U32 => build_input_stream::<u32>(&device, &config, id, emit, err_fn),
        SampleFormat::U64 => build_input_stream::<u64>(&device, &config, id, emit, err_fn),
        SampleFormat::F64 => build_input_stream::<f64>(&device, &config, id, emit, err_fn),
        other => Err(PluginError::Custom(format!(
            "audio.startCapture: unsupported sample format {other:?}"
        )))?,
    }?;

    stream
        .play()
        .map_err(|e| PluginError::Custom(format!("audio.startCapture(play): {e}")))?;
    Ok(stream)
}

fn build_input_stream<S>(
    device: &Device,
    config: &StreamConfig,
    id: u64,
    emit: EmitFn,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> PluginResult<Stream>
where
    S: Sample + cpal::SizedSample + Send + 'static,
    f32: FromSample<S>,
{
    let cb_emit = emit;
    device
        .build_input_stream(
            *config,
            move |data: &[S], _info: &cpal::InputCallbackInfo| {
                FLOAT_SCRATCH.with(|cell| {
                    let mut floats = cell.borrow_mut();
                    floats.clear();
                    floats.reserve(data.len());
                    for &s in data {
                        floats.push(f32::from_sample(s));
                    }

                    // Reinterpret f32 lanes as host-endian bytes.
                    // The top-of-crate `compile_error!` rejects BE
                    // targets so this cast is always LE; the cfg
                    // gate documents the dependency at the call
                    // site.
                    #[cfg(target_endian = "little")]
                    let bytes: &[u8] = bytemuck::cast_slice(&floats[..]);

                    B64_SCRATCH.with(|b64_cell| {
                        let mut b64 = b64_cell.borrow_mut();
                        b64.clear();
                        STANDARD.encode_string(bytes, &mut b64);
                        let timestamp = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_or(0, |d| d.as_millis() as u64);
                        let mut payload = serde_json::Map::with_capacity(3);
                        payload.insert("id".into(), serde_json::Value::from(id));
                        payload.insert("samples".into(), serde_json::Value::String(b64.clone()));
                        payload.insert("timestamp".into(), serde_json::Value::from(timestamp));
                        cb_emit("audio:samples", serde_json::Value::Object(payload));
                    });
                });
            },
            err_fn,
            None,
        )
        .map_err(|e| PluginError::Custom(format!("audio.startCapture(build_input_stream): {e}")))
}
