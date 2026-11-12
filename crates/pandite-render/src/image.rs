//! Content textures: upload, tiling, mip generation and drawing.
//!
//! Upload path, all on the GPU after one copy of the pixels:
//! 1. Rows are written in strips of at most [`STRIP_BYTES`] into a small
//!    staging texture in the source encoding (rgba8 sRGB, rgba16 unsigned
//!    or rgba32 float). Each strip is converted by a render pass into the
//!    storage texture as premultiplied linear color: `Rgba8UnormSrgb` for
//!    8-bit content, `Rgba16Float` for 16-bit and floating point content.
//!    Strips bound the temporary memory and no CPU pixel loop runs.
//! 2. Every mip level is rendered from the level above with an exact box
//!    filter (partial coverage for odd sizes). The sRGB view decodes on
//!    load and encodes on store, so averaging happens in linear light, and
//!    premultiplied storage keeps transparent edges free of dark fringes.
//! 3. Images wider or taller than the device limit are split into tiles
//!    that overlap by a gutter of at least [`MIN_GUTTER`] pixels, wide
//!    enough for two texels at the coarsest level a tile is sampled at, so
//!    filtering is seamless across tile edges. A downscaled overview
//!    texture (rendered from the tiles' mip levels) is drawn instead
//!    whenever the zoom is small enough. If the tiles do not fit in GPU
//!    memory, only the overview is kept (reduced mode).
//!
//! Nothing waits for the GPU. [`GpuImage::upload`] may run on a worker
//! thread.

use crate::gpu::Gpu;
use crate::pipelines::{
    Pipelines, COLOR_FORMAT, DRAW_PARAMS_SIZE, FRAME_PARAMS_SIZE, MASK_FORMAT, WIDE_FORMAT,
};
use crate::provider::{DrawContext, FrameContext, Placement};
use crate::settings::RenderSettings;
use pandite_core::{Error, ImageBuf, PixelFormat, Rect, Result, Size};

/// Smallest overlap between neighboring tiles, in pixels.
pub(crate) const MIN_GUTTER: u32 = 16;
/// Tile interiors start on multiples of this, so tile mip levels line up
/// with the overview's texels.
const TILE_ALIGN: u32 = 64;
/// Largest staging strip uploaded at once, in bytes.
pub(crate) const STRIP_BYTES: u64 = 32 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

struct Tile {
    texture: wgpu::Texture,
    /// Image pixels stored in the texture.
    region: Span,
    /// Image pixels this tile draws (the region minus the gutters).
    interior: Span,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// What the storage texture holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Storage {
    /// Premultiplied linear color from the given source layout.
    Color(PixelFormat),
    /// Coverage only, from the source alpha channel.
    Mask,
}

/// One raster image on the GPU, ready to be drawn with any zoom.
///
/// A building block for providers: the built-in still and animation
/// providers are thin wrappers around it, and a custom provider that
/// rasterizes its content (vector, document) can reuse it.
pub struct GpuImage {
    width: u32,
    height: u32,
    storage: Storage,
    format: wgpu::TextureFormat,
    /// Tile plan: (region, interior) per tile.
    spans: Vec<(Span, Span)>,
    /// Full resolution textures, one per span. Empty in reduced mode.
    tiles: Vec<Tile>,
    /// Downscaled whole image for tiled images, and its level shift.
    overview: Option<(Tile, u32)>,
    /// Indices into `tiles` drawn this frame; `usize::MAX` is the overview.
    visible: Vec<usize>,
}

impl std::fmt::Debug for GpuImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("tiles", &self.tiles.len())
            .field("overview", &self.overview.as_ref().map(|o| o.1))
            .finish()
    }
}

fn mip_levels(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// Split one axis into tile spans: (region start, region length, interior
/// start, interior length). Regions never exceed `max`.
fn plan_axis(len: u32, max: u32, gutter: u32) -> Vec<(u32, u32, u32, u32)> {
    if len <= max {
        return vec![(0, len, 0, len)];
    }
    let room = max.saturating_sub(2 * gutter).max(1);
    let stride = if room >= TILE_ALIGN {
        room / TILE_ALIGN * TILE_ALIGN
    } else {
        room
    };
    let mut out = Vec::new();
    let mut start = 0;
    while start < len {
        let end = start.saturating_add(stride).min(len);
        let r0 = start.saturating_sub(gutter);
        let r1 = end.saturating_add(gutter).min(len);
        out.push((r0, r1 - r0, start, end - start));
        start = end;
    }
    out
}

/// Level shift of the overview: the smallest power of two that brings the
/// image within `max` on both axes.
fn overview_shift(width: u32, height: u32, max: u32) -> u32 {
    let mut shift = 1;
    while (width >> shift) > max || (height >> shift) > max {
        shift += 1;
    }
    shift
}

fn create_storage(
    gpu: &Gpu,
    label: &str,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_levels(w, h),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn new_tile(gpu: &Gpu, region: Span, interior: Span, format: wgpu::TextureFormat) -> Tile {
    let pipes = gpu.pipelines();
    let texture = create_storage(gpu, "image tile", region.w, region.h, format);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let uniform = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("tile draw params"),
        size: DRAW_PARAMS_SIZE,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = texture_bind_group(gpu, pipes, &view, &uniform);
    Tile {
        texture,
        region,
        interior,
        uniform,
        bind_group,
    }
}

/// A bind group for the shared texture layout (group 1 of the image and
/// interface pipelines).
pub(crate) fn texture_bind_group(
    gpu: &Gpu,
    pipes: &Pipelines,
    view: &wgpu::TextureView,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("texture"),
        layout: &pipes.texture_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

/// How the image shader samples a texture: the `MODE_` constants in
/// `image.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sampling {
    /// Less than one screen pixel per texel: trilinear over the mip chain
    /// with the level of detail bias.
    Minify = 0,
    /// One screen pixel per texel or more: bilinear on level 0, named
    /// explicitly so 1:1 does not depend on level of detail precision.
    Magnify = 1,
    /// From [`RenderSettings::crisp_zoom`] on: flat texels with a one
    /// pixel seam.
    Crisp = 2,
}

impl Sampling {
    /// The mode for `pixels_per_texel` screen pixels per texel, with crisp
    /// sampling from `crisp_from` on.
    fn for_scale(pixels_per_texel: f32, crisp_from: f32) -> Self {
        if pixels_per_texel >= crisp_from {
            Self::Crisp
        } else if pixels_per_texel >= 1.0 {
            Self::Magnify
        } else {
            Self::Minify
        }
    }
}

/// Per-draw uniforms, laid out as `DrawParams` in `image.wgsl`.
struct DrawParams {
    dst: [f32; 4],
    uv: [f32; 4],
    tex_size: [f32; 2],
    checker_origin: [f32; 2],
    mode: Sampling,
    hdr: bool,
    lod: f32,
}

impl DrawParams {
    fn write(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer) {
        let words: [f32; 16] = [
            self.dst[0],
            self.dst[1],
            self.dst[2],
            self.dst[3],
            self.uv[0],
            self.uv[1],
            self.uv[2],
            self.uv[3],
            self.tex_size[0],
            self.tex_size[1],
            self.checker_origin[0],
            self.checker_origin[1],
            f32::from_bits(self.mode as u32),
            f32::from_bits(u32::from(self.hdr)),
            self.lod,
            0.0,
        ];
        queue.write_buffer(buffer, 0, bytemuck::cast_slice(&words));
    }
}

/// Frame uniforms, laid out as `FrameParams` in `image.wgsl`.
pub(crate) fn frame_params(
    target: (u32, u32),
    settings: &RenderSettings,
    encode_srgb: bool,
) -> [f32; 16] {
    let (a, b, cell) = match settings.checkerboard {
        Some(c) => (
            crate::color::linear(c.light),
            crate::color::linear(c.dark),
            c.cell.max(1.0),
        ),
        None => {
            let bg = crate::color::linear(settings.background);
            (bg, bg, 8.0)
        }
    };
    let exposure = if settings.exposure.is_finite() {
        settings.exposure.clamp(-20.0, 20.0).exp2()
    } else {
        1.0
    };
    let lod_bias = if settings.lod_bias.is_finite() {
        settings.lod_bias.clamp(-4.0, 4.0)
    } else {
        0.0
    };
    [
        target.0 as f32,
        target.1 as f32,
        exposure,
        f32::from_bits(u32::from(encode_srgb)),
        a[0],
        a[1],
        a[2],
        1.0,
        b[0],
        b[1],
        b[2],
        1.0,
        cell,
        lod_bias,
        0.0,
        0.0,
    ]
}

impl GpuImage {
    /// Upload a decoded image. The pixels are copied once into staging
    /// memory; conversion and mip generation run on the GPU.
    pub fn upload(gpu: &Gpu, image: &ImageBuf) -> Result<GpuImage> {
        Self::upload_with_limit(gpu, image, gpu.max_texture_size())
    }

    /// Upload with a custom texture side limit (tests force tiling with it).
    ///
    /// When a tiled image does not fit in GPU memory, the upload falls back
    /// to reduced mode: tiles are converted one at a time into the
    /// downscaled overview and freed, and only the overview is kept. The
    /// image then displays at the overview's resolution at every zoom.
    pub(crate) fn upload_with_limit(
        gpu: &Gpu,
        image: &ImageBuf,
        max_side: u32,
    ) -> Result<GpuImage> {
        match Self::upload_mode(gpu, image, max_side, false) {
            Ok(out) => Ok(out),
            Err(err) if image.width() > max_side || image.height() > max_side => {
                Self::upload_mode(gpu, image, max_side, true).map_err(|_| err)
            }
            Err(err) => Err(err),
        }
    }

    /// Upload in full or reduced mode.
    pub(crate) fn upload_mode(
        gpu: &Gpu,
        image: &ImageBuf,
        max_side: u32,
        reduced: bool,
    ) -> Result<GpuImage> {
        let storage = Storage::Color(image.format());
        gpu.scoped("image upload", || {
            let mut out = Self::allocate(
                gpu,
                image.width(),
                image.height(),
                storage,
                max_side,
                reduced,
            )?;
            out.fill(gpu, image)?;
            Ok(out)
        })
    }

    /// Upload only the alpha channel, as coverage for tinted icons.
    pub(crate) fn upload_mask(gpu: &Gpu, image: &ImageBuf) -> Result<GpuImage> {
        let max = gpu.max_texture_size();
        if image.width() > max || image.height() > max {
            return Err(Error::TooLarge {
                width: image.width(),
                height: image.height(),
            });
        }
        gpu.scoped("mask upload", || {
            let mut out = Self::allocate(
                gpu,
                image.width(),
                image.height(),
                Storage::Mask,
                max,
                false,
            )?;
            out.fill(gpu, image)?;
            Ok(out)
        })
    }

    /// Replace the pixels with `image`, reusing the textures when the size
    /// and pixel format match (animation frames). Otherwise reallocates.
    pub fn update(&mut self, gpu: &Gpu, image: &ImageBuf) -> Result<()> {
        let same = self.width == image.width()
            && self.height == image.height()
            && self.storage == Storage::Color(image.format());
        if !same {
            *self = Self::upload(gpu, image)?;
            return Ok(());
        }
        gpu.scoped("image update", || self.fill(gpu, image))
    }

    fn allocate(
        gpu: &Gpu,
        width: u32,
        height: u32,
        storage: Storage,
        max_side: u32,
        reduced: bool,
    ) -> Result<Self> {
        let format = match storage {
            Storage::Color(PixelFormat::Rgba8) => COLOR_FORMAT,
            Storage::Color(PixelFormat::Rgba16 | PixelFormat::RgbaF32) => {
                gpu.pipelines().wide_format
            }
            Storage::Color(other) => {
                return Err(Error::Unsupported(format!("pixel format {other:?}")));
            }
            Storage::Mask => MASK_FORMAT,
        };
        if width == 0 || height == 0 {
            return Err(Error::Invalid(format!("empty image: {width} x {height}")));
        }
        let max_side = max_side.max(8 * MIN_GUTTER);
        let shift = overview_shift(width, height, max_side);
        // Tiles are drawn down to the overview's scale, mip level `shift`:
        // keep two texels of overlap at that level.
        let gutter = (2u32 << shift).clamp(MIN_GUTTER, max_side / 8);
        let xs = plan_axis(width, max_side, gutter);
        let ys = plan_axis(height, max_side, gutter);
        let mut spans = Vec::with_capacity(xs.len() * ys.len());
        for &(ry, rh, iy, ih) in &ys {
            for &(rx, rw, ix, iw) in &xs {
                let region = Span {
                    x: rx,
                    y: ry,
                    w: rw,
                    h: rh,
                };
                let interior = Span {
                    x: ix,
                    y: iy,
                    w: iw,
                    h: ih,
                };
                spans.push((region, interior));
            }
        }
        let tiled = spans.len() > 1;
        let tiles = if reduced && tiled {
            Vec::new()
        } else {
            spans
                .iter()
                .map(|&(region, interior)| new_tile(gpu, region, interior, format))
                .collect()
        };
        let overview = if tiled {
            let ow = (width >> shift).max(1);
            let oh = (height >> shift).max(1);
            let whole = Span {
                x: 0,
                y: 0,
                w: ow,
                h: oh,
            };
            Some((new_tile(gpu, whole, whole, format), shift))
        } else {
            None
        };
        Ok(Self {
            width,
            height,
            storage,
            format,
            spans,
            tiles,
            overview,
            visible: Vec::new(),
        })
    }

    fn fill(&mut self, gpu: &Gpu, image: &ImageBuf) -> Result<()> {
        if self.tiles.is_empty() {
            return self.fill_reduced(gpu, image);
        }
        match self.storage {
            Storage::Color(_) => {
                for tile in &self.tiles {
                    convert_into(gpu, image, tile)?;
                }
            }
            Storage::Mask => {
                let tile = &self.tiles[0];
                write_mask(gpu, image, tile)?;
            }
        }
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("image mips"),
            });
        for tile in &self.tiles {
            encode_mips(gpu, &mut encoder, &tile.texture);
        }
        if let Some((overview, _)) = &self.overview {
            let tiles: Vec<&Tile> = self.tiles.iter().collect();
            self.draw_into_overview(gpu, &mut encoder, &tiles, true);
            encode_mips(gpu, &mut encoder, &overview.texture);
        }
        gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    /// Reduced mode: one full resolution tile at a time is converted, drawn
    /// into the overview and freed, so peak memory is one tile plus the
    /// overview.
    fn fill_reduced(&mut self, gpu: &Gpu, image: &ImageBuf) -> Result<()> {
        let Some((overview, _)) = &self.overview else {
            return Err(Error::Invalid("reduced image without overview".into()));
        };
        for (index, &(region, interior)) in self.spans.iter().enumerate() {
            let tile = new_tile(gpu, region, interior, self.format);
            convert_into(gpu, image, &tile)?;
            let mut encoder =
                gpu.device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("reduced tile"),
                    });
            encode_mips(gpu, &mut encoder, &tile.texture);
            self.draw_into_overview(gpu, &mut encoder, &[&tile], index == 0);
            gpu.queue().submit([encoder.finish()]);
            // Free this tile before the next one is allocated.
            drop(tile);
            gpu.wait_idle()?;
        }
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("overview mips"),
            });
        encode_mips(gpu, &mut encoder, &overview.texture);
        gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    /// Record a pass that renders `tiles` at the overview's mip level into
    /// level 0 of the overview texture. `clear` starts from transparent.
    fn draw_into_overview(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        tiles: &[&Tile],
        clear: bool,
    ) {
        let Some((overview, shift)) = &self.overview else {
            return;
        };
        let pipes = gpu.pipelines();
        let (ow, oh) = (overview.region.w, overview.region.h);
        let frame = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("overview frame"),
            size: FRAME_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = frame_params((ow, oh), &RenderSettings::default(), false);
        gpu.queue()
            .write_buffer(&frame, 0, bytemuck::cast_slice(&params));
        let frame_group = crate::renderer::frame_bind_group(gpu, pipes, &frame);
        let sx = ow as f32 / self.width as f32;
        let sy = oh as f32 / self.height as f32;
        for tile in tiles {
            let i = tile.interior;
            let r = tile.region;
            DrawParams {
                dst: [
                    i.x as f32 * sx,
                    i.y as f32 * sy,
                    i.w as f32 * sx,
                    i.h as f32 * sy,
                ],
                uv: interior_uv(tile),
                tex_size: [r.w as f32, r.h as f32],
                checker_origin: [0.0, 0.0],
                // Unused: the resample pass samples level `lod` by name.
                mode: Sampling::Minify,
                hdr: false,
                lod: *shift as f32,
            }
            .write(gpu.queue(), &tile.uniform);
        }
        let target = level_view(&overview.texture, 0);
        let load = if clear {
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
        } else {
            wgpu::LoadOp::Load
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("overview"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipes.resample(self.format));
        pass.set_bind_group(0, &frame_group, &[]);
        for tile in tiles {
            pass.set_bind_group(1, &tile.bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Size in pixels.
    pub fn size(&self) -> Size {
        Size::new(self.width as f32, self.height as f32)
    }

    /// True for floating point content, which is tone mapped on display.
    pub fn is_hdr(&self) -> bool {
        matches!(self.storage, Storage::Color(f) if f.is_hdr())
    }

    /// True when the image only carries coverage (an alpha mask).
    pub fn is_mask(&self) -> bool {
        self.storage == Storage::Mask
    }

    /// Number of full resolution textures the image is split into: 1
    /// unless it exceeds the device texture limit, 0 in reduced mode. The
    /// overview is not counted.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// True when only the downscaled overview is kept, because the full
    /// resolution tiles did not fit in GPU memory.
    pub fn is_reduced(&self) -> bool {
        self.tiles.is_empty()
    }

    /// GPU memory held, including mip levels.
    pub fn gpu_bytes(&self) -> u64 {
        let bpp: u64 = match self.format {
            WIDE_FORMAT => 8,
            MASK_FORMAT => 1,
            _ => 4,
        };
        let texels: u64 = self
            .tiles
            .iter()
            .map(|t| &t.region)
            .chain(self.overview.as_ref().map(|o| &o.0.region))
            .map(|r| u64::from(r.w) * u64::from(r.h))
            .sum();
        texels * bpp * 4 / 3
    }

    /// Choose the textures to draw and write their uniforms. Call from
    /// [`crate::ContentRenderer::prepare`].
    pub fn prepare(&mut self, ctx: &FrameContext<'_>) {
        self.prepare_at(
            ctx.gpu.queue(),
            &ctx.placement,
            ctx.visible_rect(),
            ctx.settings,
        );
    }

    pub(crate) fn prepare_at(
        &mut self,
        queue: &wgpu::Queue,
        placement: &Placement,
        clip: Rect,
        settings: &RenderSettings,
    ) {
        self.visible.clear();
        let rect = placement.image_rect;
        let scale = placement.scale;
        if scale.is_nan() || scale <= 0.0 || rect.is_empty() {
            return;
        }
        let crisp_from = settings.crisp_zoom.max(1.0);
        let hdr = self.is_hdr();
        if let Some((overview, _)) = &self.overview {
            let overview_scale = overview.region.w as f32 / self.width as f32;
            if scale <= overview_scale || self.tiles.is_empty() {
                DrawParams {
                    dst: [rect.x, rect.y, rect.w, rect.h],
                    uv: [0.0, 0.0, 1.0, 1.0],
                    tex_size: [overview.region.w as f32, overview.region.h as f32],
                    checker_origin: [rect.x, rect.y],
                    // Never crisp: overview texels are not image pixels.
                    mode: Sampling::for_scale(scale / overview_scale, f32::INFINITY),
                    hdr,
                    lod: 0.0,
                }
                .write(queue, &overview.uniform);
                self.visible.push(usize::MAX);
                return;
            }
        }
        for (index, tile) in self.tiles.iter().enumerate() {
            let i = tile.interior;
            // Both edges use the same expression, so neighbors share edges
            // bit for bit and no crack opens between tiles.
            let x0 = rect.x + i.x as f32 * scale;
            let y0 = rect.y + i.y as f32 * scale;
            let x1 = rect.x + (i.x + i.w) as f32 * scale;
            let y1 = rect.y + (i.y + i.h) as f32 * scale;
            let dst = Rect::new(x0, y0, x1 - x0, y1 - y0);
            if dst.intersect(clip).is_none() {
                continue;
            }
            DrawParams {
                dst: [dst.x, dst.y, dst.w, dst.h],
                uv: interior_uv(tile),
                tex_size: [tile.region.w as f32, tile.region.h as f32],
                checker_origin: [rect.x, rect.y],
                mode: Sampling::for_scale(scale, crisp_from),
                hdr,
                lod: 0.0,
            }
            .write(queue, &tile.uniform);
            self.visible.push(index);
        }
    }

    /// Draw the textures chosen by [`GpuImage::prepare`]. Call from
    /// [`crate::ContentRenderer::draw`].
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if self.visible.is_empty() {
            return;
        }
        let pipeline = if ctx.depth_format.is_some() {
            &ctx.pipelines.image_with_depth
        } else {
            &ctx.pipelines.image
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, ctx.frame, &[]);
        for &index in &self.visible {
            let tile = if index == usize::MAX {
                match &self.overview {
                    Some((overview, _)) => overview,
                    None => continue,
                }
            } else {
                &self.tiles[index]
            };
            pass.set_bind_group(1, &tile.bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /// The texture holding the whole image at the best available
    /// resolution, with its bind group: the single tile, or the overview of
    /// a tiled image. Used by the interface pass.
    pub(crate) fn whole(&self) -> &wgpu::BindGroup {
        match &self.overview {
            Some((overview, _)) => &overview.bind_group,
            None => &self.tiles[0].bind_group,
        }
    }

    /// The storage texture of tile `index`, for tests.
    #[cfg(test)]
    pub(crate) fn tile_texture(&self, index: usize) -> &wgpu::Texture {
        &self.tiles[index].texture
    }
}

fn interior_uv(tile: &Tile) -> [f32; 4] {
    let i = tile.interior;
    let r = tile.region;
    [
        (i.x - r.x) as f32 / r.w as f32,
        (i.y - r.y) as f32 / r.h as f32,
        (i.x + i.w - r.x) as f32 / r.w as f32,
        (i.y + i.h - r.y) as f32 / r.h as f32,
    ]
}

fn level_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("mip level"),
        base_mip_level: level,
        mip_level_count: Some(1),
        ..wgpu::TextureViewDescriptor::default()
    })
}

/// Record one render pass per mip level, each reading the level above.
fn encode_mips(gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
    let levels = texture.mip_level_count();
    if levels < 2 {
        return;
    }
    let pipes = gpu.pipelines();
    let pipeline = pipes.mip(texture.format());
    let views: Vec<wgpu::TextureView> = (0..levels).map(|l| level_view(texture, l)).collect();
    for level in 1..levels as usize {
        let source = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mip source"),
            layout: &pipes.mip_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&views[level - 1]),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mip"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &views[level],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &source, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Stage the tile's pixels strip by strip and convert them into level 0.
fn convert_into(gpu: &Gpu, image: &ImageBuf, tile: &Tile) -> Result<()> {
    let pipes = gpu.pipelines();
    let (staging_format, pipeline, layout, binding) = match image.format() {
        PixelFormat::Rgba8 => (
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &pipes.convert_srgb8,
            &pipes.convert_float_layout,
            0,
        ),
        PixelFormat::Rgba16 => (
            wgpu::TextureFormat::Rgba16Uint,
            &pipes.convert_rgba16,
            &pipes.convert_uint_layout,
            1,
        ),
        PixelFormat::RgbaF32 => (
            wgpu::TextureFormat::Rgba32Float,
            &pipes.convert_float,
            &pipes.convert_float_layout,
            0,
        ),
        other => return Err(Error::Unsupported(format!("pixel format {other:?}"))),
    };
    let bpp = image.format().bytes_per_pixel() as u64;
    let region = tile.region;
    let rows_per_strip =
        (STRIP_BYTES / (u64::from(region.w) * bpp)).clamp(1, u64::from(region.h)) as u32;
    let device = gpu.device();
    let queue = gpu.queue();
    let staging = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("staging strip"),
        size: wgpu::Extent3d {
            width: region.w,
            height: rows_per_strip,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: staging_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let staging_view = staging.create_view(&wgpu::TextureViewDescriptor::default());
    let params = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("convert params"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("convert"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(&staging_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: params.as_entire_binding(),
            },
        ],
    });
    let target = level_view(&tile.texture, 0);
    let stride = image.stride() as u64;
    let bytes_per_row = u32::try_from(stride).map_err(|_| Error::TooLarge {
        width: image.width(),
        height: image.height(),
    })?;
    let mut y = 0;
    while y < region.h {
        let rows = rows_per_strip.min(region.h - y);
        let offset = (u64::from(region.y + y) * stride + u64::from(region.x) * bpp) as usize;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &staging,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.data()[offset..],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: region.w,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        let origin: [i32; 4] = [0, y as i32, 0, 0];
        queue.write_buffer(&params, 0, bytemuck::cast_slice(&origin));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("convert strip"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("convert strip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_viewport(0.0, y as f32, region.w as f32, rows as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, y, region.w, rows);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        // One submission per strip: the next strip's staging write is
        // ordered after this one's conversion.
        queue.submit([encoder.finish()]);
        y += rows;
    }
    Ok(())
}

/// Write the alpha channel of `image` into an R8 tile.
fn write_mask(gpu: &Gpu, image: &ImageBuf, tile: &Tile) -> Result<()> {
    let data = image.data();
    let alpha: Vec<u8> = match image.format() {
        PixelFormat::Rgba8 => data.chunks_exact(4).map(|p| p[3]).collect(),
        PixelFormat::Rgba16 => data
            .chunks_exact(8)
            .map(|p| (u16::from_ne_bytes([p[6], p[7]]) >> 8) as u8)
            .collect(),
        PixelFormat::RgbaF32 => data
            .chunks_exact(16)
            .map(|p| {
                let a = f32::from_ne_bytes([p[12], p[13], p[14], p[15]]);
                (a.clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect(),
        other => return Err(Error::Unsupported(format!("pixel format {other:?}"))),
    };
    if tile.region.w != image.width() || tile.region.h != image.height() {
        return Err(Error::TooLarge {
            width: image.width(),
            height: image.height(),
        });
    }
    gpu.queue().write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tile.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &alpha,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width()),
            rows_per_image: Some(image.height()),
        },
        wgpu::Extent3d {
            width: image.width(),
            height: image.height(),
            depth_or_array_layers: 1,
        },
    );
    Ok(())
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn plan_axis_covers_with_gutters() {
        const G: u32 = MIN_GUTTER;
        assert_eq!(plan_axis(100, 256, G), vec![(0, 100, 0, 100)]);
        let spans = plan_axis(1000, 256, G);
        // Stride: the room left by both gutters, down to a multiple of 64.
        assert_eq!(spans[0], (0, 192 + G, 0, 192));
        assert_eq!(spans[1], (192 - G, 192 + 2 * G, 192, 192));
        let mut covered = 0;
        for &(r0, rl, i0, il) in &spans {
            assert!(rl <= 256);
            assert_eq!(i0, covered);
            assert!(r0 <= i0 && i0 + il <= r0 + rl);
            covered = i0 + il;
        }
        assert_eq!(covered, 1000);
        // A 16384 device limit leaves 16320-wide interiors (2 tiles for
        // 20000), and the overview of the largest image fits in 2 levels.
        assert_eq!(plan_axis(20_000, 16_384, G).len(), 2);
        assert_eq!(overview_shift(65_535, 100, 16_384), 2);
        assert_eq!(overview_shift(700, 300, 256), 2);
        assert_eq!(mip_levels(1, 1), 1);
        assert_eq!(mip_levels(256, 3), 9);
        assert_eq!(mip_levels(257, 3), 9);
    }

    #[test]
    fn sampling_mode_follows_zoom() {
        assert_eq!(Sampling::for_scale(0.999, 3.0), Sampling::Minify);
        assert_eq!(Sampling::for_scale(1.0, 3.0), Sampling::Magnify);
        assert_eq!(Sampling::for_scale(2.99, 3.0), Sampling::Magnify);
        assert_eq!(Sampling::for_scale(3.0, 3.0), Sampling::Crisp);
        assert_eq!(Sampling::for_scale(1.0, 1.0), Sampling::Crisp);
        assert_eq!(Sampling::for_scale(1.0e6, f32::INFINITY), Sampling::Magnify);
    }
}
