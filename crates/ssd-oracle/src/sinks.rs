use jgenesis_common::frontend::{AudioOutput, Color, FrameSize, RenderFrameOptions, Renderer};
use std::convert::Infallible;

pub trait AudioSink {
    fn push(&mut self, left: f32, right: f32);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullAudio;

impl AudioSink for NullAudio {
    fn push(&mut self, _left: f32, _right: f32) {}
}

/// Captures the last frame jgenesis renders (pixels + size + its own render options).
#[derive(Clone, Default)]
pub(crate) struct FrameCapture {
    pub pixels: Vec<Color>,
    pub size: Option<FrameSize>,
    pub options: RenderFrameOptions,
}

impl Renderer for FrameCapture {
    type Err = Infallible;

    fn render_frame(
        &mut self,
        frame_buffer: &[Color],
        frame_size: FrameSize,
        _target_fps: f64,
        options: RenderFrameOptions,
    ) -> Result<(), Infallible> {
        let len = (frame_size.width * frame_size.height) as usize;
        self.pixels.clear();
        self.pixels.extend_from_slice(&frame_buffer[..len]);
        self.size = Some(frame_size);
        self.options = options;
        Ok(())
    }
}

pub(crate) struct AudioAdapter<'a>(pub &'a mut dyn AudioSink);

impl AudioOutput for AudioAdapter<'_> {
    type Err = Infallible;

    fn push_sample(&mut self, l: f64, r: f64) -> Result<(), Infallible> {
        self.0.push(l as f32, r as f32);
        Ok(())
    }
}
