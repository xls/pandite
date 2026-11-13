//! Renderer providers: the plug-in point for every kind of content.
//!
//! A [`RenderProvider`] is registered in a [`ProviderRegistry`] under a
//! [`ContentKind`]. When the application shows new content the renderer
//! asks the provider for that kind to build a [`ContentRenderer`], which
//! owns the GPU resources of that one piece of content.
//!
//! Each frame the renderer calls, in order:
//! 1. [`ContentRenderer::update`] with the time since the last update. The
//!    returned duration says when the next redraw is due, so the
//!    application can sleep between animation frames.
//! 2. [`ContentRenderer::prepare`] before the render pass, with the command
//!    encoder for extra passes (shadow maps, skinning) and uniform uploads.
//! 3. [`ContentRenderer::draw`] inside the content render pass, with the
//!    placement the [`pandite_core::ViewState`] produced. When
//!    [`ContentRenderer::uses_depth`] is true that pass carries a cleared
//!    depth attachment of format [`TargetInfo::depth_format`], which is how
//!    a later `Scene3d` provider (meshes, materials, skeletal animation)
//!    plugs in without changes here.

use crate::gpu::Gpu;
use crate::pipelines::DisplayPipelines;
use crate::settings::RenderSettings;
use pandite_core::{Content, ContentKind, Error, Rect, Result, Size};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// What the renderer is asked to show.
#[derive(Clone)]
#[non_exhaustive]
pub enum RenderInput {
    /// Content from a decoder provider. Dispatched by [`Content::kind`].
    Content(Content),
    /// A reduced decode standing in for a larger image until the full
    /// resolution arrives: drawn as if it had `size` pixels, so the view
    /// does not move when the full image replaces it. Dispatched by
    /// [`Content::kind`]; the still provider supports it.
    Reduced {
        /// The reduced image.
        content: Content,
        /// The full image's size in pixels.
        size: Size,
    },
    /// Data for a provider registered by the application, dispatched by
    /// `kind`. This carries content that `pandite_core::Content` cannot
    /// express yet, for example a 3D scene before core grows a variant.
    Custom {
        /// The kind used to pick the provider.
        kind: ContentKind,
        /// Natural size in pixels, used for fit and zoom placement.
        size: Size,
        /// Provider-specific data; the provider downcasts it.
        data: Arc<dyn Any + Send + Sync>,
    },
}

impl RenderInput {
    /// The kind the provider registry dispatches on.
    pub fn kind(&self) -> ContentKind {
        match self {
            RenderInput::Content(content) | RenderInput::Reduced { content, .. } => content.kind(),
            RenderInput::Custom { kind, .. } => *kind,
        }
    }
}

impl std::fmt::Debug for RenderInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderInput::Content(content) => f
                .debug_struct("Content")
                .field("kind", &content.kind())
                .field("dimensions", &content.dimensions())
                .finish(),
            RenderInput::Reduced { content, size } => f
                .debug_struct("Reduced")
                .field("kind", &content.kind())
                .field("dimensions", &content.dimensions())
                .field("size", size)
                .finish(),
            RenderInput::Custom { kind, size, .. } => f
                .debug_struct("Custom")
                .field("kind", kind)
                .field("size", size)
                .finish_non_exhaustive(),
        }
    }
}

impl From<Content> for RenderInput {
    fn from(content: Content) -> Self {
        RenderInput::Content(content)
    }
}

/// The color target pipelines must match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetInfo {
    /// Color attachment format.
    pub color_format: wgpu::TextureFormat,
    /// Depth attachment format used when a renderer asks for depth.
    pub depth_format: wgpu::TextureFormat,
    /// Target size in physical pixels.
    pub width: u32,
    /// Target height in physical pixels.
    pub height: u32,
}

/// Where the content lands this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The content rectangle in target pixels, from
    /// [`pandite_core::ViewState::image_rect`] with the origin rounded to
    /// whole pixels so 1:1 and integer zoom map texels onto pixels exactly.
    pub image_rect: Rect,
    /// The viewport the view state was computed for. Content is clipped to it.
    pub viewport: Rect,
    /// Target pixels per content pixel.
    pub scale: f32,
}

/// Passed to [`RenderProvider::create`].
pub struct ProviderContext<'a> {
    /// The GPU to create resources on.
    pub gpu: &'a Arc<Gpu>,
    /// The color target the renderer draws into.
    pub target: TargetInfo,
}

/// Passed to [`ContentRenderer::prepare`] before the content pass.
pub struct FrameContext<'a> {
    /// The GPU.
    pub gpu: &'a Gpu,
    /// Encoder recorded before the content pass. Extra passes go here.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// Where the content lands.
    pub placement: Placement,
    /// The color target.
    pub target: TargetInfo,
    /// Current renderer settings.
    pub settings: &'a RenderSettings,
}

impl FrameContext<'_> {
    /// The part of the target the content may cover: the target rectangle
    /// intersected with the viewport. Use it to cull.
    pub fn visible_rect(&self) -> Rect {
        let full = Rect::new(
            0.0,
            0.0,
            self.target.width as f32,
            self.target.height as f32,
        );
        full.intersect(self.placement.viewport)
            .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
    }
}

/// Passed to [`ContentRenderer::draw`] inside the content pass.
pub struct DrawContext<'a> {
    /// Where the content lands.
    pub placement: Placement,
    /// The color target.
    pub target: TargetInfo,
    /// Format of the pass's depth attachment, present exactly when the
    /// renderer reported [`ContentRenderer::uses_depth`].
    pub depth_format: Option<wgpu::TextureFormat>,
    /// Current renderer settings.
    pub settings: &'a RenderSettings,
    pub(crate) frame: &'a wgpu::BindGroup,
    pub(crate) pipelines: &'a DisplayPipelines,
}

/// The GPU side of one piece of content.
pub trait ContentRenderer: Send {
    /// Natural size in content pixels, used by fit and zoom.
    fn size(&self) -> Size;

    /// Advance time by `dt`. Returns how long until the picture changes
    /// again, or `None` when it stays as it is until something else
    /// changes (a still, or a finished animation).
    fn update(&mut self, dt: Duration) -> Option<Duration>;

    /// True when [`ContentRenderer::draw`] needs a depth attachment.
    fn uses_depth(&self) -> bool {
        false
    }

    /// Upload per-frame data and record work that must run before the
    /// content pass.
    fn prepare(&mut self, ctx: &mut FrameContext<'_>) -> Result<()>;

    /// Record draw calls into the content pass. The scissor is already set
    /// to the viewport.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>);

    /// How long the upload took, when the GPU measured it (see
    /// [`crate::Gpu::set_measure_uploads`]). `None` by default.
    fn upload_timings(&self) -> Option<crate::UploadTimings> {
        None
    }
}

/// Builds a [`ContentRenderer`] for one kind of content.
pub trait RenderProvider: Send + Sync {
    /// Short stable name, for diagnostics.
    fn name(&self) -> &'static str;

    /// Create the GPU resources for `input`. Uploads happen here, so this
    /// may run on a worker thread (see [`crate::ContentFactory`]).
    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> Result<Box<dyn ContentRenderer>>;
}

/// Renderer providers keyed by [`ContentKind`].
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: HashMap<ContentKind, Arc<dyn RenderProvider>>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for (kind, provider) in &self.providers {
            map.entry(kind, &provider.name());
        }
        map.finish()
    }
}

impl ProviderRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the built-in providers: [`crate::StillProvider`] for
    /// [`ContentKind::Still`] and [`crate::AnimationProvider`] for
    /// [`ContentKind::Animation`]. [`ContentKind::Scene3d`] stays free.
    pub fn with_builtin() -> Self {
        let mut registry = Self::new();
        registry.register(ContentKind::Still, Arc::new(crate::still::StillProvider));
        registry.register(
            ContentKind::Animation,
            Arc::new(crate::anim::AnimationProvider::default()),
        );
        registry
    }

    /// Register `provider` for `kind`, returning the provider it replaces.
    pub fn register(
        &mut self,
        kind: ContentKind,
        provider: Arc<dyn RenderProvider>,
    ) -> Option<Arc<dyn RenderProvider>> {
        self.providers.insert(kind, provider)
    }

    /// Remove the provider for `kind`.
    pub fn unregister(&mut self, kind: ContentKind) -> Option<Arc<dyn RenderProvider>> {
        self.providers.remove(&kind)
    }

    /// The provider for `kind`.
    pub fn get(&self, kind: ContentKind) -> Option<&Arc<dyn RenderProvider>> {
        self.providers.get(&kind)
    }

    /// True when some provider handles `kind`.
    pub fn supports(&self, kind: ContentKind) -> bool {
        self.providers.contains_key(&kind)
    }

    /// Dispatch `input` to the provider for its kind. Fails with
    /// [`Error::Unsupported`] when no provider is registered for it.
    pub fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> Result<Box<dyn ContentRenderer>> {
        let kind = input.kind();
        let provider = self.get(kind).ok_or_else(|| {
            Error::Unsupported(format!("no renderer provider for {kind:?} content"))
        })?;
        provider.create(ctx, input)
    }
}
