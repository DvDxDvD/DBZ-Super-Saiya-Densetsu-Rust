use ssd_oracle::AudioSink;

const FREQUENCY: u64 = 48_000;
const MAX_LATENCY_MS: usize = 150;

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
}

pub(crate) fn open_with<F>(open: F) -> Audio
where
    F: FnOnce() -> Result<sdl3::audio::AudioStreamOwner, String>,
{
    match open() {
        Ok(stream) => Audio { stream: Some(stream), pending: Vec::with_capacity(4096) },
        Err(e) => {
            log::warn!("audio unavailable, running muted: {e}");
            Audio { stream: None, pending: Vec::new() }
        }
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
            // Prime ~50 ms of silence to avoid an underrun before the first frame's samples arrive.
            let silence = vec![0.0_f32; (FREQUENCY as usize / 20) * 2];
            stream.put_data_f32(&silence).map_err(|e| e.to_string())?;
            stream.resume().map_err(|e| e.to_string())?; // device streams start paused
            Ok(stream)
        })
    }

    /// A muted output, with no device: what a run without sound has (the bench).
    pub fn muted() -> Audio {
        Audio { stream: None, pending: Vec::new() }
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
        trim_for_latency(queued, FREQUENCY, &mut self.pending);
        if let Err(e) = stream.put_data_f32(&self.pending) {
            log::warn!("dropping audio after SDL error: {e}");
        }
        self.pending.clear();
    }
}

impl AudioSink for Audio {
    fn push(&mut self, left: f32, right: f32) {
        if !self.is_muted() {
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
