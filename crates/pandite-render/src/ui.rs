//! The interface pass: draws a [`DrawList`] over the content.
//!
//! Every command becomes one 64-byte instance of a single instanced quad
//! pipeline. Consecutive instances share a draw call until the scissor or
//! the bound texture changes; shapes need no texture, so they join any
//! batch, and all text shares the glyph atlas. The instance buffer and the
//! scratch vectors only grow, so steady-state frames allocate nothing.

use crate::color::linear_premultiplied;
use crate::gpu::Gpu;
use crate::image::{texture_bind_group, GpuImage};
use crate::pipelines::{DisplayPipelines, DRAW_PARAMS_SIZE, MASK_FORMAT, UI_INSTANCE_SIZE};
use crate::text::{FontSet, GlyphAtlas, Placed};
use pandite_core::{Color, DrawCmd, DrawList, ImageBuf, Rect, TextureId};
use std::collections::HashMap;
use std::sync::Arc;

const KIND_FILL: f32 = 0.0;
const KIND_BORDER: f32 = 1.0;
const KIND_IMAGE: f32 = 2.0;
const KIND_MASK: f32 = 3.0;
const KIND_GLYPH: f32 = 4.0;

type Instance = [f32; 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatchTexture {
    Atlas,
    Image(TextureId),
}

#[derive(Clone, Copy, Debug)]
struct Batch {
    texture: Option<BatchTexture>,
    scissor: [u32; 4],
    start: u32,
    end: u32,
}

pub(crate) struct UiRenderer {
    frame_uniform: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    instances: Vec<Instance>,
    batches: Vec<Batch>,
    placed: Vec<Placed>,
    buffer: Option<wgpu::Buffer>,
    textures: HashMap<TextureId, GpuImage>,
    fonts: Arc<FontSet>,
    atlas: GlyphAtlas,
    atlas_texture: Option<(wgpu::Texture, wgpu::BindGroup)>,
    atlas_uniform: wgpu::Buffer,
    /// The first upload failure, until taken.
    pub(crate) upload_error: Option<String>,
}

fn instance(
    rect: Rect,
    uv: [f32; 4],
    color: [f32; 4],
    radius: f32,
    width: f32,
    kind: f32,
) -> Instance {
    [
        rect.x, rect.y, rect.w, rect.h, uv[0], uv[1], uv[2], uv[3], color[0], color[1], color[2],
        color[3], radius, width, kind, 0.0,
    ]
}

/// Pixel rows and columns whose centers lie inside `rect`, clamped to the
/// target: the same rule the rasterizer applies to geometry.
pub(crate) fn scissor_for(rect: Rect, width: u32, height: u32) -> [u32; 4] {
    let edge = |v: f32, max: u32| -> u32 {
        if v.is_nan() {
            0
        } else {
            (v - 0.5).ceil().clamp(0.0, max as f32) as u32
        }
    };
    let x0 = edge(rect.x, width);
    let y0 = edge(rect.y, height);
    let x1 = edge(rect.x + rect.w, width).max(x0);
    let y1 = edge(rect.y + rect.h, height).max(y0);
    [x0, y0, x1 - x0, y1 - y0]
}

impl UiRenderer {
    pub(crate) fn new(gpu: &Gpu, fonts: Arc<FontSet>) -> Self {
        let pipes = gpu.pipelines();
        let frame_uniform = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui frame"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui frame"),
            layout: &pipes.frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&pipes.ui_sampler),
                },
            ],
        });
        let atlas_uniform = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas params"),
            size: DRAW_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        Self {
            frame_uniform,
            frame_group,
            instances: Vec::new(),
            batches: Vec::new(),
            placed: Vec::new(),
            buffer: None,
            textures: HashMap::new(),
            fonts,
            atlas: GlyphAtlas::new(gpu.max_texture_size().min(4096)),
            atlas_texture: None,
            atlas_uniform,
            upload_error: None,
        }
    }

    pub(crate) fn set_fonts(&mut self, fonts: Arc<FontSet>) {
        if !Arc::ptr_eq(&self.fonts, &fonts) {
            self.fonts = fonts;
            self.atlas.clear();
        }
    }

    pub(crate) fn upload(&mut self, gpu: &Gpu, id: TextureId, image: &ImageBuf, alpha_mask: bool) {
        let result = if alpha_mask {
            GpuImage::upload_mask(gpu, image)
        } else {
            GpuImage::upload(gpu, image)
        };
        match result {
            Ok(texture) => {
                self.textures.insert(id, texture);
            }
            Err(err) => {
                self.textures.remove(&id);
                self.upload_error
                    .get_or_insert_with(|| format!("texture {}: {err}", id.0));
            }
        }
    }

    pub(crate) fn remove(&mut self, id: TextureId) {
        self.textures.remove(&id);
    }

    pub(crate) fn has_texture(&self, id: TextureId) -> bool {
        self.textures.contains_key(&id)
    }

    /// Number of draw calls the last prepared frame needs.
    pub(crate) fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// The instance buffer, for tests that check it is reused.
    #[cfg(test)]
    pub(crate) fn instance_buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// Build instances and batches, upload new glyphs and the instance data.
    pub(crate) fn prepare(
        &mut self,
        gpu: &Gpu,
        list: &DrawList,
        width: u32,
        height: u32,
        encode_srgb: bool,
    ) {
        if self.build(list, width, height).is_err() {
            // The atlas is full at its largest size: start over with an
            // empty atlas. Glyphs that still do not fit are skipped.
            self.atlas.clear();
            let _ = self.build(list, width, height);
        }
        self.flush_atlas(gpu);
        let frame: [f32; 4] = [
            width as f32,
            height as f32,
            f32::from_bits(u32::from(encode_srgb)),
            0.0,
        ];
        gpu.queue()
            .write_buffer(&self.frame_uniform, 0, bytemuck::cast_slice(&frame));
        if self.instances.is_empty() {
            return;
        }
        let needed = self.instances.len() as u64 * UI_INSTANCE_SIZE;
        let capacity = self.buffer.as_ref().map_or(0, wgpu::Buffer::size);
        if capacity < needed {
            let size = needed.next_power_of_two().max(64 * UI_INSTANCE_SIZE);
            self.buffer = Some(gpu.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui instances"),
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer {
            gpu.queue()
                .write_buffer(buffer, 0, bytemuck::cast_slice(&self.instances));
        }
    }

    fn push(&mut self, texture: Option<BatchTexture>, scissor: [u32; 4], inst: Instance) {
        let index = self.instances.len() as u32;
        self.instances.push(inst);
        if let Some(last) = self.batches.last_mut() {
            let compatible = texture.is_none() || last.texture.is_none() || last.texture == texture;
            if last.scissor == scissor && compatible && last.end == index {
                if last.texture.is_none() {
                    last.texture = texture;
                }
                last.end = index + 1;
                return;
            }
        }
        self.batches.push(Batch {
            texture,
            scissor,
            start: index,
            end: index + 1,
        });
    }

    fn build(
        &mut self,
        list: &DrawList,
        width: u32,
        height: u32,
    ) -> Result<(), crate::text::AtlasFull> {
        self.instances.clear();
        self.batches.clear();
        let full = [0, 0, width, height];
        let mut scissor = full;
        for cmd in &list.cmds {
            let visible = scissor[2] > 0 && scissor[3] > 0;
            match cmd {
                DrawCmd::Clip(rect) => {
                    scissor = rect.map_or(full, |r| scissor_for(r, width, height));
                }
                _ if !visible => {}
                DrawCmd::Rect {
                    rect,
                    color,
                    radius,
                } => {
                    if !rect.is_empty() && color.a > 0 {
                        let inst = instance(
                            *rect,
                            [0.0; 4],
                            linear_premultiplied(*color),
                            radius.max(0.0),
                            0.0,
                            KIND_FILL,
                        );
                        self.push(None, scissor, inst);
                    }
                }
                DrawCmd::Border {
                    rect,
                    color,
                    width: line,
                    radius,
                } => {
                    if !rect.is_empty() && color.a > 0 && *line > 0.0 {
                        let inst = instance(
                            *rect,
                            [0.0; 4],
                            linear_premultiplied(*color),
                            radius.max(0.0),
                            *line,
                            KIND_BORDER,
                        );
                        self.push(None, scissor, inst);
                    }
                }
                DrawCmd::Image {
                    rect,
                    texture,
                    uv,
                    tint,
                } => {
                    let Some(image) = self.textures.get(texture) else {
                        continue;
                    };
                    if rect.is_empty() || tint.a == 0 {
                        continue;
                    }
                    let kind = if image.is_mask() {
                        KIND_MASK
                    } else {
                        KIND_IMAGE
                    };
                    let inst = instance(
                        *rect,
                        [uv.x, uv.y, uv.x + uv.w, uv.y + uv.h],
                        linear_premultiplied(*tint),
                        0.0,
                        0.0,
                        kind,
                    );
                    self.push(Some(BatchTexture::Image(*texture)), scissor, inst);
                }
                DrawCmd::Text {
                    pos,
                    text,
                    size,
                    color,
                } => {
                    self.push_text(*pos, text, *size, *color, scissor)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn push_text(
        &mut self,
        pos: pandite_core::Point,
        text: &str,
        size: f32,
        color: Color,
        scissor: [u32; 4],
    ) -> Result<(), crate::text::AtlasFull> {
        let px = FontSet::quantize(size);
        if px <= 0.0 || color.a == 0 || !pos.x.is_finite() || !pos.y.is_finite() {
            return Ok(());
        }
        let fonts = Arc::clone(&self.fonts);
        let line = fonts.line(px);
        let baseline = (pos.y + line.ascent).round();
        let mut placed = std::mem::take(&mut self.placed);
        placed.clear();
        fonts.layout(text, px, |p| placed.push(p));
        let tint = linear_premultiplied(color);
        let mut result = Ok(());
        for p in &placed {
            let entry = match self.atlas.glyph(&fonts, p, px) {
                Ok(Some(entry)) => entry,
                Ok(None) => continue,
                Err(full) => {
                    result = Err(full);
                    break;
                }
            };
            let x = (pos.x + p.x).round() + entry.left as f32;
            let y = baseline + entry.top as f32;
            let rect = Rect::new(x, y, entry.w as f32, entry.h as f32);
            let uv = [
                entry.x as f32,
                entry.y as f32,
                (entry.x + entry.w) as f32,
                (entry.y + entry.h) as f32,
            ];
            let inst = instance(rect, uv, tint, 0.0, 0.0, KIND_GLYPH);
            self.push(Some(BatchTexture::Atlas), scissor, inst);
        }
        self.placed = placed;
        result
    }

    fn flush_atlas(&mut self, gpu: &Gpu) {
        let size = self.atlas.size();
        let stale = self
            .atlas_texture
            .as_ref()
            .is_none_or(|(texture, _)| texture.width() != size);
        if stale {
            let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("glyph atlas"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MASK_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = texture_bind_group(gpu, gpu.pipelines(), &view, &self.atlas_uniform);
            self.atlas_texture = Some((texture, group));
            // A new texture starts empty: upload everything.
            let _ = self.atlas.take_dirty();
            self.upload_rows(gpu, 0, size);
            return;
        }
        if let Some((y0, y1)) = self.atlas.take_dirty() {
            self.upload_rows(gpu, y0, y1.min(size));
        }
    }

    fn upload_rows(&self, gpu: &Gpu, y0: u32, y1: u32) {
        let Some((texture, _)) = &self.atlas_texture else {
            return;
        };
        if y1 <= y0 {
            return;
        }
        let size = self.atlas.size();
        let start = (y0 * size) as usize;
        let end = (y1 * size) as usize;
        gpu.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &self.atlas.pixels()[start..end],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size),
                rows_per_image: Some(y1 - y0),
            },
            wgpu::Extent3d {
                width: size,
                height: y1 - y0,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Record the batches into `pass`.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipelines: &DisplayPipelines) {
        let (Some(buffer), Some((_, atlas_group))) = (&self.buffer, &self.atlas_texture) else {
            return;
        };
        if self.batches.is_empty() {
            return;
        }
        pass.set_pipeline(&pipelines.ui);
        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        for batch in &self.batches {
            let [x, y, w, h] = batch.scissor;
            if w == 0 || h == 0 {
                continue;
            }
            let group = match batch.texture {
                Some(BatchTexture::Image(id)) => match self.textures.get(&id) {
                    Some(image) => image.whole(),
                    None => continue,
                },
                Some(BatchTexture::Atlas) | None => atlas_group,
            };
            pass.set_scissor_rect(x, y, w, h);
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..4, batch.start..batch.end);
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn scissor_follows_pixel_centers() {
        assert_eq!(
            scissor_for(Rect::new(10.0, 20.0, 30.0, 40.0), 100, 100),
            [10, 20, 30, 40]
        );
        // 10.6 excludes pixel 10 (center 10.5); 40.4 excludes pixel 40.
        assert_eq!(
            scissor_for(Rect::new(10.6, 0.0, 29.8, 10.0), 100, 100),
            [11, 0, 29, 10]
        );
        assert_eq!(
            scissor_for(Rect::new(-50.0, -50.0, 500.0, 20.0), 100, 100),
            [0, 0, 100, 0]
        );
        assert_eq!(
            scissor_for(Rect::new(200.0, 0.0, 10.0, 10.0), 100, 100)[2],
            0
        );
    }
}
