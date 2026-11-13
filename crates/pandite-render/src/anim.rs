//! Built-in provider for frame animations (GIF, animated WebP, APNG).

use crate::gpu::Gpu;
use crate::image::GpuImage;
use crate::provider::{
    ContentRenderer, DrawContext, FrameContext, ProviderContext, RenderInput, RenderProvider,
};
use pandite_core::{Animation, Content, Error, Result, Size};
use std::time::Duration;

/// Default GPU memory an animation may use to keep every frame resident.
pub const ANIMATION_CACHE_BYTES: u64 = 256 << 20;

/// Plays [`pandite_core::Content::Animation`].
///
/// Frame selection is [`Animation::frame_at`] of the time accumulated by
/// [`ContentRenderer::update`], so looping, finite loop counts (holding the
/// last frame) and the next deadline follow the core rules exactly.
///
/// The first frame uploads when the content is created. Every frame drawn
/// also uploads the one after it, so a frame change never waits for an
/// upload. When all frames fit in [`AnimationProvider::cache_bytes`] they
/// stay resident and later loops cost nothing; longer animations stream
/// through two sets of textures (the frame shown and the next one).
#[derive(Clone, Copy, Debug)]
pub struct AnimationProvider {
    /// GPU memory budget for keeping every frame resident.
    pub cache_bytes: u64,
}

impl Default for AnimationProvider {
    fn default() -> Self {
        Self {
            cache_bytes: ANIMATION_CACHE_BYTES,
        }
    }
}

impl RenderProvider for AnimationProvider {
    fn name(&self) -> &'static str {
        "animation"
    }

    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> Result<Box<dyn ContentRenderer>> {
        let anim = match input {
            RenderInput::Content(Content::Animation(anim)) => anim.clone(),
            other => {
                return Err(Error::Unsupported(format!(
                    "the animation provider plays frame animations, not {:?} content",
                    other.kind()
                )))
            }
        };
        let first = GpuImage::upload(ctx.gpu, &anim.frames()[0].image)?;
        let total = first.gpu_bytes().saturating_mul(anim.frames().len() as u64);
        let frames = if total <= self.cache_bytes {
            let mut cache: Vec<Option<GpuImage>> = Vec::new();
            cache.resize_with(anim.frames().len(), || None);
            cache[0] = Some(first);
            Frames::Resident(cache)
        } else {
            Frames::Streamed(vec![(first, 0)])
        };
        Ok(Box::new(AnimationRenderer {
            anim,
            elapsed: Duration::ZERO,
            current: 0,
            frames,
        }))
    }
}

/// Where the frame textures live.
enum Frames {
    /// One slot per frame, filled on first use and kept.
    Resident(Vec<Option<GpuImage>>),
    /// At most two images and the frame each holds: shown and next.
    Streamed(Vec<(GpuImage, usize)>),
}

struct AnimationRenderer {
    anim: Animation,
    elapsed: Duration,
    current: usize,
    frames: Frames,
}

impl AnimationRenderer {
    /// Make frame `index` resident without evicting the shown frame.
    fn load(&mut self, gpu: &Gpu, index: usize) -> Result<()> {
        let image = &self.anim.frames()[index].image;
        match &mut self.frames {
            Frames::Resident(cache) => {
                if cache[index].is_none() {
                    cache[index] = Some(GpuImage::upload(gpu, image)?);
                }
            }
            Frames::Streamed(slots) => {
                if slots.iter().any(|(_, held)| *held == index) {
                    return Ok(());
                }
                if slots.len() < 2 {
                    slots.push((GpuImage::upload(gpu, image)?, index));
                } else {
                    let current = self.current;
                    let free = slots
                        .iter()
                        .position(|(_, held)| *held != current)
                        .unwrap_or(0);
                    let slot = &mut slots[free];
                    slot.0.update(gpu, image)?;
                    slot.1 = index;
                }
            }
        }
        Ok(())
    }

    fn shown_mut(&mut self) -> Option<&mut GpuImage> {
        let current = self.current;
        match &mut self.frames {
            Frames::Resident(cache) => cache.get_mut(current).and_then(Option::as_mut),
            Frames::Streamed(slots) => slots
                .iter_mut()
                .find(|(_, held)| *held == current)
                .map(|(image, _)| image),
        }
    }

    fn shown(&self) -> Option<&GpuImage> {
        let current = self.current;
        match &self.frames {
            Frames::Resident(cache) => cache.get(current).and_then(Option::as_ref),
            Frames::Streamed(slots) => slots
                .iter()
                .find(|(_, held)| *held == current)
                .map(|(image, _)| image),
        }
    }
}

impl ContentRenderer for AnimationRenderer {
    fn size(&self) -> Size {
        let first = &self.anim.frames()[0].image;
        Size::new(first.width() as f32, first.height() as f32)
    }

    fn update(&mut self, dt: Duration) -> Option<Duration> {
        self.elapsed = self.elapsed.saturating_add(dt);
        let (index, next) = self.anim.frame_at(self.elapsed);
        self.current = index;
        next
    }

    fn frame_index(&self) -> usize {
        self.current
    }

    fn prepare(&mut self, ctx: &mut FrameContext<'_>) -> Result<()> {
        let gpu = ctx.gpu;
        self.load(gpu, self.current)?;
        if let Some(image) = self.shown_mut() {
            image.prepare(ctx);
        }
        // Upload the next frame while this one is on screen.
        let count = self.anim.frames().len();
        let next = (self.current + 1) % count;
        if next != self.current {
            self.load(gpu, next)?;
        }
        Ok(())
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if let Some(image) = self.shown() {
            image.draw(pass, ctx);
        }
    }
}
