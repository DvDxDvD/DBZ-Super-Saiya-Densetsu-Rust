//! The Audio page's choices as the sound output uses them (`audio.*` of the PC file). They only shape the samples
//! handed to the speakers; the emulated machine never sees them.

use ssd_menus::options::{Values, audio};

/// The volume and the output of the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSettings {
    /// 0 (silence) to 10 (the sound as the game makes it).
    pub volume: u8,
    /// Both sides play the same sound.
    pub mono: bool,
}

impl Default for AudioSettings {
    /// What the game did before the Audio page: full volume, stereo.
    fn default() -> Self {
        Self::from_values(&Values::new())
    }
}

impl AudioSettings {
    /// The choices of the PC file's values (a missing or wild value is the row's default or its last choice).
    #[must_use]
    pub fn from_values(values: &Values) -> Self {
        Self {
            volume: u8::try_from(audio::index(values, "audio.volume")).unwrap_or(audio::FULL),
            mono: audio::index(values, "audio.output") == 1,
        }
    }

    /// The gain of the volume: exactly 1.0 at full volume, 0.0 at silence.
    #[must_use]
    pub fn gain(self) -> f32 {
        f32::from(self.volume.min(audio::FULL)) / f32::from(audio::FULL)
    }

    /// One stereo frame as the speakers get it: folded to mono if asked, then scaled by the volume. At the
    /// defaults the frame comes out bit for bit as it went in.
    #[must_use]
    pub fn shape(self, left: f32, right: f32) -> (f32, f32) {
        let (left, right) = if self.mono {
            let mid = f32::midpoint(left, right);
            (mid, mid)
        } else {
            (left, right)
        };
        if self.volume >= audio::FULL {
            (left, right)
        } else {
            (left * self.gain(), right * self.gain())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(volume: i64, output: i64) -> AudioSettings {
        let mut values = Values::new();
        values.set("audio.volume", volume);
        values.set("audio.output", output);
        AudioSettings::from_values(&values)
    }

    #[test]
    fn the_defaults_leave_every_sample_byte_for_byte() {
        let settings = AudioSettings::default();
        assert_eq!(settings, with(10, 0));
        for (l, r) in
            [(0.0, 0.0), (0.25, -0.5), (-1.0, 1.0), (0.123_456_79, -0.987_654_3), (-0.0, 1e-30)]
        {
            let (a, b) = settings.shape(l, r);
            assert_eq!((a.to_bits(), b.to_bits()), (l.to_bits(), r.to_bits()));
        }
    }

    #[test]
    fn zero_is_silence_and_the_steps_rise_evenly() {
        assert_eq!(with(0, 0).shape(0.8, -0.8), (0.0, -0.0));
        let half = with(5, 0).shape(0.8, -0.8);
        assert!((half.0 - 0.4).abs() < 1e-6 && (half.1 + 0.4).abs() < 1e-6);
        assert!(with(3, 0).gain() < with(4, 0).gain());
        assert_eq!(with(99, 0).volume, 10, "a wild value clamps to full");
    }

    #[test]
    fn mono_plays_the_mean_of_both_sides_on_both() {
        assert_eq!(with(10, 1).shape(0.5, -0.25), (0.125, 0.125));
        assert_eq!(with(10, 1).shape(0.5, 0.5), (0.5, 0.5));
        let (l, r) = with(5, 1).shape(1.0, 0.0);
        assert_eq!(l, r);
        assert!((l - 0.25).abs() < 1e-6);
    }
}
