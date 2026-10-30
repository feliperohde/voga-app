//! Sound in: a microphone (cpal) and the computer's sound (macOS: a Core Audio process tap,
//! from Assunta's audio/capture/core_audio.rs, macOS 14.2+; Windows: WASAPI loopback). Each
//! source's callback pushes mono samples into a lock-free buffer; `Capture::read`, on the
//! session's audio thread, takes them, converts them to 16 kHz and mixes them into one stream.
//!
//! The streams are not Send on every platform: a Capture lives on the thread that opened it.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use rubato::{FftFixedIn, Resampler};
use serde::Serialize;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use super::SAMPLE_RATE;

/// Samples a source may hold before the reader takes them (a few seconds at 48 kHz).
const BUFFER_SAMPLES: usize = 48_000 * 4;
/// A source that stops sending does not hold the other back by more than this (16 kHz samples).
const MAX_LAG: usize = SAMPLE_RATE as usize / 2;
/// Input frames per resampling step.
const RESAMPLE_CHUNK: usize = 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphone {
    pub name: String,
    pub is_default: bool,
}

pub fn microphones() -> Vec<Microphone> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    devices
        .filter_map(|d| d.name().ok())
        .map(|name| Microphone { is_default: default.as_deref() == Some(name.as_str()), name })
        .collect()
}

/// Whether this computer can hear its own sound (macOS 14.2+ or Windows).
pub fn system_audio_available() -> bool {
    if cfg!(windows) {
        return true;
    }
    if !cfg!(target_os = "macos") {
        return false;
    }
    let version = sysinfo::System::os_version().unwrap_or_default();
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (major, minor) >= (14, 2)
}

/// Converts one source's mono sound to 16 kHz.
struct To16k {
    resampler: Option<FftFixedIn<f32>>,
    input: Vec<f32>,
}

impl To16k {
    fn new(rate: u32) -> Result<Self> {
        let resampler = if rate == SAMPLE_RATE {
            None
        } else {
            Some(FftFixedIn::new(rate as usize, SAMPLE_RATE as usize, RESAMPLE_CHUNK, 2, 1).map_err(|e| anyhow!("Resampler for {rate} Hz: {e}"))?)
        };
        Ok(Self { resampler, input: Vec::new() })
    }

    fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        let Some(resampler) = self.resampler.as_mut() else {
            out.extend_from_slice(samples);
            return;
        };
        self.input.extend_from_slice(samples);
        loop {
            let needed = resampler.input_frames_next();
            if self.input.len() < needed {
                break;
            }
            let chunk: Vec<f32> = self.input.drain(..needed).collect();
            match resampler.process(&[chunk], None) {
                Ok(mut converted) => out.append(&mut converted[0]),
                Err(e) => log::warn!("Live: resampling failed: {e}"),
            }
        }
    }
}

/// One source: what its callback pushed, at its own rate, and its sound at 16 kHz.
struct Feed {
    name: &'static str,
    consumer: HeapCons<f32>,
    /// The source's current rate (the computer's sound can change it while running).
    rate: Arc<AtomicU32>,
    converting_from: u32,
    to16k: To16k,
    pending: Vec<f32>,
}

impl Feed {
    fn new(name: &'static str, consumer: HeapCons<f32>, rate: Arc<AtomicU32>) -> Result<Self> {
        let from = rate.load(Ordering::Acquire);
        Ok(Self { name, consumer, rate, converting_from: from, to16k: To16k::new(from)?, pending: Vec::new() })
    }

    fn take(&mut self) {
        let rate = self.rate.load(Ordering::Acquire);
        if rate != self.converting_from && rate > 0 {
            log::info!("Live: {} now at {rate} Hz", self.name);
            match To16k::new(rate) {
                Ok(to16k) => {
                    self.to16k = to16k;
                    self.converting_from = rate;
                }
                Err(e) => log::warn!("Live: {e}"),
            }
        }
        let mut raw = vec![0.0f32; self.consumer.occupied_len()];
        let n = self.consumer.pop_slice(&mut raw);
        raw.truncate(n);
        self.to16k.push(&raw, &mut self.pending);
    }
}

/// The open sources, mixed. Keep it on the thread that opened it.
pub struct Capture {
    feeds: Vec<Feed>,
    _streams: Vec<cpal::Stream>,
    #[cfg(target_os = "macos")]
    _tap: Option<macos::SystemTap>,
}

impl Capture {
    /// `microphone`: a device name ("" for the default one), or None; `system`: the computer's sound.
    pub fn open(microphone: Option<&str>, system: bool) -> Result<Self> {
        let mut feeds = Vec::new();
        let mut streams = Vec::new();
        if let Some(name) = microphone {
            let host = cpal::default_host();
            let device = if name.is_empty() {
                host.default_input_device()
            } else {
                host.input_devices()?.find(|d| d.name().is_ok_and(|n| n == name)).or_else(|| host.default_input_device())
            }
            .ok_or_else(|| anyhow!("No microphone found"))?;
            let config = device.default_input_config().context("The microphone gave no format")?;
            let (stream, feed) = cpal_feed("microphone", &device, config)?;
            streams.push(stream);
            feeds.push(feed);
        }
        #[cfg(target_os = "macos")]
        let mut tap = None;
        if system {
            if !system_audio_available() {
                return Err(anyhow!("This computer cannot capture its own sound (macOS 14.2 or later, or Windows)"));
            }
            #[cfg(target_os = "macos")]
            {
                let (system_tap, feed) = macos::SystemTap::open()?;
                tap = Some(system_tap);
                feeds.push(feed);
            }
            #[cfg(windows)]
            {
                // WASAPI loopback: an input stream on the output device hears what it plays
                let device = cpal::default_host().default_output_device().ok_or_else(|| anyhow!("No sound output found"))?;
                let config = device.default_output_config().context("The sound output gave no format")?;
                let (stream, feed) = cpal_feed("system", &device, config)?;
                streams.push(stream);
                feeds.push(feed);
            }
        }
        if feeds.is_empty() {
            return Err(anyhow!("Choose a microphone or the computer's sound"));
        }
        Ok(Self {
            feeds,
            _streams: streams,
            #[cfg(target_os = "macos")]
            _tap: tap,
        })
    }

    /// The mixed 16 kHz sound heard since the last call.
    pub fn read(&mut self) -> Vec<f32> {
        for feed in &mut self.feeds {
            feed.take();
        }
        let longest = self.feeds.iter().map(|f| f.pending.len()).max().unwrap_or(0);
        let shortest = self.feeds.iter().map(|f| f.pending.len()).min().unwrap_or(0);
        let n = shortest.max(longest.saturating_sub(MAX_LAG));
        let mut mixed = vec![0.0f32; n];
        for feed in &mut self.feeds {
            let take = n.min(feed.pending.len());
            for (m, s) in mixed.iter_mut().zip(feed.pending.drain(..take)) {
                *m += s;
            }
        }
        if self.feeds.len() > 1 {
            for m in &mut mixed {
                *m = m.clamp(-1.0, 1.0);
            }
        }
        mixed
    }
}

/// A cpal input stream (a microphone, or a Windows output in loopback) pushing mono samples.
fn cpal_feed(name: &'static str, device: &cpal::Device, config: cpal::SupportedStreamConfig) -> Result<(cpal::Stream, Feed)> {
    let (producer, consumer) = HeapRb::<f32>::new(BUFFER_SAMPLES).split();
    let channels = config.channels() as usize;
    let rate = Arc::new(AtomicU32::new(config.sample_rate().0));
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let on_error = move |e: cpal::StreamError| log::warn!("Live: {name} stream error: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(&config, mono_pusher::<f32>(producer, channels), on_error, None),
        cpal::SampleFormat::I16 => device.build_input_stream(&config, mono_pusher::<i16>(producer, channels), on_error, None),
        cpal::SampleFormat::I32 => device.build_input_stream(&config, mono_pusher::<i32>(producer, channels), on_error, None),
        cpal::SampleFormat::U16 => device.build_input_stream(&config, mono_pusher::<u16>(producer, channels), on_error, None),
        other => return Err(anyhow!("Unsupported sample format {other:?} for the {name}")),
    }
    .with_context(|| format!("Could not open the {name}"))?;
    stream.play().with_context(|| format!("Could not start the {name}"))?;
    log::info!("Live: {name} open at {} Hz, {channels} channel(s)", config.sample_rate.0);
    Ok((stream, Feed::new(name, consumer, rate)?))
}

/// The callback of a cpal stream: averages the channels and pushes; drops what does not fit.
fn mono_pusher<T>(mut producer: HeapProd<f32>, channels: usize) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static
where
    T: cpal::Sample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let mut mono: Vec<f32> = Vec::new();
    move |data: &[T], _: &cpal::InputCallbackInfo| {
        mono.clear();
        for frame in data.chunks(channels.max(1)) {
            let sum: f32 = frame.iter().map(|&s| <f32 as cpal::FromSample<T>>::from_sample_(s)).sum();
            mono.push(sum / frame.len() as f32);
        }
        producer.push_slice(&mono);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! The computer's sound on macOS: a global process tap in a private aggregate device
    //! (Assunta's CoreAudioCapture, without the async stream: the session's thread reads it).

    use super::{Feed, BUFFER_SAMPLES};
    use anyhow::{anyhow, Result};
    use cidre::{arc, av, cat, cf, core_audio as ca, os};
    use ringbuf::traits::{Producer, Split};
    use ringbuf::{HeapProd, HeapRb};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct TapContext {
        format: arc::R<av::AudioFormat>,
        producer: HeapProd<f32>,
        rate: Arc<AtomicU32>,
    }

    pub struct SystemTap {
        // Dropped in this order: the device stops before its context goes
        _device: ca::hardware::StartedDevice<ca::AggregateDevice>,
        _ctx: Box<TapContext>,
        _tap: ca::TapGuard,
    }

    impl SystemTap {
        pub fn open() -> Result<(Self, Feed)> {
            let output = ca::System::default_output_device().map_err(|e| anyhow!("No sound output: {e:?}"))?;
            let output_uid = output.uid().map_err(|e| anyhow!("Sound output without an id: {e:?}"))?;
            // Mono, every process. The permission prompt (NSAudioCaptureUsageDescription) shows
            // here; denied, the tap only hears silence.
            let desc = ca::TapDesc::with_mono_global_tap_excluding_processes(&cidre::ns::Array::new());
            let tap = desc.create_process_tap().map_err(|e| anyhow!("Could not tap the computer's sound: {e:?}"))?;
            let asbd = tap.asbd().map_err(|e| anyhow!("The tap gave no format: {e:?}"))?;
            let format = av::AudioFormat::with_asbd(&asbd).ok_or_else(|| anyhow!("Unsupported tap format"))?;

            // A tap is recorded through a private aggregate device clocked by the output device.
            // Its only member is the tap: with the output device as one too, everything plays twice.
            let tap_uid = tap.uid().map_err(|e| anyhow!("Tap without an id: {e:?}"))?;
            let member = cf::DictionaryOf::with_keys_values(&[ca::sub_device_keys::uid()], &[tap_uid.as_type_ref()]);
            let taps = cf::ArrayOf::from_slice(&[member.as_ref()]);
            let device_uid = cf::Uuid::new().to_cf_string();
            let settings: [(&cf::String, &cf::Type); 7] = [
                (ca::aggregate_device_keys::uid(), &device_uid),
                (ca::aggregate_device_keys::name(), cf::str!(c"Voga live").as_type_ref()),
                (ca::aggregate_device_keys::main_sub_device(), &output_uid),
                (ca::aggregate_device_keys::tap_list(), &taps),
                (ca::aggregate_device_keys::tap_auto_start(), cf::Boolean::value_true()),
                (ca::aggregate_device_keys::is_private(), cf::Boolean::value_true()),
                (ca::aggregate_device_keys::is_stacked(), cf::Boolean::value_false()),
            ];
            let agg_desc = cf::DictionaryOf::with_keys_values(&settings.map(|(key, _)| key), &settings.map(|(_, value)| value));

            let (producer, consumer) = HeapRb::<f32>::new(BUFFER_SAMPLES).split();
            let rate = Arc::new(AtomicU32::new(asbd.sample_rate as u32));
            let mut ctx = Box::new(TapContext { format, producer, rate: rate.clone() });

            extern "C" fn io_proc(
                device: ca::Device,
                _now: &cat::AudioTimeStamp,
                input_data: &cat::AudioBufList<1>,
                _input_time: &cat::AudioTimeStamp,
                _output_data: &mut cat::AudioBufList<1>,
                _output_time: &cat::AudioTimeStamp,
                ctx: Option<&mut TapContext>,
            ) -> os::Status {
                let Some(ctx) = ctx else { return os::Status::NO_ERR };
                let rate = device.nominal_sample_rate().unwrap_or(ctx.format.absd().sample_rate) as u32;
                ctx.rate.store(rate, Ordering::Release);
                if let Some(view) = av::AudioPcmBuf::with_buf_list_no_copy(&ctx.format, input_data, None) {
                    if let Some(data) = view.data_f32_at(0) {
                        ctx.producer.push_slice(data);
                    }
                } else if ctx.format.common_format() == av::audio::CommonFormat::PcmF32 {
                    let buffer = &input_data.buffers[0];
                    let count = buffer.data_bytes_size as usize / std::mem::size_of::<f32>();
                    if count > 0 && !buffer.data.is_null() {
                        let data = unsafe { std::slice::from_raw_parts(buffer.data as *const f32, count) };
                        ctx.producer.push_slice(data);
                    }
                }
                os::Status::NO_ERR
            }

            let device = ca::AggregateDevice::with_desc(&agg_desc).map_err(|e| anyhow!("Could not make the tap device: {e:?}"))?;
            let proc_id = device.create_io_proc_id(io_proc, Some(ctx.as_mut())).map_err(|e| anyhow!("Could not read the tap device: {e:?}"))?;
            let started = ca::device_start(device, Some(proc_id)).map_err(|e| anyhow!("Could not start the tap device: {e:?}"))?;
            log::info!("Live: the computer's sound is tapped at {} Hz", asbd.sample_rate);
            let feed = Feed::new("system", consumer, rate)?;
            Ok((Self { _device: started, _ctx: ctx, _tap: tap }, feed))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_converts_to_16_khz() {
        let mut to16k = To16k::new(48_000).unwrap();
        let mut out = Vec::new();
        // One second at 48 kHz, in uneven blocks as callbacks deliver them
        let second: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48.0).sin() * 0.1).collect();
        for block in second.chunks(441) {
            to16k.push(block, &mut out);
        }
        // All but the last partial chunk and the filter's delay
        assert!(out.len() > 15_000 && out.len() <= 16_000, "{}", out.len());
    }

    #[test]
    fn a_stalled_source_holds_the_mix_back_only_so_long() {
        let feed = |name, samples: usize| {
            let (_producer, consumer) = HeapRb::<f32>::new(64).split();
            let mut feed = Feed::new(name, consumer, Arc::new(AtomicU32::new(SAMPLE_RATE))).unwrap();
            feed.pending = vec![0.25; samples];
            feed
        };
        let mut capture = Capture {
            feeds: vec![feed("microphone", 4_000), feed("system", 1_000)],
            _streams: Vec::new(),
            #[cfg(target_os = "macos")]
            _tap: None,
        };
        // Both have 1000: those are mixed
        let mixed = capture.read();
        assert_eq!(mixed.len(), 1_000);
        assert!((mixed[0] - 0.5).abs() < 1e-6);
        // The system sends nothing more: the microphone waits up to MAX_LAG, then goes alone
        capture.feeds[0].pending.extend(vec![0.25; MAX_LAG]);
        let alone = capture.read();
        assert_eq!(alone.len(), 3_000);
        assert!((alone[0] - 0.25).abs() < 1e-6);
    }
}
