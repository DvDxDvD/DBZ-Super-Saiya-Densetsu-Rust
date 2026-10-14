//! The drawing shared by the window and the UI snapshots: the game frames as textures, and the egui
//! interface painted through egui-wgpu into any texture. The window paints into its surface, the
//! snapshot into an offscreen texture, both through [`Canvas::paint`], so a snapshot shows what the
//! window draws.

use ssd_hd::HdFrame;
use ssd_oracle::Frame;

/// The game frame as egui draws it.
#[derive(Debug, Clone, Copy)]
pub struct GameImage {
    pub id: egui::TextureId,
    pub width: u32,
    pub height: u32,
    /// Width over height of one pixel (1 when the frame does not say).
    pub pixel_aspect: f64,
}

/// One game view as the canvas uploads it: RGBA pixels of `width` x `height`, and the width over
/// height of one pixel. The Classic picture is the machine's frame ([`GameView::classic`]); the
/// Enhanced one is an `ssd_hd` picture. The Classic picture is drawn with the nearest pixel (its
/// pixels are the game's), the Enhanced one smoothed, so the art's anti-aliased edges are kept
/// when the window is not a whole multiple of the picture.
#[derive(Debug, Clone, Copy)]
pub struct GameView<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f64,
    /// Whether the picture is drawn with linear filtering (see above).
    pub smooth: bool,
}

impl<'a> GameView<'a> {
    /// The machine's frame as the PPU drew it (the Classic picture).
    #[must_use]
    pub fn classic(frame: &Frame<'a>) -> Self {
        Self {
            rgba: bytemuck::cast_slice(frame.pixels),
            width: frame.size.width,
            height: frame.size.height,
            pixel_aspect: pixel_aspect(frame),
            smooth: false,
        }
    }

    /// The Enhanced picture `picture`, shown with the pixel aspect of `frame`, the machine's frame
    /// it was made from: the picture keeps the proportions of the Classic one.
    #[must_use]
    pub fn enhanced(picture: &'a HdFrame, frame: &Frame<'_>) -> Self {
        Self {
            rgba: &picture.rgba,
            width: picture.width,
            height: picture.height,
            pixel_aspect: pixel_aspect(frame),
            smooth: true,
        }
    }
}

/// The width over height of one pixel of the frame, 1 when the frame does not say.
fn pixel_aspect(frame: &Frame<'_>) -> f64 {
    match frame.options.pixel_aspect_ratio {
        Some(ratio) => ratio.get(),
        None => 1.0,
    }
}

/// The texture of one game view, uploaded with its frame each time it is drawn.
struct GameTexture {
    texture: wgpu::Texture,
    id: egui::TextureId,
    width: u32,
    height: u32,
    smooth: bool,
}

/// One run of the interface: tessellated, waiting to be painted (or discarded).
pub struct Pass {
    /// What egui asks of the system (cursor, clipboard, text input); a window carries it out.
    pub platform: egui::PlatformOutput,
    jobs: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    /// Textures egui no longer uses: freed once the pass is painted or discarded.
    free: Vec<egui::TextureId>,
}

/// The egui context, the egui-wgpu renderer and the game textures, on one device.
pub struct Canvas {
    egui: egui::Context,
    renderer: egui_wgpu::Renderer,
    /// One texture per game view (the frame of the run, and the two of a verification).
    games: Vec<Option<GameTexture>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// The wgpu instance: DX12 first on Windows, where wgpu prefers Vulkan, which shows wrong colours
/// on some AMD drivers.
#[must_use]
pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: if cfg!(windows) { wgpu::Backends::DX12 } else { wgpu::Backends::PRIMARY },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// A device and its queue on `adapter`, with the default features and limits.
///
/// # Errors
///
/// The adapter cannot create a device.
pub fn device(adapter: &wgpu::Adapter) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ssd device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .map_err(|e| anyhow::anyhow!("creating the wgpu device: {e}"))
}

impl Canvas {
    /// A canvas that paints into textures of `format`. It must not be sRGB-aware: egui draws in
    /// gamma space, and a non-sRGB format gives the exact colours.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let renderer = egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..egui_wgpu::RendererOptions::default()
            },
        );
        let egui = egui::Context::default();
        crate::theme::install(&egui);
        Self { egui, renderer, games: Vec::new(), device, queue }
    }

    /// The egui context: what the interface knows between passes (focus, the memory of layouts).
    #[must_use]
    pub fn context(&self) -> &egui::Context {
        &self.egui
    }

    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The largest texture side the device takes: egui's input must say so.
    #[must_use]
    pub fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }

    /// The texture of game view `slot`, uploaded with the pixels of `view`.
    fn game_image(&mut self, slot: usize, view: &GameView<'_>) -> GameImage {
        let (width, height, smooth) = (view.width, view.height, view.smooth);
        if self.games.len() <= slot {
            self.games.resize_with(slot + 1, || None);
        }
        let rebuild = self.games[slot]
            .as_ref()
            .is_none_or(|g| (g.width, g.height, g.smooth) != (width, height, smooth));
        if rebuild {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("game frame"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let filter = if smooth { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
            let id = match self.games[slot].take() {
                Some(old) => {
                    self.renderer.update_egui_texture_from_wgpu_texture(
                        &self.device,
                        &view,
                        filter,
                        old.id,
                    );
                    old.id
                }
                None => self.renderer.register_native_texture(&self.device, &view, filter),
            };
            self.games[slot] = Some(GameTexture { texture, id, width, height, smooth });
        }
        let game = self.games[slot].as_ref().expect("created above");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &game.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            view.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        GameImage { id: game.id, width, height, pixel_aspect: view.pixel_aspect }
    }

    /// Runs one pass of the interface: the game views (each of `views` as a texture for `ui` to
    /// place, in order) and what `ui` builds from `raw`. The textures egui needs are uploaded now;
    /// the pass is painted with [`Canvas::paint`] or dropped with [`Canvas::discard`].
    pub fn run(
        &mut self,
        raw: egui::RawInput,
        views: &[GameView<'_>],
        mut ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> Pass {
        let games: Vec<GameImage> =
            views.iter().enumerate().map(|(slot, view)| self.game_image(slot, view)).collect();
        let mut output = self.egui.run_ui(raw, |root| ui(root, &games));
        let platform = std::mem::take(&mut output.platform_output);
        // Taken out of the output and drained below: epaint asserts in debug builds that a
        // `TexturesDelta` is empty when dropped, and a pass that is not painted must not leave one.
        let mut textures = std::mem::take(&mut output.textures_delta);
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.renderer.update_texture(&self.device, &self.queue, id, delta);
            }
        }
        let free: Vec<egui::TextureId> = textures.free.drain().collect();
        let jobs = self.egui.tessellate(output.shapes, output.pixels_per_point);
        Pass { platform, jobs, pixels_per_point: output.pixels_per_point, free }
    }

    /// Paints a pass into `target`, `size` pixels, cleared to black, and submits it. The index of
    /// the submission says when the GPU has finished with it.
    pub fn paint(
        &mut self,
        pass: Pass,
        target: &wgpu::TextureView,
        size: [u32; 2],
    ) -> wgpu::SubmissionIndex {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: pass.pixels_per_point,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ssd frame") });
        let extra = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &pass.jobs,
            &screen,
        );
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssd pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.renderer.render(&mut rpass.forget_lifetime(), &pass.jobs, &screen);
        }
        let submitted =
            self.queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        self.release(pass.free);
        submitted
    }

    /// Drops a pass that is not painted (the surface is not there this time), freeing its
    /// textures all the same.
    pub fn discard(&mut self, pass: Pass) {
        self.release(pass.free);
    }

    /// Frees textures egui no longer uses.
    fn release(&mut self, free: Vec<egui::TextureId>) {
        for id in free {
            self.renderer.free_texture(&id);
        }
    }
}
