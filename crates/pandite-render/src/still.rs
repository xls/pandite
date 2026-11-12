//! Built-in provider for still images.

use crate::image::GpuImage;
use crate::provider::{
    ContentRenderer, DrawContext, FrameContext, ProviderContext, RenderInput, RenderProvider,
};
use pandite_core::{Error, Result, Size};
use std::time::Duration;

/// Draws [`pandite_core::Content::Still`]. Given animation content it shows
/// the first frame, so registering it for [`pandite_core::ContentKind::Animation`]
/// turns animation off.
#[derive(Clone, Copy, Debug, Default)]
pub struct StillProvider;

impl RenderProvider for StillProvider {
    fn name(&self) -> &'static str {
        "still"
    }

    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> Result<Box<dyn ContentRenderer>> {
        match input {
            RenderInput::Content(content) => Ok(Box::new(StillRenderer {
                image: GpuImage::upload(ctx.gpu, content.first_image())?,
            })),
            other => Err(Error::Unsupported(format!(
                "the still provider draws decoded images, not {:?} data",
                other.kind()
            ))),
        }
    }
}

struct StillRenderer {
    image: GpuImage,
}

impl ContentRenderer for StillRenderer {
    fn size(&self) -> Size {
        self.image.size()
    }

    fn update(&mut self, _dt: Duration) -> Option<Duration> {
        None
    }

    fn prepare(&mut self, ctx: &mut FrameContext<'_>) -> Result<()> {
        self.image.prepare(ctx);
        Ok(())
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        self.image.draw(pass, ctx);
    }
}
