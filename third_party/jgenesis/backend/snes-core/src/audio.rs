//! SNES audio resampling code

use crate::apu;
use bincode::{Decode, Encode};
use dsp::design::FilterType;
use dsp::iir::FirstOrderIirFilter;
use dsp::sinc::QualitySincResampler;
use jgenesis_common::frontend::AudioOutput;
use jgenesis_proc_macros::{FakeDecode, FakeEncode};

const SNES_AUDIO_FREQUENCY: f64 = apu::OUTPUT_FREQUENCY as f64;

fn new_dc_offset_filter() -> FirstOrderIirFilter {
    dsp::design::butterworth(5.0, SNES_AUDIO_FREQUENCY, FilterType::HighPass)
}

/// Whether the resampler takes samples: on by default, and not part of a save state.
#[derive(Debug, Clone, Copy, FakeEncode, FakeDecode)]
struct Enabled(bool);

impl Default for Enabled {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct AudioResampler {
    enabled: Enabled,
    dc_offset_l: FirstOrderIirFilter,
    dc_offset_r: FirstOrderIirFilter,
    resampler: QualitySincResampler<2>,
}

impl AudioResampler {
    pub fn new() -> Self {
        Self {
            enabled: Enabled::default(),
            dc_offset_l: new_dc_offset_filter(),
            dc_offset_r: new_dc_offset_filter(),
            resampler: QualitySincResampler::new(SNES_AUDIO_FREQUENCY, 48000.0),
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = Enabled(on);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.0
    }

    pub fn collect_sample(&mut self, sample_l: f64, sample_r: f64) {
        if !self.enabled.0 {
            return;
        }
        let sample_l = self.dc_offset_l.filter(sample_l);
        let sample_r = self.dc_offset_r.filter(sample_r);
        self.resampler.collect([sample_l, sample_r]);
    }

    pub fn output_samples<A: AudioOutput>(&mut self, audio_output: &mut A) -> Result<(), A::Err> {
        while let Some([sample_l, sample_r]) = self.resampler.output_buffer_pop_front() {
            audio_output.push_sample(sample_l, sample_r)?;
        }

        Ok(())
    }

    pub fn update_output_frequency(&mut self, output_frequency: u64) {
        self.resampler.update_output_frequency(output_frequency as f64);
    }
}
