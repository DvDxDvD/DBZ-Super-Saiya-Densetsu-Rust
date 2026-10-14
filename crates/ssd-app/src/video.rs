//! The window: the SDL3 window with its wgpu surface, the cursors, the clipboard and the text input
//! of the interface. What is drawn is [`Canvas`]'s, shared with the UI snapshots.

use crate::canvas::{self, Canvas, GameImage, GameView};
use crate::egui_input::{Geometry, InputAdapter};
use anyhow::Context;
use sdl3::clipboard::ClipboardUtil;
use sdl3::event::Event;
use sdl3::keyboard::TextInputUtil;
use sdl3::mouse::{Cursor, SystemCursor};
use sdl3::video::Window;
use std::time::Instant;

pub(crate) fn should_render(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

/// The surface format of the window. egui draws in gamma space, so a format that is not sRGB-aware
/// gives exact colours: `Bgra8Unorm` and `Rgba8Unorm` first, then any other such format, then the
/// first one the surface offers.
pub(crate) fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
        .into_iter()
        .find(|format| formats.contains(format))
        .or_else(|| formats.iter().copied().find(|format| !format.is_srgb()))
        .or_else(|| formats.first().copied())
}

/// A surface configuration that is due but not made yet. Configuring a surface while one of its
/// textures is alive panics in wgpu, so a `Suboptimal` texture is drawn and presented as it is, and
/// the surface is configured again at the start of the next present.
#[derive(Debug, Default, PartialEq, Eq)]
struct Reconfigure {
    due: bool,
}

impl Reconfigure {
    fn request(&mut self) {
        self.due = true;
    }

    /// Whether a configuration is due; taking it clears the request.
    fn take(&mut self) -> bool {
        std::mem::take(&mut self.due)
    }
}

/// The three mouse cursors the interface uses, created when first needed.
#[derive(Default)]
struct Cursors {
    shown: Option<CursorKind>,
    cursors: [Option<Cursor>; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorKind {
    Arrow,
    Hand,
    IBeam,
}

impl CursorKind {
    fn of(icon: egui::CursorIcon) -> Self {
        match icon {
            egui::CursorIcon::PointingHand | egui::CursorIcon::Grab => Self::Hand,
            egui::CursorIcon::Text | egui::CursorIcon::VerticalText => Self::IBeam,
            _ => Self::Arrow,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn system(self) -> SystemCursor {
        match self {
            Self::Arrow => SystemCursor::Arrow,
            Self::Hand => SystemCursor::Hand,
            Self::IBeam => SystemCursor::IBeam,
        }
    }
}

impl Cursors {
    fn show(&mut self, icon: egui::CursorIcon) {
        let kind = CursorKind::of(icon);
        if self.shown == Some(kind) {
            return;
        }
        let slot = &mut self.cursors[kind.index()];
        if slot.is_none() {
            *slot = Cursor::from_system(kind.system()).ok();
        }
        if let Some(cursor) = slot {
            cursor.set();
            self.shown = Some(kind);
        }
    }
}

pub struct Video {
    canvas: Canvas,
    // SAFETY: the surface must not outlive the window it was created from, so the window is
    // declared after it and dropped after it.
    surface_config: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    cursors: Cursors,
    clipboard: ClipboardUtil,
    text_input: TextInputUtil,
    text_input_on: bool,
    minimized: bool,
    reconfigure: Reconfigure,
    started: Instant,
    window: Window,
}

fn window_size(window: &Window) -> (u32, u32) {
    window.size_in_pixels()
}

impl Video {
    /// Opens the window (1440 x 900 points, the size of the design) and sets up wgpu and egui.
    pub fn new(video: &sdl3::VideoSubsystem, title: &str) -> anyhow::Result<Self> {
        let mut builder = video.window(title, 1440, 900);
        builder.high_pixel_density();
        builder.resizable();
        builder.position_centered();
        let mut window = builder.build().context("creating window")?;
        if let Err(e) = window.set_minimum_size(640, 400) {
            log::warn!("setting the minimum window size failed: {e}");
        }

        let instance = canvas::instance();
        // SAFETY: the surface is dropped before the window (field order of `Video`).
        let surface = unsafe {
            instance.create_surface_unsafe(
                wgpu::SurfaceTargetUnsafe::from_display_and_window(&window, &window)
                    .map_err(|e| anyhow::anyhow!("getting the window handle: {e}"))?,
            )
        }
        .map_err(|e| anyhow::anyhow!("creating the wgpu surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| anyhow::anyhow!("no wgpu adapter: {e}"))?;
        let info = adapter.get_info();
        log::info!("wgpu adapter {} ({:?})", info.name, info.backend);
        let (device, queue) = canvas::device(&adapter)?;

        let capabilities = surface.get_capabilities(&adapter);
        let format =
            pick_format(&capabilities.formats).context("the surface supports no format")?;
        let (width, height) = window_size(&window);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            // The main loop paces the frames itself.
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 1,
            alpha_mode: capabilities.alpha_modes.first().copied().unwrap_or_default(),
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);
        let canvas = Canvas::new(device, queue, format);
        // Text events only while a text field has the keyboard (see `present`).
        let text_input = video.text_input();
        text_input.stop(&window);
        Ok(Self {
            canvas,
            surface_config,
            surface,
            cursors: Cursors::default(),
            clipboard: video.clipboard(),
            text_input,
            text_input_on: false,
            minimized: false,
            reconfigure: Reconfigure::default(),
            started: Instant::now(),
            window,
        })
    }

    /// Windows keeps reporting the last size while minimised, so track it explicitly
    /// (`WindowEvent::Minimized` / `WindowEvent::Restored`).
    pub fn set_minimized(&mut self, minimized: bool) {
        self.minimized = minimized;
    }

    pub fn set_title(&mut self, title: &str) {
        if let Err(e) = self.window.set_title(title) {
            log::warn!("setting the window title failed: {e}");
        }
    }

    pub fn handle_resize(&mut self) {
        let (width, height) = window_size(&self.window);
        if should_render(width, height)
            && (width, height) != (self.surface_config.width, self.surface_config.height)
        {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(self.canvas.device(), &self.surface_config);
        }
    }

    /// How SDL's coordinates relate to egui's points in this window now.
    pub fn geometry(&self) -> Geometry {
        Geometry {
            pixel_density: self.window.pixel_density(),
            pixels_per_point: self.window.display_scale().max(0.1),
        }
    }

    /// Seconds since the window opened: the clock of the interface.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Whether a text field of the interface has the keyboard (as of the last frame).
    pub fn text_focus(&self) -> bool {
        self.canvas.context().text_edit_focused()
    }

    /// The system clipboard's text.
    pub fn clipboard_text(&self) -> Option<String> {
        self.clipboard.clipboard_text().ok().filter(|text| !text.is_empty())
    }

    /// Feeds an SDL event to the interface.
    pub fn handle_event(&self, input: &mut InputAdapter, event: &Event) {
        input.handle(event, self.geometry(), self.text_focus(), || self.clipboard_text());
    }

    /// Draws one window frame: the game views (each of `views` as a texture for `ui` to place, in
    /// order) and the interface `ui` builds from the events in `input`. Skipped while the window is
    /// minimised.
    pub fn present(
        &mut self,
        views: &[GameView<'_>],
        input: &mut InputAdapter,
        ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> anyhow::Result<()> {
        let size = window_size(&self.window);
        if self.minimized || !should_render(size.0, size.1) {
            return Ok(());
        }
        let raw =
            input.take_raw_input(self.geometry(), size, self.now(), self.canvas.max_texture_side());
        let pass = self.canvas.run(raw, views, ui);
        self.handle_platform_output(&pass.platform);

        // The last texture was presented, so none is alive: the surface may be configured now.
        if self.reconfigure.take() {
            self.surface.configure(self.canvas.device(), &self.surface_config);
        }
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                // Drawn and presented as it is; the surface adapts to the window before the next.
                self.reconfigure.request();
                texture
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(self.canvas.device(), &self.surface_config);
                self.canvas.discard(pass);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.canvas.discard(pass);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.canvas.discard(pass);
                anyhow::bail!("the wgpu surface failed validation")
            }
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.canvas.paint(pass, &view, [self.surface_config.width, self.surface_config.height]);
        self.canvas.queue().present(surface_texture);
        Ok(())
    }

    /// Carries out what egui asked of the system: cursor, clipboard, text input.
    fn handle_platform_output(&mut self, output: &egui::PlatformOutput) {
        self.cursors.show(output.cursor_icon);
        for command in &output.commands {
            if let egui::OutputCommand::CopyText(text) = command
                && let Err(e) = self.clipboard.set_clipboard_text(text)
            {
                log::warn!("copying to the clipboard failed: {e}");
            }
        }
        let wants_text = self.canvas.context().text_edit_focused();
        if wants_text != self.text_input_on {
            self.text_input_on = wants_text;
            if wants_text {
                self.text_input.start(&self.window);
            } else {
                self.text_input.stop(&self.window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sized_window_is_not_rendered() {
        assert!(!should_render(0, 0));
        assert!(!should_render(800, 0));
        assert!(should_render(1, 1));
    }

    #[test]
    fn the_surface_format_is_a_linear_one_the_surface_offers() {
        use wgpu::TextureFormat as F;
        assert_eq!(pick_format(&[F::Bgra8UnormSrgb, F::Bgra8Unorm]), Some(F::Bgra8Unorm));
        assert_eq!(pick_format(&[F::Rgba16Float, F::Rgba8Unorm]), Some(F::Rgba8Unorm));
        assert_eq!(pick_format(&[F::Rgb10a2Unorm, F::Rgba8UnormSrgb]), Some(F::Rgb10a2Unorm));
        assert_eq!(pick_format(&[F::Bgra8UnormSrgb, F::Rgba8UnormSrgb]), Some(F::Bgra8UnormSrgb));
        assert_eq!(pick_format(&[]), None, "a surface with no format");
    }

    #[test]
    fn a_suboptimal_surface_is_configured_again_once_then_not_until_asked_again() {
        let mut reconfigure = Reconfigure::default();
        assert!(!reconfigure.take(), "nothing due at first");
        reconfigure.request();
        assert!(reconfigure.take(), "due after a suboptimal acquire");
        assert!(!reconfigure.take(), "taken: not due again");
        reconfigure.request();
        reconfigure.request();
        assert!(reconfigure.take());
        assert!(!reconfigure.take(), "requests do not stack");
    }

    #[test]
    fn cursors_follow_the_icon_egui_asks_for() {
        assert_eq!(CursorKind::of(egui::CursorIcon::Default), CursorKind::Arrow);
        assert_eq!(CursorKind::of(egui::CursorIcon::PointingHand), CursorKind::Hand);
        assert_eq!(CursorKind::of(egui::CursorIcon::Text), CursorKind::IBeam);
        assert_eq!(CursorKind::of(egui::CursorIcon::Wait), CursorKind::Arrow);
        let mut seen = [false; 3];
        for kind in [CursorKind::Arrow, CursorKind::Hand, CursorKind::IBeam] {
            seen[kind.index()] = true;
        }
        assert_eq!(seen, [true; 3], "each kind has a slot of its own");
    }
}
