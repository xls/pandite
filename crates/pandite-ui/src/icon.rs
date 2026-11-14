//! The interface icons: original vector shapes, rasterized in this crate.
//!
//! Every icon is a short list of strokes, fills and arcs on a 24 x 24
//! design grid (see [`crate::raster`]). No third-party icon set is used.
//! Icons are rasterized as white alpha masks at the pixel size the DPI scale
//! needs, uploaded once through [`TextureSink`] with `alpha_mask = true`,
//! and drawn with [`pandite_core::DrawCmd::Image`] whose tint is the icon
//! color. Rasterization runs again only when the pixel size changes.

use crate::raster::{rasterize, Op, Shape, Transform, P};
use pandite_core::{ImageBuf, PixelFormat, TextureId, TextureSink};

/// First texture id used for icons. See [`Icon::texture_id`].
pub const ICON_TEXTURE_BASE: u64 = 1 << 40;

/// An interface icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Icon {
    /// An open folder.
    Open,
    /// A floppy disk.
    SaveAs,
    /// Two overlapping sheets with text lines.
    CopyPath,
    /// A U-turn arrow.
    Undo,
    /// A counterclockwise arrow.
    RotateLeft,
    /// A clockwise arrow.
    RotateRight,
    /// Two triangles mirrored across a vertical axis.
    FlipHorizontal,
    /// Two triangles mirrored across a horizontal axis.
    FlipVertical,
    /// Two interlocking corner marks.
    Crop,
    /// A small box with an outward arrow.
    Resize,
    /// Two stacked boxes with an outward arrow.
    BulkResize,
    /// A frame with a double arrow touching both sides.
    Fit,
    /// The characters "1:1".
    ActualSize,
    /// A magnifier with a plus.
    ZoomIn,
    /// A magnifier with a minus.
    ZoomOut,
    /// A left chevron.
    Previous,
    /// A right chevron.
    Next,
    /// A large frame above a row of three small ones.
    Gallery,
    /// Four outward corner marks.
    Fullscreen,
    /// Four inward corner marks.
    ExitFullscreen,
    /// Three dots (toolbar overflow).
    More,
    /// A check mark.
    Check,
    /// A cross.
    Close,
    /// A framed landscape (missing thumbnail, empty screen).
    Picture,
    /// A triangle with an exclamation mark (failed thumbnail, notice).
    Warning,
    /// A cloud with a down arrow (file not downloaded from a cloud drive).
    Cloud,
    /// A down chevron (drop-down lists).
    ChevronDown,
    /// The check mark of a thumbnail badge, at the size of the badge.
    Mark,
}

const W: f32 = 2.0;

const fn path(pts: &'static [P], width: f32) -> Op {
    Op::Add(Shape::Path {
        pts,
        width,
        closed: false,
    })
}

const fn closed(pts: &'static [P], width: f32) -> Op {
    Op::Add(Shape::Path {
        pts,
        width,
        closed: true,
    })
}

const fn fill(pts: &'static [P]) -> Op {
    Op::Add(Shape::Fill { pts })
}

const fn disc(x: f32, y: f32, r: f32) -> Op {
    Op::Add(Shape::Disc { c: (x, y), r })
}

const fn frame(x: f32, y: f32, w: f32, h: f32, r: f32) -> Op {
    Op::Add(Shape::Frame {
        x,
        y,
        w,
        h,
        r,
        width: W,
    })
}

const fn boxed(x: f32, y: f32, w: f32, h: f32, r: f32) -> Op {
    Op::Add(Shape::Box { x, y, w, h, r })
}

const OPEN: &[Op] = &[
    path(
        &[
            (3.0, 18.5),
            (3.0, 5.5),
            (9.0, 5.5),
            (11.0, 7.5),
            (18.5, 7.5),
            (18.5, 10.5),
        ],
        W,
    ),
    closed(&[(3.0, 18.5), (6.0, 11.0), (22.0, 11.0), (19.0, 18.5)], W),
];

const SAVE_AS: &[Op] = &[
    closed(
        &[
            (4.0, 4.0),
            (16.5, 4.0),
            (20.0, 7.5),
            (20.0, 20.0),
            (4.0, 20.0),
        ],
        W,
    ),
    path(&[(8.0, 4.0), (8.0, 8.5), (15.0, 8.5), (15.0, 4.0)], W),
    path(&[(7.5, 20.0), (7.5, 14.0), (16.5, 14.0), (16.5, 20.0)], W),
];

const COPY_PATH: &[Op] = &[
    frame(3.0, 3.0, 11.0, 13.0, 2.0),
    Op::Cut(
        Shape::Box {
            x: 9.0,
            y: 8.0,
            w: 11.5,
            h: 13.0,
            r: 2.0,
        },
        1.0,
    ),
    frame(9.0, 8.0, 11.5, 13.0, 2.0),
    path(&[(12.5, 12.5), (17.0, 12.5)], 1.6),
    path(&[(12.5, 16.5), (15.5, 16.5)], 1.6),
];

const UNDO: &[Op] = &[
    Op::Add(Shape::Arc {
        c: (13.0, 13.5),
        r: 5.5,
        start: 270.0,
        sweep: 180.0,
        width: W,
    }),
    path(&[(13.0, 8.0), (6.0, 8.0)], W),
    path(&[(9.5, 4.5), (6.0, 8.0), (9.5, 11.5)], W),
    path(&[(13.0, 19.0), (8.0, 19.0)], W),
];

const ROTATE: &[Op] = &[
    Op::Add(Shape::Arc {
        c: (12.0, 13.0),
        r: 7.0,
        start: 0.0,
        sweep: 262.0,
        width: W,
    }),
    fill(&[(10.5, 2.0), (16.5, 6.0), (10.5, 10.0)]),
];

const FLIP: &[Op] = &[
    path(&[(12.0, 2.5), (12.0, 5.0)], 1.5),
    path(&[(12.0, 8.0), (12.0, 10.5)], 1.5),
    path(&[(12.0, 13.5), (12.0, 16.0)], 1.5),
    path(&[(12.0, 19.0), (12.0, 21.5)], 1.5),
    fill(&[(9.5, 5.0), (9.5, 19.0), (2.0, 12.0)]),
    closed(&[(14.6, 5.5), (14.6, 18.5), (21.6, 12.0)], 1.8),
];

const CROP: &[Op] = &[
    path(&[(7.0, 2.5), (7.0, 17.0), (21.5, 17.0)], W),
    path(&[(2.5, 7.0), (17.0, 7.0), (17.0, 21.5)], W),
];

const RESIZE: &[Op] = &[
    frame(3.0, 12.0, 9.0, 9.0, 1.5),
    path(&[(11.5, 12.5), (20.5, 3.5)], W),
    path(&[(14.5, 3.5), (20.5, 3.5), (20.5, 9.5)], W),
];

const BULK_RESIZE: &[Op] = &[
    frame(7.0, 8.0, 9.0, 9.0, 1.5),
    Op::Cut(
        Shape::Box {
            x: 2.5,
            y: 12.5,
            w: 9.0,
            h: 9.0,
            r: 1.5,
        },
        1.2,
    ),
    frame(2.5, 12.5, 9.0, 9.0, 1.5),
    path(&[(16.0, 8.0), (21.0, 3.0)], W),
    path(&[(16.0, 3.0), (21.0, 3.0), (21.0, 8.0)], W),
];

const FIT: &[Op] = &[
    frame(2.5, 5.0, 19.0, 14.0, 2.0),
    path(&[(7.0, 12.0), (17.0, 12.0)], W),
    path(&[(9.5, 9.5), (7.0, 12.0), (9.5, 14.5)], W),
    path(&[(14.5, 9.5), (17.0, 12.0), (14.5, 14.5)], W),
];

const ACTUAL_SIZE: &[Op] = &[
    path(&[(4.0, 8.5), (7.0, 6.0), (7.0, 18.5)], 2.2),
    disc(12.0, 9.5, 1.5),
    disc(12.0, 15.5, 1.5),
    path(&[(16.0, 8.5), (19.0, 6.0), (19.0, 18.5)], 2.2),
];

const ZOOM_IN: &[Op] = &[
    Op::Add(Shape::Ring {
        c: (10.0, 10.0),
        r: 7.5,
        width: W,
    }),
    path(&[(16.0, 16.0), (21.5, 21.5)], 2.8),
    path(&[(5.8, 10.0), (14.2, 10.0)], 2.6),
    path(&[(10.0, 5.8), (10.0, 14.2)], 2.6),
];

const ZOOM_OUT: &[Op] = &[
    Op::Add(Shape::Ring {
        c: (10.0, 10.0),
        r: 7.5,
        width: W,
    }),
    path(&[(16.0, 16.0), (21.5, 21.5)], 2.8),
    path(&[(5.8, 10.0), (14.2, 10.0)], 2.6),
];

const CHEVRON: &[Op] = &[path(&[(15.0, 5.0), (8.0, 12.0), (15.0, 19.0)], 2.4)];

const GALLERY: &[Op] = &[
    frame(3.0, 3.0, 18.0, 11.0, 2.0),
    boxed(3.0, 16.5, 4.8, 4.5, 1.0),
    boxed(9.6, 16.5, 4.8, 4.5, 1.0),
    boxed(16.2, 16.5, 4.8, 4.5, 1.0),
];

const FULLSCREEN: &[Op] = &[
    path(&[(3.0, 9.0), (3.0, 3.0), (9.0, 3.0)], W),
    path(&[(15.0, 3.0), (21.0, 3.0), (21.0, 9.0)], W),
    path(&[(21.0, 15.0), (21.0, 21.0), (15.0, 21.0)], W),
    path(&[(9.0, 21.0), (3.0, 21.0), (3.0, 15.0)], W),
];

const EXIT_FULLSCREEN: &[Op] = &[
    path(&[(3.0, 9.0), (9.0, 9.0), (9.0, 3.0)], W),
    path(&[(15.0, 3.0), (15.0, 9.0), (21.0, 9.0)], W),
    path(&[(21.0, 15.0), (15.0, 15.0), (15.0, 21.0)], W),
    path(&[(9.0, 21.0), (9.0, 15.0), (3.0, 15.0)], W),
];

const MORE: &[Op] = &[
    disc(5.0, 12.0, 1.9),
    disc(12.0, 12.0, 1.9),
    disc(19.0, 12.0, 1.9),
];

const CHECK: &[Op] = &[path(&[(4.5, 12.5), (9.5, 17.5), (19.5, 6.5)], 2.4)];

const CLOSE: &[Op] = &[
    path(&[(6.0, 6.0), (18.0, 18.0)], 2.2),
    path(&[(18.0, 6.0), (6.0, 18.0)], 2.2),
];

const PICTURE: &[Op] = &[
    frame(3.0, 4.0, 18.0, 16.0, 2.0),
    fill(&[
        (5.5, 18.0),
        (10.0, 11.5),
        (13.5, 16.0),
        (15.5, 13.5),
        (18.5, 18.0),
    ]),
    disc(16.0, 8.5, 1.8),
];

const WARNING: &[Op] = &[
    closed(&[(12.0, 3.5), (21.5, 20.0), (2.5, 20.0)], W),
    path(&[(12.0, 9.5), (12.0, 14.0)], 2.2),
    disc(12.0, 17.0, 1.3),
];

const CLOUD_PARTS: &[Shape] = &[
    Shape::Disc {
        c: (7.5, 15.0),
        r: 4.0,
    },
    Shape::Disc {
        c: (12.5, 10.5),
        r: 5.5,
    },
    Shape::Disc {
        c: (17.0, 14.5),
        r: 4.5,
    },
    Shape::Box {
        x: 7.5,
        y: 13.0,
        w: 9.5,
        h: 6.0,
        r: 0.0,
    },
];

const CLOUD: &[Op] = &[
    Op::Add(Shape::Outline {
        parts: CLOUD_PARTS,
        width: W,
    }),
    path(&[(12.3, 10.5), (12.3, 16.0)], 1.8),
    path(&[(9.8, 13.7), (12.3, 16.2), (14.8, 13.7)], 1.8),
];

const CHEVRON_DOWN: &[Op] = &[path(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)], 2.2)];

impl Icon {
    /// Every icon.
    pub const ALL: [Icon; 28] = [
        Icon::Open,
        Icon::SaveAs,
        Icon::CopyPath,
        Icon::Undo,
        Icon::RotateLeft,
        Icon::RotateRight,
        Icon::FlipHorizontal,
        Icon::FlipVertical,
        Icon::Crop,
        Icon::Resize,
        Icon::BulkResize,
        Icon::Fit,
        Icon::ActualSize,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::Previous,
        Icon::Next,
        Icon::Gallery,
        Icon::Fullscreen,
        Icon::ExitFullscreen,
        Icon::More,
        Icon::Check,
        Icon::Close,
        Icon::Picture,
        Icon::Warning,
        Icon::Cloud,
        Icon::ChevronDown,
        Icon::Mark,
    ];

    /// The position of this icon in [`Icon::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// The texture id the icon is uploaded under:
    /// `ICON_TEXTURE_BASE + index`.
    pub fn texture_id(self) -> TextureId {
        TextureId(ICON_TEXTURE_BASE + self.index() as u64)
    }

    /// The icon uploaded under `id`, if any.
    pub fn from_texture_id(id: TextureId) -> Option<Icon> {
        let index = id.0.checked_sub(ICON_TEXTURE_BASE)?;
        Icon::ALL.get(usize::try_from(index).ok()?).copied()
    }

    /// A short name, for the icon sheet and test messages.
    pub fn name(self) -> &'static str {
        match self {
            Icon::Open => "open",
            Icon::SaveAs => "save-as",
            Icon::CopyPath => "copy-path",
            Icon::Undo => "undo",
            Icon::RotateLeft => "rotate-left",
            Icon::RotateRight => "rotate-right",
            Icon::FlipHorizontal => "flip-horizontal",
            Icon::FlipVertical => "flip-vertical",
            Icon::Crop => "crop",
            Icon::Resize => "resize",
            Icon::BulkResize => "bulk-resize",
            Icon::Fit => "fit",
            Icon::ActualSize => "actual-size",
            Icon::ZoomIn => "zoom-in",
            Icon::ZoomOut => "zoom-out",
            Icon::Previous => "previous",
            Icon::Next => "next",
            Icon::Gallery => "gallery",
            Icon::Fullscreen => "fullscreen",
            Icon::ExitFullscreen => "exit-fullscreen",
            Icon::More => "more",
            Icon::Check => "check",
            Icon::Mark => "mark",
            Icon::Close => "close",
            Icon::Picture => "picture",
            Icon::Warning => "warning",
            Icon::Cloud => "cloud",
            Icon::ChevronDown => "chevron-down",
        }
    }

    fn shape(self) -> (&'static [Op], Transform) {
        match self {
            Icon::Open => (OPEN, Transform::Identity),
            Icon::SaveAs => (SAVE_AS, Transform::Identity),
            Icon::CopyPath => (COPY_PATH, Transform::Identity),
            Icon::Undo => (UNDO, Transform::Identity),
            Icon::RotateLeft => (ROTATE, Transform::MirrorX),
            Icon::RotateRight => (ROTATE, Transform::Identity),
            Icon::FlipHorizontal => (FLIP, Transform::Identity),
            Icon::FlipVertical => (FLIP, Transform::Transpose),
            Icon::Crop => (CROP, Transform::Identity),
            Icon::Resize => (RESIZE, Transform::Identity),
            Icon::BulkResize => (BULK_RESIZE, Transform::Identity),
            Icon::Fit => (FIT, Transform::Identity),
            Icon::ActualSize => (ACTUAL_SIZE, Transform::Identity),
            Icon::ZoomIn => (ZOOM_IN, Transform::Identity),
            Icon::ZoomOut => (ZOOM_OUT, Transform::Identity),
            Icon::Previous => (CHEVRON, Transform::Identity),
            Icon::Next => (CHEVRON, Transform::MirrorX),
            Icon::Gallery => (GALLERY, Transform::Identity),
            Icon::Fullscreen => (FULLSCREEN, Transform::Identity),
            Icon::ExitFullscreen => (EXIT_FULLSCREEN, Transform::Identity),
            Icon::More => (MORE, Transform::Identity),
            Icon::Check | Icon::Mark => (CHECK, Transform::Identity),
            Icon::Close => (CLOSE, Transform::Identity),
            Icon::Picture => (PICTURE, Transform::Identity),
            Icon::Warning => (WARNING, Transform::Identity),
            Icon::Cloud => (CLOUD, Transform::Identity),
            Icon::ChevronDown => (CHEVRON_DOWN, Transform::Identity),
        }
    }

    /// The pixel size this icon is rasterized at when the toolbar icons are
    /// `px` pixels. The mark of a thumbnail badge is drawn at 12/20 of that
    /// size, and a mask made at its own size stays sharp there.
    pub fn raster_px(self, px: u32) -> u32 {
        match self {
            Icon::Mark => ((px as f32 * 0.6).round() as u32).max(1),
            _ => px,
        }
    }

    /// The antialiased coverage mask at `size` x `size` pixels (0 to 255,
    /// row major, top row first).
    pub fn coverage(self, size: u32) -> Vec<u8> {
        let (ops, transform) = self.shape();
        rasterize(ops, transform, size.clamp(1, 1024))
    }

    /// The icon as a white RGBA8 image whose alpha is the coverage, ready
    /// for upload with `alpha_mask = true`.
    pub fn rasterize(self, size: u32) -> ImageBuf {
        let size = size.clamp(1, 1024);
        let mask = self.coverage(size);
        let mut data = Vec::with_capacity(mask.len() * 4);
        for a in mask {
            data.extend_from_slice(&[255, 255, 255, a]);
        }
        ImageBuf::new(size, size, PixelFormat::Rgba8, data).expect("icon buffer matches its size")
    }
}

/// Rasterized icons for one pixel size and what has been uploaded.
#[derive(Default)]
pub(crate) struct IconCache {
    rasterized_px: u32,
    images: Vec<ImageBuf>,
    uploaded_px: u32,
}

impl std::fmt::Debug for IconCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconCache")
            .field("rasterized_px", &self.rasterized_px)
            .field("icons", &self.images.len())
            .field("uploaded_px", &self.uploaded_px)
            .finish()
    }
}

impl IconCache {
    /// Make sure every icon is uploaded at `px` pixels. Rasterizes only
    /// when the size changed. Returns true when anything was uploaded.
    pub fn sync(&mut self, px: u32, sink: &mut dyn TextureSink) -> bool {
        let px = px.clamp(1, 1024);
        if self.uploaded_px == px {
            return false;
        }
        if self.rasterized_px != px || self.images.len() != Icon::ALL.len() {
            self.images.clear();
            self.images.extend(
                Icon::ALL
                    .iter()
                    .map(|icon| icon.rasterize(icon.raster_px(px))),
            );
            self.rasterized_px = px;
        }
        for (icon, image) in Icon::ALL.iter().zip(&self.images) {
            sink.upload(icon.texture_id(), image, true);
        }
        self.uploaded_px = px;
        true
    }

    /// Forget what was uploaded (for example after the renderer lost its
    /// textures). The next sync uploads again without rasterizing.
    pub fn invalidate(&mut self) {
        self.uploaded_px = 0;
    }

    /// The pixel size currently uploaded, 0 when none.
    pub fn uploaded_px(&self) -> u32 {
        self.uploaded_px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_ids_round_trip_and_all_is_ordered() {
        for (i, icon) in Icon::ALL.iter().enumerate() {
            assert_eq!(icon.index(), i);
            assert_eq!(Icon::from_texture_id(icon.texture_id()), Some(*icon));
        }
        assert_eq!(Icon::from_texture_id(TextureId(5)), None);
        assert_eq!(
            Icon::from_texture_id(TextureId(ICON_TEXTURE_BASE + 9999)),
            None
        );
    }

    #[test]
    fn every_icon_has_ink_and_soft_edges() {
        for icon in Icon::ALL {
            let mask = icon.coverage(20);
            let ink = mask.iter().filter(|&&v| v > 128).count();
            let soft = mask.iter().filter(|&&v| v > 0 && v < 255).count();
            assert!(ink >= 12, "{} has too little ink: {ink}", icon.name());
            assert!(ink < 300, "{} is nearly solid: {ink}", icon.name());
            assert!(soft > 0, "{} is not antialiased", icon.name());
        }
    }
}
