use crate::options::audio::AudioSettings;
use ssd_oracle::AudioSink;

const FREQUENCY: u64 = 48_000;
const MAX_LATENCY_MS: usize = 150;
/// The queue the rate control keeps: room for a late frame or two before the device runs dry.
const TARGET_MS: f32 = 60.0;
/// The most the playback speed is bent to get back to [`TARGET_MS`]: half a percent, which no ear hears.
const MAX_BEND: f32 = 0.005;

/// The speed SDL plays the queue at for a queue of `queued_ms`: faster above the target, slower below, by up to
/// [`MAX_BEND`]. The game makes its samples on the clock of its frames, the device takes them on its own (and a
/// phone's frames are not a desktop's), so without the bend the queue drifts until it runs dry (gaps: crackle) or
/// overflows (cut samples: clicks).
pub(crate) fn rate_ratio(queued_ms: f32) -> f32 {
    1.0 + MAX_BEND * ((queued_ms - TARGET_MS) / TARGET_MS).clamp(-1.0, 1.0)
}

/// Keeps queued plus pending audio under `MAX_LATENCY_MS` by dropping the oldest pending samples.
///
/// Frame pacing is driven by the clock, so the SDL queue can drift away from the emulator's
/// sample production; this is the drift rule. Whole stereo frames are dropped, never a single
/// channel.
pub(crate) fn trim_for_latency(queued_bytes: usize, frequency: u64, pending: &mut Vec<f32>) {
    let max_bytes = (frequency as usize * MAX_LATENCY_MS / 1000) * 2 * 4;
    let total = queued_bytes + pending.len() * 4;
    if total > max_bytes {
        let excess_floats = ((total - max_bytes) / 4).min(pending.len());
        let excess_floats = excess_floats - excess_floats % 2;
        pending.drain(..excess_floats);
    }
}

/// Audio output: collects the samples of a frame and queues them to an SDL3 playback stream.
pub struct Audio {
    stream: Option<sdl3::audio::AudioStreamOwner>,
    pending: Vec<f32>,
    settings: AudioSettings,
    /// The queue at the last frame's end, in milliseconds.
    queued_ms: f32,
    /// The frames that found the queue empty (the device ran dry): each is a gap the ear hears.
    underruns: u32,
}

pub(crate) fn open_with<F>(open: F) -> Audio
where
    F: FnOnce() -> Result<sdl3::audio::AudioStreamOwner, String>,
{
    let stream = open().inspect_err(|e| log::warn!("audio unavailable, running muted: {e}")).ok();
    Audio {
        pending: if stream.is_some() { Vec::with_capacity(4096) } else { Vec::new() },
        stream,
        settings: AudioSettings::default(),
        queued_ms: 0.0,
        underruns: 0,
    }
}

impl Audio {
    /// Opens the default playback device; never fails, falling back to muted output.
    pub fn open(sdl: &sdl3::Sdl) -> Audio {
        open_with(|| {
            let audio = sdl.audio().map_err(|e| e.to_string())?;
            let spec = sdl3::audio::AudioSpec {
                freq: Some(FREQUENCY as i32),
                channels: Some(2),
                format: Some(sdl3::audio::AudioFormat::f32_sys()),
            };
            let device = audio.open_playback_device(&spec).map_err(|e| e.to_string())?;
            let stream = device.open_device_stream(Some(&spec)).map_err(|e| e.to_string())?;
            // Prime the target's worth of silence so the first frames do not find the device dry.
            let silence = vec![0.0_f32; (FREQUENCY as f32 * TARGET_MS / 1000.0) as usize * 2];
            stream.put_data_f32(&silence).map_err(|e| e.to_string())?;
            stream.resume().map_err(|e| e.to_string())?; // device streams start paused
            Ok(stream)
        })
    }

    /// A muted output, with no device: what a run without sound has (the bench).
    pub fn muted() -> Audio {
        open_with(|| Err("muted".to_owned()))
    }

    /// The queue at the last frame's end in milliseconds, and the frames so far that found it empty.
    pub fn health(&self) -> (f32, u32) {
        (self.queued_ms, self.underruns)
    }

    /// Sets the volume and the output the samples are shaped by from now on.
    pub fn set_settings(&mut self, settings: AudioSettings) {
        self.settings = settings;
    }

    pub fn frequency() -> u64 {
        FREQUENCY
    }

    pub fn is_muted(&self) -> bool {
        self.stream.is_none()
    }

    /// Flushes the samples pushed during the frame to SDL, trimming them to the latency cap.
    pub fn end_frame(&mut self) {
        let Some(stream) = &self.stream else {
            self.pending.clear();
            return;
        };
        let queued = stream.queued_bytes().map_or(0, |q| q.max(0) as usize);
        if queued == 0 && !self.pending.is_empty() {
            self.underruns += 1;
        }
        trim_for_latency(queued, FREQUENCY, &mut self.pending);
        if let Err(e) = stream.put_data_f32(&self.pending) {
            log::warn!("dropping audio after SDL error: {e}");
        }
        // bytes of stereo f32 to milliseconds
        self.queued_ms =
            (queued + self.pending.len() * 4) as f32 / (FREQUENCY as f32 * 2.0 * 4.0) * 1000.0;
        // SAFETY: the stream is alive (owned by `self`); the call only sets the stream's playback ratio.
        unsafe {
            sdl3::sys::audio::SDL_SetAudioStreamFrequencyRatio(
                stream.raw(),
                rate_ratio(self.queued_ms),
            );
        }
        self.pending.clear();
    }
}

impl AudioSink for Audio {
    fn push(&mut self, left: f32, right: f32) {
        if !self.is_muted() {
            let (left, right) = self.settings.shape(left, right);
            self.pending.push(left);
            self.pending.push(right);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_failure_falls_back_to_muted() {
        let mut audio = open_with(|| Err("no device".to_owned()));
        assert!(audio.is_muted());
        assert_eq!(Audio::frequency(), 48_000);
        audio.push(0.5, -0.5);
        assert!(audio.pending.is_empty(), "a muted Audio must not accumulate samples");
        audio.end_frame(); // must not panic
    }

    #[test]
    fn the_rate_bends_toward_the_target_queue_by_half_a_percent_at_most() {
        assert!((rate_ratio(TARGET_MS) - 1.0).abs() < 1e-6);
        assert!(rate_ratio(90.0) > 1.0, "a full queue plays a little faster");
        assert!(rate_ratio(30.0) < 1.0, "a low one a little slower");
        assert!((rate_ratio(10_000.0) - 1.005).abs() < 1e-6);
        assert!((rate_ratio(0.0) - 0.995).abs() < 1e-6);
    }

    #[test]
    fn latency_trim_drops_oldest_samples_above_150ms() {
        let freq = 48_000;
        let max_bytes = (freq as usize * 150 / 1000) * 2 * 4; // 57_600
        // pending = 0.0, 1.0, ... 1599.0 (6400 bytes)
        let fresh = || (0..1600).map(|i| i as f32).collect::<Vec<f32>>();

        // total = queued + 6400 = max + 12 bytes -> 3 excess floats, aligned down to 2 (one stereo frame)
        let mut pending = fresh();
        trim_for_latency(max_bytes - 6400 + 12, freq, &mut pending);
        assert_eq!(pending.len(), 1598);
        assert_eq!(pending[0], 2.0, "oldest samples are dropped first");
        assert_eq!(*pending.last().unwrap(), 1599.0, "newest samples are kept");

        // exactly at the cap: untouched
        let mut at_cap = fresh();
        trim_for_latency(max_bytes - 6400, freq, &mut at_cap);
        assert_eq!(at_cap.len(), 1600);

        // far above the cap: everything pending may go, but never more than pending
        let mut all = fresh();
        trim_for_latency(max_bytes + 100_000, freq, &mut all);
        assert_eq!(all, Vec::<f32>::new());
    }
}
