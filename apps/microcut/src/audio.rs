//! Sound output through `cpal`. A decoder thread fills a sample queue; the device callback empties
//! it and counts what it played, which is the playback clock.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use microcut_media::AudioFormat;

use crate::volume::gain_of;

/// Queue length in seconds of audio: enough to ride out a slow decoder, short enough to stop
/// promptly.
const QUEUE_SECONDS: usize = 1;

/// State shared by the decoder thread, the device callback and the UI.
pub struct AudioShared {
    queue: Mutex<VecDeque<f32>>,
    /// Signalled when the callback made room in the queue.
    room: Condvar,
    capacity: usize,
    /// Until set, the callback plays silence and the clock stands still.
    running: AtomicBool,
    /// Samples (all channels counted) taken from the queue and played.
    played: AtomicU64,
    /// The decoder delivered everything (or gave up).
    decoder_done: AtomicBool,
    /// Asks the decoder to stop.
    stop: AtomicBool,
    error: Mutex<Option<String>>,
    /// Volume set by the user (see [`crate::volume`]), applied to every sample.
    gain: Arc<AtomicU32>,
}

impl AudioShared {
    /// Lets the device start consuming samples (and the clock run).
    pub fn start(&self) {
        self.running.store(true, Ordering::Release);
    }

    /// Seconds of audio played so far.
    pub fn played_secs(&self, format: AudioFormat) -> f64 {
        let samples = self.played.load(Ordering::Acquire) as f64;
        samples / f64::from(format.channels.max(1)) / f64::from(format.sample_rate.max(1))
    }

    /// Whether everything decoded has been played: the clock cannot advance any further.
    pub fn exhausted(&self) -> bool {
        self.decoder_done.load(Ordering::Acquire) && self.lock_queue().is_empty()
    }

    /// Error reported by the device or the decoder.
    pub fn error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_error(&self, message: String) {
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
    }

    pub fn set_decoder_done(&self) {
        self.decoder_done.store(true, Ordering::Release);
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.room.notify_all();
    }

    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Appends `samples`, waiting while the queue is full. Returns `false` if asked to stop.
    pub fn push(&self, samples: &[f32]) -> bool {
        let mut rest = samples;
        while !rest.is_empty() {
            let mut queue = self.lock_queue();
            while queue.len() >= self.capacity {
                if self.stop_requested() {
                    return false;
                }
                queue = self
                    .room
                    .wait_timeout(queue, Duration::from_millis(50))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            if self.stop_requested() {
                return false;
            }
            let take = rest.len().min(self.capacity.saturating_sub(queue.len()));
            let (now, later) = rest.split_at(take);
            queue.extend(now);
            rest = later;
        }
        true
    }

    fn lock_queue(&self) -> std::sync::MutexGuard<'_, VecDeque<f32>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Device callback body: fills `out` from the queue, silence where nothing is available.
    fn fill<T: SizedSample + FromSample<f32>>(&self, out: &mut [T]) {
        let silence = T::EQUILIBRIUM;
        if !self.running.load(Ordering::Acquire) {
            out.fill(silence);
            return;
        }
        // Never block the audio thread: if the decoder holds the lock, play silence this time.
        let Ok(mut queue) = self.queue.try_lock() else {
            out.fill(silence);
            return;
        };
        let gain = gain_of(&self.gain);
        let mut taken: u64 = 0;
        for slot in out.iter_mut() {
            *slot = match queue.pop_front() {
                Some(sample) => {
                    taken += 1;
                    T::from_sample(sample * gain)
                }
                None => silence,
            };
        }
        drop(queue);
        self.played.fetch_add(taken, Ordering::AcqRel);
        self.room.notify_one();
    }
}

/// An open output stream on the default sound device. Not `Send` (a `cpal` stream stays on the
/// thread that made it), so it lives with the UI.
pub struct AudioOut {
    _stream: cpal::Stream,
    shared: Arc<AudioShared>,
    format: AudioFormat,
}

impl AudioOut {
    /// Opens the default output device in its preferred format and starts it (silent until
    /// [`AudioShared::start`]); samples are scaled by the gain in `gain`.
    pub fn open(gain: Arc<AtomicU32>) -> Result<AudioOut, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no sound output device".to_owned())?;
        let supported = device
            .default_output_config()
            .map_err(|err| format!("sound device: {err}"))?;
        let config = supported.config();
        let format = AudioFormat {
            sample_rate: config.sample_rate,
            channels: config.channels,
        };
        let capacity = usize::try_from(format.sample_rate)
            .unwrap_or(48_000)
            .saturating_mul(usize::from(format.channels))
            .saturating_mul(QUEUE_SECONDS)
            .max(4096);
        let shared = Arc::new(AudioShared {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            room: Condvar::new(),
            capacity,
            running: AtomicBool::new(false),
            played: AtomicU64::new(0),
            decoder_done: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
            gain,
        });

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, &shared),
            SampleFormat::F64 => build::<f64>(&device, &config, &shared),
            SampleFormat::I16 => build::<i16>(&device, &config, &shared),
            SampleFormat::I32 => build::<i32>(&device, &config, &shared),
            SampleFormat::U16 => build::<u16>(&device, &config, &shared),
            SampleFormat::U8 => build::<u8>(&device, &config, &shared),
            other => Err(format!("sound device format {other} is not supported")),
        }?;
        stream
            .play()
            .map_err(|err| format!("sound device: {err}"))?;
        Ok(AudioOut {
            _stream: stream,
            shared,
            format,
        })
    }

    pub fn shared(&self) -> &Arc<AudioShared> {
        &self.shared
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        self.shared.request_stop();
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: &Arc<AudioShared>,
) -> Result<cpal::Stream, String> {
    let (data, errors) = (Arc::clone(shared), Arc::clone(shared));
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |out: &mut [T], _| data.fill(out),
            move |err| errors.set_error(format!("sound device: {err}")),
            None,
        )
        .map_err(|err| format!("sound device: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(capacity: usize) -> AudioShared {
        AudioShared {
            queue: Mutex::new(VecDeque::new()),
            room: Condvar::new(),
            capacity,
            running: AtomicBool::new(false),
            played: AtomicU64::new(0),
            decoder_done: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
            gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
        }
    }

    #[test]
    fn silent_and_still_until_started() {
        let s = shared(16);
        assert!(s.push(&[0.5; 8]));
        let mut out = [1.0f32; 4];
        s.fill(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(s.played.load(Ordering::Acquire), 0);
    }

    #[test]
    fn clock_counts_played_samples_only() {
        let s = shared(16);
        s.start();
        assert!(s.push(&[0.25; 6]));
        let mut out = [1.0f32; 8];
        s.fill(&mut out);
        assert_eq!(out, [0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.0, 0.0]);
        let format = AudioFormat {
            sample_rate: 3,
            channels: 2,
        };
        assert!((s.played_secs(format) - 1.0).abs() < 1e-12);
        s.set_decoder_done();
        assert!(s.exhausted());
    }

    #[test]
    fn integer_devices_get_converted_samples() {
        let s = shared(16);
        s.start();
        assert!(s.push(&[1.0, -1.0, 0.0]));
        let mut out = [7i16; 3];
        s.fill(&mut out);
        assert_eq!(out, [i16::MAX, i16::MIN, 0]);
    }

    #[test]
    fn volume_scales_the_samples() {
        let s = shared(16);
        s.start();
        s.gain.store(0.25f32.to_bits(), Ordering::Release);
        assert!(s.push(&[0.8, -0.4]));
        let mut out = [0.0f32; 2];
        s.fill(&mut out);
        assert_eq!(out, [0.2, -0.1]);
    }

    #[test]
    fn push_gives_up_when_stopped() {
        let s = shared(4);
        s.request_stop();
        assert!(!s.push(&[0.0; 10]));
    }
}
