//! The renderer: owns a color target, the active content and the interface
//! pass, and turns a [`ViewState`] plus a [`DrawList`] into a frame.

use crate::color::clear_value;
use crate::gpu::{self, Gpu, GpuOptions};
use crate::image::frame_params;
use crate::pipelines::{DisplayPipelines, Pipelines, DEPTH_FORMAT, FRAME_PARAMS_SIZE};
use crate::provider::{
    ContentRenderer, DrawContext, FrameContext, Placement, ProviderContext, ProviderRegistry,
    RenderInput, TargetInfo,
};
use crate::settings::RenderSettings;
use crate::text::{FontSet, TextMeasurer};
use crate::ui::{scissor_for, UiRenderer};
use pandite_core::{
    Content, ContentKind, DrawList, Error, ImageBuf, PixelFormat, Rect, Result, Size, TextMeasure,
    TextureId, TextureSink, ViewState,
};
use std::sync::Arc;
use std::time::Duration;

/// Format of offscreen targets: 8-bit sRGB, read back as sRGB bytes.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// What [`Renderer::render`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStatus {
    /// The frame was drawn (and presented, for a window).
    Drawn,
    /// Nothing was drawn: the target has zero size, the window is
    /// occluded, or acquiring the next image timed out. Try again later.
    Skipped,
}

/// Recreates a lost window surface.
type SurfaceFactory = Box<dyn Fn(&wgpu::Instance) -> Result<wgpu::Surface<'static>> + Send + Sync>;

enum Target {
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        factory: Option<SurfaceFactory>,
    },
    Offscreen {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
    },
}

/// Content created by a provider, ready to be shown. Made by
/// [`ContentFactory::create`], possibly on a worker thread.
pub struct PreparedContent {
    kind: ContentKind,
    renderer: Box<dyn ContentRenderer>,
}

impl std::fmt::Debug for PreparedContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedContent")
            .field("kind", &self.kind)
            .field("size", &self.renderer.size())
            .finish()
    }
}

impl PreparedContent {
    /// The content kind that chose the provider.
    pub fn kind(&self) -> ContentKind {
        self.kind
    }

    /// Natural size in pixels.
    pub fn size(&self) -> Size {
        self.renderer.size()
    }

    /// How long the upload took, when the GPU measured it (see
    /// [`Gpu::set_measure_uploads`]).
    pub fn upload_timings(&self) -> Option<crate::UploadTimings> {
        self.renderer.upload_timings()
    }
}

/// Creates content renderers away from the render thread, so decoding
/// threads can also do the GPU upload of the next image. Cheap to clone,
/// `Send` and `Sync`.
#[derive(Clone)]
pub struct ContentFactory {
    gpu: Arc<Gpu>,
    providers: ProviderRegistry,
    target: TargetInfo,
}

impl std::fmt::Debug for ContentFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentFactory")
            .field("providers", &self.providers)
            .finish_non_exhaustive()
    }
}

impl ContentFactory {
    /// Dispatch `input` to the provider registered for its kind and build
    /// its GPU resources. Fails with [`Error::Unsupported`] for a kind no
    /// provider handles.
    pub fn create(&self, input: &RenderInput) -> Result<PreparedContent> {
        let ctx = ProviderContext {
            gpu: &self.gpu,
            target: self.target,
        };
        let renderer = self.providers.create(&ctx, input)?;
        Ok(PreparedContent {
            kind: input.kind(),
            renderer,
        })
    }
}

/// Draws content and interface into a window surface or an offscreen
/// texture.
pub struct Renderer {
    gpu: Arc<Gpu>,
    target: Target,
    format: wgpu::TextureFormat,
    encode_srgb: bool,
    width: u32,
    height: u32,
    settings: RenderSettings,
    providers: ProviderRegistry,
    content: Option<PreparedContent>,
    display: Arc<DisplayPipelines>,
    frame_uniform: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    depth: Option<(wgpu::Texture, wgpu::TextureView)>,
    ui: UiRenderer,
    measurer: TextMeasurer,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("gpu", &self.gpu)
            .field("format", &self.format)
            .field("size", &(self.width, self.height))
            .field("content", &self.content)
            .finish_non_exhaustive()
    }
}

/// The frame bind group (group 0 of the image pipelines).
pub(crate) fn frame_bind_group(
    gpu: &Gpu,
    pipes: &Pipelines,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout: &pipes.frame_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&pipes.sampler),
            },
        ],
    })
}

fn offscreen_texture(
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Pick the surface format: 8-bit sRGB when offered (the hardware encodes
/// on store and blending happens in linear light), else the first format
/// with encoding in the shaders.
fn surface_format(caps: &wgpu::SurfaceCapabilities) -> Option<wgpu::TextureFormat> {
    let preferred = [
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ];
    preferred
        .into_iter()
        .find(|f| caps.formats.contains(f))
        .or_else(|| {
            caps.formats
                .iter()
                .copied()
                .find(wgpu::TextureFormat::is_srgb)
        })
        .or_else(|| caps.formats.first().copied())
}

fn present_mode(caps: &wgpu::SurfaceCapabilities, vsync: bool) -> wgpu::PresentMode {
    if vsync {
        // FIFO is the one mode every platform supports and never tears.
        wgpu::PresentMode::Fifo
    } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::AutoNoVsync
    }
}

impl Renderer {
    /// A renderer drawing into an offscreen texture of `width` x `height`
    /// pixels, read back with [`Renderer::read_pixels`].
    pub fn offscreen(
        gpu: Arc<Gpu>,
        width: u32,
        height: u32,
        settings: RenderSettings,
    ) -> Result<Self> {
        Self::offscreen_with_format(gpu, width, height, OFFSCREEN_FORMAT, settings)
    }

    /// An offscreen renderer with another 8-bit RGBA format. A non-sRGB
    /// format exercises the shader encoding used for such window surfaces.
    pub(crate) fn offscreen_with_format(
        gpu: Arc<Gpu>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        settings: RenderSettings,
    ) -> Result<Self> {
        let (texture, view) = offscreen_texture(&gpu, width, height, format);
        Self::build(
            gpu,
            Target::Offscreen { texture, view },
            format,
            width,
            height,
            settings,
        )
    }

    /// Open the GPU for `window` and create a renderer presenting to it.
    ///
    /// Adapters are tried in the order [`GpuOptions`] describes, keeping
    /// only those that can present to the window. When opening a device or
    /// configuring the surface fails on one, the next one is tried.
    ///
    /// `window` is anything with raw window and display handles, for
    /// example winit's `Arc<Window>`; this crate does not depend on winit.
    /// `width` and `height` are the window's inner size in physical pixels.
    /// On macOS call this on the main thread.
    pub fn for_window<W>(
        window: W,
        width: u32,
        height: u32,
        options: &GpuOptions,
        settings: RenderSettings,
    ) -> Result<Self>
    where
        W: wgpu::rwh::HasWindowHandle
            + wgpu::rwh::HasDisplayHandle
            + Clone
            + std::fmt::Debug
            + Send
            + Sync
            + 'static,
    {
        let mut attempt = 0;
        let (instance, surface, candidates) = loop {
            let (instance, surface) = gpu::window_instance(window.clone(), options)?;
            let candidates = gpu::candidates(&instance, Some(&surface), options);
            if !gpu::wait_for_discrete(options, &candidates, attempt) {
                break (instance, surface, candidates);
            }
            attempt += 1;
        };
        let mut surface = Some(surface);
        let mut failures = Vec::new();
        for adapter in candidates {
            let gpu = match Gpu::open(instance.clone(), adapter) {
                Ok(gpu) => Arc::new(gpu),
                Err(err) => {
                    failures.push(err.to_string());
                    continue;
                }
            };
            let target = window.clone();
            let factory: SurfaceFactory = Box::new(move |instance: &wgpu::Instance| {
                instance
                    .create_surface(wgpu::SurfaceTarget::from_window_without_display(
                        target.clone(),
                    ))
                    .map_err(|e| Error::Backend(format!("cannot recreate the window surface: {e}")))
            });
            // A failed attempt drops its surface: the next one gets a new one.
            let this_surface = match surface.take() {
                Some(s) => s,
                None => factory(&instance)?,
            };
            let summary = gpu.describe();
            match Self::with_surface_impl(
                gpu,
                this_surface,
                Some(factory),
                width,
                height,
                settings.clone(),
            ) {
                Ok(renderer) => return Ok(renderer),
                Err(err) => failures.push(format!("{summary}: {err}")),
            }
        }
        Err(gpu::no_adapter(options, &failures))
    }

    /// A renderer presenting to `surface`, which must come from
    /// `gpu.instance()` and be compatible with its adapter. A lost surface
    /// cannot be recreated by this renderer; [`Renderer::render`] then
    /// returns an error and the caller creates a new surface.
    pub fn with_surface(
        gpu: Arc<Gpu>,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        settings: RenderSettings,
    ) -> Result<Self> {
        Self::with_surface_impl(gpu, surface, None, width, height, settings)
    }

    fn with_surface_impl(
        gpu: Arc<Gpu>,
        surface: wgpu::Surface<'static>,
        factory: Option<SurfaceFactory>,
        width: u32,
        height: u32,
        settings: RenderSettings,
    ) -> Result<Self> {
        let caps = surface.get_capabilities(gpu.adapter());
        let format = surface_format(&caps)
            .ok_or_else(|| Error::Backend("the surface offers no texture format".into()))?;
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: present_mode(&caps, settings.vsync),
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        if width > 0 && height > 0 {
            // In a scope, so a failure reaches the caller (and lets
            // `for_window` try the next adapter).
            gpu.scoped("window surface configuration", || {
                surface.configure(gpu.device(), &config);
                Ok(())
            })?;
        }
        Self::build(
            gpu,
            Target::Surface {
                surface,
                config,
                factory,
            },
            format,
            width,
            height,
            settings,
        )
    }

    fn build(
        gpu: Arc<Gpu>,
        target: Target,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        settings: RenderSettings,
    ) -> Result<Self> {
        let scope = Arc::clone(&gpu);
        scope.scoped("renderer setup", move || {
            Self::build_scoped(gpu, target, format, width, height, settings)
        })
    }

    fn build_scoped(
        gpu: Arc<Gpu>,
        target: Target,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        settings: RenderSettings,
    ) -> Result<Self> {
        let pipes = gpu.pipelines();
        let display = pipes.display(gpu.device(), format);
        let frame_uniform = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame params"),
            size: FRAME_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = frame_bind_group(&gpu, pipes, &frame_uniform);
        let fonts = FontSet::shared(settings.font);
        let ui = UiRenderer::new(&gpu, Arc::clone(&fonts));
        Ok(Self {
            gpu,
            target,
            format,
            encode_srgb: !format.is_srgb(),
            width,
            height,
            settings,
            providers: ProviderRegistry::with_builtin(),
            content: None,
            display,
            frame_uniform,
            frame_group,
            depth: None,
            ui,
            measurer: TextMeasurer::from_set(fonts),
        })
    }

    /// The shared GPU.
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// Color target format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Target size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The whole target as a viewport rectangle.
    pub fn full_viewport(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f32, self.height as f32)
    }

    /// Facts about the color target for provider pipelines.
    pub fn target_info(&self) -> TargetInfo {
        TargetInfo {
            color_format: self.format,
            depth_format: DEPTH_FORMAT,
            width: self.width,
            height: self.height,
        }
    }

    /// Follow a window resize (physical pixels). A zero size is allowed
    /// (minimized); frames are skipped until the size is non-zero.
    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        match &mut self.target {
            Target::Surface {
                surface, config, ..
            } => {
                if width > 0 && height > 0 {
                    config.width = width;
                    config.height = height;
                    surface.configure(self.gpu.device(), config);
                }
            }
            Target::Offscreen { texture, view } => {
                let (t, v) = offscreen_texture(&self.gpu, width, height, self.format);
                *texture = t;
                *view = v;
            }
        }
    }

    /// Current settings.
    pub fn settings(&self) -> &RenderSettings {
        &self.settings
    }

    /// Replace the settings. A vsync change reconfigures the surface; a font
    /// change takes effect for the next frame and for
    /// [`Renderer::text_measurer`].
    pub fn set_settings(&mut self, settings: RenderSettings) {
        if settings.vsync != self.settings.vsync {
            if let Target::Surface {
                surface, config, ..
            } = &mut self.target
            {
                let caps = surface.get_capabilities(self.gpu.adapter());
                config.present_mode = present_mode(&caps, settings.vsync);
                if self.width > 0 && self.height > 0 {
                    surface.configure(self.gpu.device(), config);
                }
            }
        }
        if settings.font != self.settings.font {
            let fonts = FontSet::shared(settings.font);
            self.ui.set_fonts(Arc::clone(&fonts));
            self.measurer = TextMeasurer::from_set(fonts);
        }
        self.settings = settings;
    }

    /// The provider registry. Register providers before setting content of
    /// their kind.
    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    /// The provider registry, for registering custom providers.
    pub fn providers_mut(&mut self) -> &mut ProviderRegistry {
        &mut self.providers
    }

    /// A factory that creates content for this renderer on any thread.
    pub fn content_factory(&self) -> ContentFactory {
        ContentFactory {
            gpu: Arc::clone(&self.gpu),
            providers: self.providers.clone(),
            target: self.target_info(),
        }
    }

    /// Show decoded content. Uploads it on this thread; see
    /// [`Renderer::content_factory`] to upload elsewhere. Fails with
    /// [`Error::Unsupported`] when no provider handles its kind, keeping the
    /// previous content.
    pub fn set_content(&mut self, content: Content) -> Result<()> {
        self.set_input(&RenderInput::Content(content))
    }

    /// Show any input, dispatched to the provider for its kind.
    pub fn set_input(&mut self, input: &RenderInput) -> Result<()> {
        let prepared = self.content_factory().create(input)?;
        self.set_prepared(prepared);
        Ok(())
    }

    /// Show content prepared by a [`ContentFactory`]. Playback starts at
    /// time zero.
    pub fn set_prepared(&mut self, content: PreparedContent) {
        self.content = Some(content);
    }

    /// Remove the content; frames show the background and interface only.
    pub fn clear_content(&mut self) {
        self.content = None;
    }

    /// Kind of the content shown, if any.
    pub fn content_kind(&self) -> Option<ContentKind> {
        self.content.as_ref().map(|c| c.kind)
    }

    /// Natural size of the content shown, for [`ViewState`] math.
    pub fn content_size(&self) -> Option<Size> {
        self.content.as_ref().map(|c| c.renderer.size())
    }

    /// The frame of the content on screen: its index among the frames of an
    /// animation, 0 for a still. `None` without content.
    pub fn content_frame(&self) -> Option<usize> {
        self.content.as_ref().map(|c| c.renderer.frame_index())
    }

    /// Advance animated content by `dt` (the real time since the previous
    /// call). Returns how long until the next redraw is due, or `None` when
    /// the picture will not change by itself; sleep that long between
    /// frames.
    pub fn update(&mut self, dt: Duration) -> Option<Duration> {
        self.content.as_mut().and_then(|c| c.renderer.update(dt))
    }

    /// Where `view` places the content in `viewport`: the rectangle from
    /// [`ViewState::image_rect`] with its origin rounded to whole pixels.
    pub fn placement(&self, view: &ViewState, viewport: Rect) -> Option<Placement> {
        let size = self.content_size()?;
        Some(placement_for(view, size, viewport))
    }

    /// A text measurer that matches what this renderer draws.
    pub fn text_measurer(&self) -> TextMeasurer {
        self.measurer.clone()
    }

    /// Draw one frame: background, the content placed by `view` inside
    /// `viewport` (physical pixels, normally [`Renderer::full_viewport`]
    /// or the area above a panel), then `ui` on top. Presents to the
    /// window when the target is a surface.
    pub fn render(
        &mut self,
        view: &ViewState,
        viewport: Rect,
        ui: &DrawList,
    ) -> Result<FrameStatus> {
        if self.gpu.is_lost() {
            return Err(Error::Backend("the GPU device was lost".into()));
        }
        if self.width == 0 || self.height == 0 {
            return Ok(FrameStatus::Skipped);
        }
        // Errors raised by this frame's calls on this thread come back
        // here; errors of other threads sharing the device do not.
        let gpu = Arc::clone(&self.gpu);
        let status = gpu.scoped("render", || {
            let Some((frame, surface_texture)) = self.acquire()? else {
                return Ok(FrameStatus::Skipped);
            };
            let result = self.draw_frame(&frame, view, viewport, ui);
            if let Some(texture) = surface_texture {
                if result.is_ok() {
                    self.gpu.queue().present(texture);
                }
            }
            result.map(|()| FrameStatus::Drawn)
        })?;
        Ok(status)
    }

    /// The first failure of a [`TextureSink::upload`] since the last call.
    /// The sink interface cannot return errors; a failed upload leaves the
    /// id unset, so its draws are skipped.
    pub fn take_texture_error(&mut self) -> Option<String> {
        self.ui.upload_error.take()
    }

    /// Get the view to draw into, reconfiguring or recreating the surface
    /// once when it is outdated or lost.
    fn acquire(&mut self) -> Result<Option<(wgpu::TextureView, Option<wgpu::SurfaceTexture>)>> {
        let gpu = Arc::clone(&self.gpu);
        match &mut self.target {
            Target::Offscreen { view, .. } => Ok(Some((view.clone(), None))),
            Target::Surface {
                surface,
                config,
                factory,
            } => {
                for attempt in 0..2 {
                    match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(texture) => {
                            let view = texture.texture.create_view(&Default::default());
                            return Ok(Some((view, Some(texture))));
                        }
                        wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                            if attempt == 0 {
                                // Reconfigure for the next frame; this one
                                // is still presentable but drop it first.
                                drop(texture);
                                surface.configure(gpu.device(), config);
                                continue;
                            }
                            let view = texture.texture.create_view(&Default::default());
                            return Ok(Some((view, Some(texture))));
                        }
                        wgpu::CurrentSurfaceTexture::Timeout
                        | wgpu::CurrentSurfaceTexture::Occluded => return Ok(None),
                        wgpu::CurrentSurfaceTexture::Outdated => {
                            surface.configure(gpu.device(), config);
                        }
                        wgpu::CurrentSurfaceTexture::Lost => {
                            let Some(factory) = factory else {
                                return Err(Error::Backend("the window surface was lost".into()));
                            };
                            *surface = factory(gpu.instance())?;
                            surface.configure(gpu.device(), config);
                        }
                        wgpu::CurrentSurfaceTexture::Validation => {
                            return Err(Error::Backend(
                                "acquiring the window image failed validation".into(),
                            ));
                        }
                    }
                }
                Ok(None)
            }
        }
    }

    fn ensure_depth(&mut self) {
        let fits = self
            .depth
            .as_ref()
            .is_some_and(|(t, _)| t.width() == self.width && t.height() == self.height);
        if fits {
            return;
        }
        let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.depth = Some((texture, view));
    }

    fn draw_frame(
        &mut self,
        target_view: &wgpu::TextureView,
        view: &ViewState,
        viewport: Rect,
        ui: &DrawList,
    ) -> Result<()> {
        let gpu = Arc::clone(&self.gpu);
        let params = frame_params((self.width, self.height), &self.settings, self.encode_srgb);
        gpu.queue()
            .write_buffer(&self.frame_uniform, 0, bytemuck::cast_slice(&params));
        let target = self.target_info();
        let full = self.full_viewport();
        let viewport = viewport
            .intersect(full)
            .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        let mut uses_depth = false;
        let mut placement = None;
        if let Some(content) = &mut self.content {
            if !viewport.is_empty() {
                let placed = placement_for(view, content.renderer.size(), viewport);
                let mut ctx = FrameContext {
                    gpu: &gpu,
                    encoder: &mut encoder,
                    placement: placed,
                    target,
                    settings: &self.settings,
                };
                content.renderer.prepare(&mut ctx)?;
                uses_depth = content.renderer.uses_depth();
                placement = Some(placed);
            }
        }
        if uses_depth {
            self.ensure_depth();
        }
        self.ui
            .prepare(&gpu, ui, self.width, self.height, self.encode_srgb);

        let clear = clear_value(self.settings.background, !self.encode_srgb);
        {
            let depth_attachment = if uses_depth {
                self.depth
                    .as_ref()
                    .map(|(_, view)| wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    })
            } else {
                None
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("content"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: depth_attachment,
                ..Default::default()
            });
            if let (Some(content), Some(placement)) = (&self.content, placement) {
                let [x, y, w, h] = scissor_for(viewport, self.width, self.height);
                if w > 0 && h > 0 {
                    pass.set_scissor_rect(x, y, w, h);
                    let ctx = DrawContext {
                        placement,
                        target,
                        depth_format: uses_depth.then_some(DEPTH_FORMAT),
                        settings: &self.settings,
                        frame: &self.frame_group,
                        pipelines: &self.display,
                    };
                    content.renderer.draw(&mut pass, &ctx);
                }
            }
            if !uses_depth {
                self.ui.draw(&mut pass, &self.display);
            }
        }
        if uses_depth {
            // The interface pipeline has no depth state: draw it in a
            // second pass over the finished content.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.ui.draw(&mut pass, &self.display);
        }
        gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    /// Read the offscreen target back as RGBA8 sRGB pixels. Waits for the
    /// GPU. Window targets return [`Error::Unsupported`].
    pub fn read_pixels(&self) -> Result<ImageBuf> {
        let Target::Offscreen { texture, .. } = &self.target else {
            return Err(Error::Unsupported("read_pixels on a window surface".into()));
        };
        let data = self.gpu.read_texture(texture, 0)?;
        ImageBuf::new(texture.width(), texture.height(), PixelFormat::Rgba8, data)
    }

    /// Number of interface draw calls the last frame used.
    pub fn ui_draw_calls(&self) -> usize {
        self.ui.batch_count()
    }

    /// True when an interface texture with this id is uploaded.
    pub fn has_texture(&self, id: TextureId) -> bool {
        self.ui.has_texture(id)
    }

    /// The interface instance buffer, for tests that check it is reused.
    #[cfg(test)]
    pub(crate) fn ui_instance_buffer(&self) -> Option<wgpu::Buffer> {
        self.ui.instance_buffer().cloned()
    }
}

fn placement_for(view: &ViewState, size: Size, viewport: Rect) -> Placement {
    let mut rect = view.image_rect(size, viewport);
    rect.x = rect.x.round();
    rect.y = rect.y.round();
    Placement {
        image_rect: rect,
        viewport,
        scale: view.scale(size, viewport.size()),
    }
}

impl TextureSink for Renderer {
    fn upload(&mut self, id: TextureId, image: &ImageBuf, alpha_mask: bool) {
        self.ui.upload(&self.gpu, id, image, alpha_mask);
    }

    fn remove(&mut self, id: TextureId) {
        self.ui.remove(id);
    }
}

impl TextMeasure for Renderer {
    fn measure(&self, text: &str, size: f32) -> Size {
        self.measurer.measure(text, size)
    }
}
