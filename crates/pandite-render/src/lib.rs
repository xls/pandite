//! pandite-render: the wgpu renderer of the pandite image viewer.
//!
//! # Targets
//!
//! A [`Renderer`] draws into a window surface ([`Renderer::for_window`]
//! takes any raw-window-handle window, such as winit's `Arc<Window>`; this
//! crate does not depend on winit) or into an offscreen texture
//! ([`Renderer::offscreen`], read back with [`Renderer::read_pixels`]).
//! It follows resizes, reconfigures an outdated surface, recreates a lost
//! one, presents with FIFO when vsync is on, and renders into an sRGB
//! target so blending happens in linear light.
//!
//! # Frame
//!
//! ```text
//! let mut r = Renderer::for_window(window.clone(), w, h, &GpuOptions::default(), settings)?;
//! r.set_content(content)?;                  // provider for content.kind()
//! r.upload(TextureId(7), &thumbnail, false); // TextureSink, for the interface
//! let next = r.update(dt);                  // Some(time to next frame) while animating
//! r.render(&view, r.full_viewport(), &list)?; // content, then interface; presents
//! r.resize(new_w, new_h);                   // on window resize
//! ```
//!
//! # Providers
//!
//! Content is drawn by renderer providers ([`RenderProvider`], creating a
//! [`ContentRenderer`]) registered per [`pandite_core::ContentKind`] in a
//! [`ProviderRegistry`]. Built in: [`StillProvider`] and
//! [`AnimationProvider`]. The trait has per-frame `update(dt)` returning the
//! next deadline, a `prepare` step with the command encoder, and `draw` with
//! the render pass, the placement and an optional depth attachment, so a
//! 3D scene provider (meshes, materials, skeletal animation) fits later.
//! Unknown kinds fail with [`pandite_core::Error::Unsupported`].
//!
//! # Zoom quality
//!
//! - Content is stored as premultiplied linear color: `Rgba8UnormSrgb` for
//!   8-bit sources, `Rgba16Float` for 16-bit and floating point sources.
//! - Every texture has a full mip chain, rendered on the GPU from the level
//!   above with an exact box filter in linear light (see [`GpuImage`]).
//! - Minification samples trilinearly with 16x anisotropic filtering and a
//!   small negative level-of-detail bias ([`RenderSettings::lod_bias`]).
//! - Magnification is bilinear up to [`RenderSettings::crisp_zoom`]
//!   (default [`CRISP_ZOOM_DEFAULT`], 3 screen pixels per image pixel). From
//!   there on each image pixel is drawn as a flat square with a one screen
//!   pixel antialiased seam: crisp for pixel peeping, without the uneven
//!   pixel widths of plain nearest sampling at fractional zoom.
//! - The image origin is rounded to whole pixels, so 1:1 and integer zoom
//!   map texels exactly onto pixels.
//! - Images larger than the device texture limit are tiled with overlapping
//!   gutters, plus a downscaled overview used when zoomed out. When the
//!   tiles do not fit in GPU memory only the overview is kept (documented
//!   fallback: the image shows at reduced resolution).
//! - Floating point (HDR) content is scaled by [`RenderSettings::exposure`]
//!   and tone mapped: identity up to 0.75, then a smooth shoulder to white.
//! - Transparent content is composited over a checkerboard
//!   ([`Checkerboard`]) or the background color, in linear light.
//! - Uploads copy the pixels once into strip-sized staging textures and do
//!   conversion and mip generation in render passes. Nothing waits for the
//!   GPU, and [`ContentFactory`] runs uploads on a worker thread.
//!   Animations upload the next frame while the current one is shown.
//!
//! # Interface
//!
//! The [`pandite_core::DrawList`] is drawn on top in one instanced pipeline:
//! rounded rectangles and borders with antialiased edges from a signed
//! distance, textured images with tint, alpha masks tinted as icons, clip
//! rectangles as scissors, and text from a glyph atlas rasterized by
//! fontdue. The renderer implements [`pandite_core::TextureSink`] and
//! [`pandite_core::TextMeasure`]; [`TextMeasurer`] measures with the same
//! layout code the drawing uses.
//!
//! The embedded font is Fira Sans Regular (subset), under the SIL Open
//! Font License 1.1 shipped as `fonts/FiraSans-OFL.txt`. By default the
//! system interface font is used when present ([`FontChoice`]).

#![forbid(unsafe_code)]

mod anim;
mod color;
mod gpu;
mod image;
mod pipelines;
mod provider;
mod renderer;
mod settings;
mod still;
mod text;
mod ui;

#[cfg(test)]
mod tests;

pub use wgpu;

pub use anim::{AnimationProvider, ANIMATION_CACHE_BYTES};
pub use gpu::{Gpu, GpuOptions};
pub use image::GpuImage;
pub use pipelines::DEPTH_FORMAT;
pub use provider::{
    ContentRenderer, DrawContext, FrameContext, Placement, ProviderContext, ProviderRegistry,
    RenderInput, RenderProvider, TargetInfo,
};
pub use renderer::{ContentFactory, FrameStatus, PreparedContent, Renderer, OFFSCREEN_FORMAT};
pub use settings::{Checkerboard, RenderSettings, CRISP_ZOOM_DEFAULT};
pub use still::StillProvider;
pub use text::{FontChoice, TextMeasurer};
