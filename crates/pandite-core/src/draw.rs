//! The draw list: the only interface between the interface crate and the
//! renderer crate.
//!
//! The interface crate lays out widgets and emits a [`DrawList`] each frame.
//! The renderer crate turns it into GPU draws. Neither depends on the other.
//! Coordinates are physical pixels with the origin at the window's top-left.

use crate::geom::{Point, Rect, Size};
use crate::image::ImageBuf;

/// A straight-alpha sRGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha, 255 is opaque.
    pub a: u8,
}

impl Color {
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Fully transparent.
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    /// An opaque color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A color with alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// The same color with a different alpha.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

/// Names a texture owned by the renderer. The interface crate picks ids and
/// uploads pixels through a [`TextureSink`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u64);

/// One drawing instruction. Commands draw in list order, later on top.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DrawCmd {
    /// A filled rectangle with optional rounded corners.
    Rect {
        /// Area to fill.
        rect: Rect,
        /// Fill color.
        color: Color,
        /// Corner radius in pixels, 0 for square corners.
        radius: f32,
    },
    /// A rectangle outline drawn inside `rect`.
    Border {
        /// Outer edge of the outline.
        rect: Rect,
        /// Line color.
        color: Color,
        /// Line width in pixels.
        width: f32,
        /// Corner radius in pixels.
        radius: f32,
    },
    /// A textured quad.
    Image {
        /// Destination area.
        rect: Rect,
        /// Texture to sample.
        texture: TextureId,
        /// Source area in normalized texture coordinates (0 to 1).
        uv: Rect,
        /// Multiplied with the texel. White draws the texture unchanged.
        /// For alpha-only icon textures this is the icon color.
        tint: Color,
    },
    /// A run of text on one line. `pos` is the top-left of the line box.
    Text {
        /// Top-left of the line box.
        pos: Point,
        /// The characters to draw. No line breaks.
        text: String,
        /// Font size in pixels.
        size: f32,
        /// Text color.
        color: Color,
    },
    /// Restrict following commands to this area. `None` removes the limit.
    Clip(Option<Rect>),
}

/// The drawing instructions for one frame of interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    /// Commands in draw order.
    pub cmds: Vec<DrawCmd>,
}

impl DrawList {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove all commands, keeping the allocation.
    pub fn clear(&mut self) {
        self.cmds.clear();
    }

    /// True when nothing would be drawn.
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Add a filled rectangle.
    pub fn rect(&mut self, rect: Rect, color: Color, radius: f32) {
        self.cmds.push(DrawCmd::Rect {
            rect,
            color,
            radius,
        });
    }

    /// Add a rectangle outline.
    pub fn border(&mut self, rect: Rect, color: Color, width: f32, radius: f32) {
        self.cmds.push(DrawCmd::Border {
            rect,
            color,
            width,
            radius,
        });
    }

    /// Add a whole texture drawn into `rect`.
    pub fn image(&mut self, rect: Rect, texture: TextureId, tint: Color) {
        self.cmds.push(DrawCmd::Image {
            rect,
            texture,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            tint,
        });
    }

    /// Add one line of text.
    pub fn text(&mut self, pos: Point, text: impl Into<String>, size: f32, color: Color) {
        self.cmds.push(DrawCmd::Text {
            pos,
            text: text.into(),
            size,
            color,
        });
    }

    /// Set or remove the clip area.
    pub fn clip(&mut self, rect: Option<Rect>) {
        self.cmds.push(DrawCmd::Clip(rect));
    }
}

/// Text metrics the interface crate needs for layout. The renderer crate
/// implements this with the same font it draws with.
pub trait TextMeasure {
    /// Width and line height of `text` at `size` pixels.
    fn measure(&self, text: &str, size: f32) -> Size;
}

/// Texture upload channel from the interface crate to the renderer.
pub trait TextureSink {
    /// Create or replace the texture `id` with `image`. With `alpha_mask`
    /// set, only the alpha channel is used and [`DrawCmd::Image::tint`]
    /// supplies the color (icons).
    fn upload(&mut self, id: TextureId, image: &ImageBuf, alpha_mask: bool);

    /// Free the texture `id`. Unknown ids are ignored.
    fn remove(&mut self, id: TextureId);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_list_keeps_order() {
        let mut list = DrawList::new();
        assert!(list.is_empty());
        list.clip(Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        list.rect(Rect::new(1.0, 1.0, 2.0, 2.0), Color::BLACK, 0.0);
        list.image(Rect::new(0.0, 0.0, 4.0, 4.0), TextureId(7), Color::WHITE);
        list.text(
            Point::new(0.0, 0.0),
            "hi",
            12.0,
            Color::WHITE.with_alpha(128),
        );
        list.clip(None);
        assert_eq!(list.cmds.len(), 5);
        assert!(matches!(list.cmds[0], DrawCmd::Clip(Some(_))));
        assert!(matches!(
            &list.cmds[2],
            DrawCmd::Image { texture: TextureId(7), uv, .. } if *uv == Rect::new(0.0, 0.0, 1.0, 1.0)
        ));
        assert!(matches!(&list.cmds[3], DrawCmd::Text { color, .. } if color.a == 128));
        list.clear();
        assert!(list.is_empty());
    }
}
