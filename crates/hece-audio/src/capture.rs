//! Microphone capture. The device is opened by [`Capture::start`] and closed by
//! [`Capture::stop`] or [`Capture::cancel`]; nothing is held open while idle.
//!
//! Two lessons from Handy: some devices (Bluetooth, USB webcams) deliver the first samples
//! only after a delay, so [`CaptureEvent::Live`] reports when audio actually arrives and its
//! latency is measured; and the end of a sentence can be cut, so capture continues for a
//! short post-roll after stop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::AudioError;
use crate::resample::{downmix, to_16k};

/// Capture settings.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Input device name (as listed by [`input_devices`]) or id; `None` for the default.
    pub device: Option<String>,
    /// Recording limit; on reaching it [`CaptureEvent::LimitReached`] fires and later audio is
    /// dropped, but everything up to the limit is kept.
    pub max_duration: Duration,
    /// Extra capture after stop so the last word is not cut.
    pub post_roll: Duration,
}

impl Default for CaptureConfig {
    /// Default device, 10 min limit, 300 ms post-roll.
    fn default() -> Self {
        Self {
            device: None,
            max_duration: Duration::from_mins(10),
            post_roll: Duration::from_millis(300),
        }
    }
}

/// Things that happen during a capture. Delivered on the audio thread: the handler must be
/// quick (e.g. send on a channel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureEvent {
    /// The first audio arrived, `latency` after [`Capture::start`] was called.
    Live {
        /// Time from start to the first samples.
        latency: Duration,
    },
    /// The recording limit was reached; the caller should stop and process as usual.
    LimitReached,
    /// The stream reported an error (e.g. the device was unplugged).
    StreamError {
        /// Stable code.
        code: &'static str,
    },
}

/// A finished recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// 16 kHz mono samples.
    pub samples: Vec<i16>,
    /// Time from start to the first samples; `None` if no audio ever arrived.
    pub first_audio_latency: Option<Duration>,
    /// Whether the limit cut the recording.
    pub limit_reached: bool,
}

/// Names of the available input devices.
pub fn input_devices() -> Result<Vec<String>, AudioError> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| AudioError::from_cpal(&e))?;
    Ok(devices.map(|d| d.to_string()).collect())
}

fn find_device(name: Option<&str>) -> Result<cpal::Device, AudioError> {
    let host = cpal::default_host();
    let Some(wanted) = name else {
        return host.default_input_device().ok_or(AudioError::NoDevice);
    };
    let devices = host
        .input_devices()
        .map_err(|e| AudioError::from_cpal(&e))?;
    for device in devices {
        let id_matches = device.id().is_ok_and(|id| id.to_string() == wanted);
        if id_matches || device.to_string() == wanted {
            return Ok(device);
        }
    }
    Err(AudioError::DeviceNotFound)
}

/// Shared between the audio callback and the capture thread.
struct Shared {
    mono: Mutex<Vec<f32>>,
    limit_frames: usize,
    limit_hit: AtomicBool,
    live: AtomicBool,
    started: Instant,
    latency: Mutex<Option<Duration>>,
    events: Box<dyn Fn(CaptureEvent) + Send + Sync>,
}

impl Shared {
    fn push<T>(&self, data: &[T], channels: u16)
    where
        T: Sample,
        f32: FromSample<T>,
    {
        if data.is_empty() || self.limit_hit.load(Ordering::Relaxed) {
            return;
        }
        if !self.live.swap(true, Ordering::Relaxed) {
            let latency = self.started.elapsed();
            *self.latency.lock().unwrap_or_else(PoisonError::into_inner) = Some(latency);
            (self.events)(CaptureEvent::Live { latency });
        }
        let frames: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
        let mono = downmix(&frames, channels);
        let mut buf = self.mono.lock().unwrap_or_else(PoisonError::into_inner);
        let room = self.limit_frames.saturating_sub(buf.len());
        buf.extend_from_slice(&mono[..mono.len().min(room)]);
        if buf.len() >= self.limit_frames {
            self.limit_hit.store(true, Ordering::Relaxed);
            (self.events)(CaptureEvent::LimitReached);
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels;
    let data_shared = Arc::clone(shared);
    let err_shared = Arc::clone(shared);
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| data_shared.push(data, channels),
            move |e: cpal::Error| {
                if AudioError::is_transient(&e) {
                    tracing::debug!(kind = ?e.kind(), "transient audio stream event");
                    return;
                }
                let code = AudioError::from_cpal(&e).code();
                tracing::warn!(code, "audio stream error");
                (err_shared.events)(CaptureEvent::StreamError { code });
            },
            None,
        )
        .map_err(|e| AudioError::from_cpal(&e))
}

/// Starts the stream; runs on the capture thread because a `cpal::Stream` must stay on the
/// thread that created it on some platforms.
fn open(
    device: &cpal::Device,
    supported: cpal::SupportedStreamConfig,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, AudioError> {
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build_stream::<f32>(device, &config, shared),
        SampleFormat::I16 => build_stream::<i16>(device, &config, shared),
        SampleFormat::U16 => build_stream::<u16>(device, &config, shared),
        SampleFormat::I32 => build_stream::<i32>(device, &config, shared),
        SampleFormat::I8 => build_stream::<i8>(device, &config, shared),
        SampleFormat::U8 => build_stream::<u8>(device, &config, shared),
        SampleFormat::F64 => build_stream::<f64>(device, &config, shared),
        _ => Err(AudioError::UnsupportedFormat),
    }?;
    stream.play().map_err(|e| AudioError::from_cpal(&e))?;
    Ok(stream)
}

fn device_and_config(
    name: Option<&str>,
) -> Result<(cpal::Device, cpal::SupportedStreamConfig), AudioError> {
    let device = find_device(name)?;
    let config = device
        .default_input_config()
        .map_err(|e| AudioError::from_cpal(&e))?;
    Ok((device, config))
}

enum Command {
    Stop,
    Cancel,
}

/// A running capture.
pub struct Capture {
    commands: mpsc::Sender<Command>,
    /// The recording, sent before the device is closed (closing can take a few hundred ms).
    done: mpsc::Receiver<Result<Recording, AudioError>>,
    thread: Option<JoinHandle<Result<(), AudioError>>>,
    shared: Arc<Shared>,
    rate: u32,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture").finish_non_exhaustive()
    }
}

impl Capture {
    /// Opens the microphone and starts recording. Returns once the stream is running (audio may
    /// arrive later; see [`CaptureEvent::Live`]).
    pub fn start(
        cfg: &CaptureConfig,
        events: impl Fn(CaptureEvent) + Send + Sync + 'static,
    ) -> Result<Self, AudioError> {
        let started = Instant::now();
        let (commands, command_rx) = mpsc::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(Arc<Shared>, u32), AudioError>>(1);
        let (done_tx, done) = mpsc::sync_channel(1);
        let device = cfg.device.clone();
        let post_roll = cfg.post_roll;
        let max = cfg.max_duration;
        let events: Box<dyn Fn(CaptureEvent) + Send + Sync> = Box::new(events);
        let thread = std::thread::Builder::new()
            .name("hece-capture".to_owned())
            .spawn(move || {
                let opened = device_and_config(device.as_deref()).and_then(|(dev, supported)| {
                    let rate = supported.sample_rate();
                    let shared = Arc::new(Shared {
                        mono: Mutex::new(Vec::new()),
                        limit_frames: frames_for(max, rate),
                        limit_hit: AtomicBool::new(false),
                        live: AtomicBool::new(false),
                        started,
                        latency: Mutex::new(None),
                        events,
                    });
                    let stream = open(&dev, supported, &shared)?;
                    Ok((stream, rate, shared))
                });
                let (stream, rate, shared) = match opened {
                    Ok(opened) => opened,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.clone()));
                        return Err(e);
                    }
                };
                let _ = ready_tx.send(Ok((Arc::clone(&shared), rate)));
                let command = command_rx.recv().unwrap_or(Command::Cancel);
                if matches!(command, Command::Stop) {
                    if !shared.limit_hit.load(Ordering::Relaxed) {
                        std::thread::sleep(post_roll);
                    }
                    // Callbacks until the stream is dropped are ignored from here on.
                    let limit_reached = shared.limit_hit.swap(true, Ordering::Relaxed);
                    let mono = std::mem::take(
                        &mut *shared.mono.lock().unwrap_or_else(PoisonError::into_inner),
                    );
                    let first_audio_latency = *shared
                        .latency
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let recording = to_16k(&mono, rate).map(|samples| Recording {
                        samples,
                        first_audio_latency,
                        limit_reached,
                    });
                    let _ = done_tx.send(recording);
                }
                drop(stream);
                Ok(())
            })
            .map_err(|_| AudioError::Thread)?;
        match ready_rx.recv() {
            Ok(Ok((shared, rate))) => Ok(Self {
                commands,
                done,
                thread: Some(thread),
                shared,
                rate,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AudioError::Thread)
            }
        }
    }

    /// Sample rate of the device, i.e. of [`Capture::read_from`].
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Mono samples at the device rate recorded so far, from index `start` on (for chunking
    /// while recording).
    #[must_use]
    pub fn read_from(&self, start: usize) -> Vec<f32> {
        let mono = self
            .shared
            .mono
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        mono.get(start..).map(<[f32]>::to_vec).unwrap_or_default()
    }

    /// Stops after the post-roll and returns the 16 kHz mono recording; the device closes in
    /// the background. Blocks for the post-roll and resampling; call it from a blocking context.
    pub fn stop(mut self) -> Result<Recording, AudioError> {
        let _ = self.commands.send(Command::Stop);
        // Detached: the capture thread only has the device left to close.
        self.thread.take();
        self.done.recv().map_err(|_| AudioError::Thread)?
    }

    /// Closes the device and discards the audio.
    pub fn cancel(mut self) {
        let _ = self.commands.send(Command::Cancel);
        if let Err(e) = self.join() {
            tracing::debug!(code = e.code(), "capture cancel");
        }
    }

    fn join(&mut self) -> Result<(), AudioError> {
        let thread = self.thread.take().ok_or(AudioError::Thread)?;
        thread.join().map_err(|_| AudioError::Thread)?
    }
}

impl Drop for Capture {
    /// A capture dropped without stop or cancel is cancelled, so the device never stays open.
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.commands.send(Command::Cancel);
            let _ = self.join();
        }
    }
}

fn frames_for(d: Duration, rate: u32) -> usize {
    let frames = d.as_millis().saturating_mul(u128::from(rate)) / 1000;
    usize::try_from(frames).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(limit_frames: usize, events: mpsc::Sender<CaptureEvent>) -> Shared {
        let events = Mutex::new(events);
        Shared {
            mono: Mutex::new(Vec::new()),
            limit_frames,
            limit_hit: AtomicBool::new(false),
            live: AtomicBool::new(false),
            started: Instant::now(),
            latency: Mutex::new(None),
            events: Box::new(move |e| {
                let _ = events.lock().unwrap().send(e);
            }),
        }
    }

    #[test]
    fn first_audio_marks_live_once_and_limit_keeps_what_fits() {
        let (tx, rx) = mpsc::channel();
        let s = shared(3, tx);
        s.push::<f32>(&[], 1);
        s.push::<f32>(&[0.1, 0.2], 1);
        s.push::<f32>(&[0.3, 0.4], 1);
        s.push::<f32>(&[0.5], 1);
        assert_eq!(*s.mono.lock().unwrap(), [0.1, 0.2, 0.3]);
        let events: Vec<CaptureEvent> = rx.try_iter().collect();
        assert!(matches!(events[0], CaptureEvent::Live { .. }));
        assert_eq!(events[1..], [CaptureEvent::LimitReached]);
        assert!(s.latency.lock().unwrap().is_some());
    }

    #[test]
    fn stereo_i16_is_downmixed() {
        let (tx, _rx) = mpsc::channel();
        let s = shared(usize::MAX, tx);
        s.push::<i16>(&[i16::MAX, 0, 0, 0], 2);
        let mono = s.mono.lock().unwrap().clone();
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.5).abs() < 1e-3 && mono[1].abs() < 1e-6);
    }
}
