//! Gate G2: rotate, flip and crop move pixels exactly, in every format.

use pandite_core::{Error, ImageBuf, PixelFormat, Rect};
use pandite_ops::{
    crop, flip_horizontal, flip_vertical, rotate, rotate180, rotate270, rotate90, PixelRect,
    Rotation,
};

const FORMATS: [PixelFormat; 3] = [
    PixelFormat::Rgba8,
    PixelFormat::Rgba16,
    PixelFormat::RgbaF32,
];

/// An image whose pixel at (x, y) stores x in channel 0 and y in channel 1,
/// in the format's own channel type, plus a format-specific fill.
fn tagged(format: PixelFormat, w: u32, h: u32) -> ImageBuf {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            match format {
                PixelFormat::Rgba8 => data.extend_from_slice(&[x as u8, y as u8, 0xA5, 0xFF]),
                PixelFormat::Rgba16 => {
                    for v in [x as u16, y as u16, 0xBEEF, 0xFFFF] {
                        data.extend_from_slice(&v.to_ne_bytes());
                    }
                }
                _ => {
                    for v in [x as f32, y as f32, 0.25, 1.0] {
                        data.extend_from_slice(&v.to_ne_bytes());
                    }
                }
            }
        }
    }
    ImageBuf::new(w, h, format, data).unwrap()
}

/// The (x, y) tag stored at pixel (x, y) of `img`.
fn tag(img: &ImageBuf, x: u32, y: u32) -> (u32, u32) {
    let bpp = img.format().bytes_per_pixel();
    let p = &img.row(y)[x as usize * bpp..(x as usize + 1) * bpp];
    match img.format() {
        PixelFormat::Rgba8 => {
            assert_eq!(&p[2..], &[0xA5, 0xFF]);
            (u32::from(p[0]), u32::from(p[1]))
        }
        PixelFormat::Rgba16 => {
            let c = |i: usize| u16::from_ne_bytes([p[2 * i], p[2 * i + 1]]);
            assert_eq!((c(2), c(3)), (0xBEEF, 0xFFFF));
            (u32::from(c(0)), u32::from(c(1)))
        }
        _ => {
            let c = |i: usize| f32::from_ne_bytes(p[4 * i..4 * i + 4].try_into().unwrap());
            assert_eq!((c(2), c(3)), (0.25, 1.0));
            (c(0) as u32, c(1) as u32)
        }
    }
}

/// Odd, even, thin and tile-crossing shapes.
const SHAPES: [(u32, u32); 6] = [(5, 3), (4, 4), (1, 7), (7, 1), (1, 1), (130, 70)];

/// Check that `out` holds source pixel (x, y) at `to(x, y)` for every pixel.
fn assert_moves(
    src: &ImageBuf,
    out: &ImageBuf,
    size: (u32, u32),
    to: impl Fn(u32, u32) -> (u32, u32),
) {
    assert_eq!((out.width(), out.height()), size);
    assert_eq!(out.format(), src.format());
    for y in 0..src.height() {
        for x in 0..src.width() {
            let (dx, dy) = to(x, y);
            assert_eq!(
                tag(out, dx, dy),
                (x, y),
                "{:?} source ({x}, {y})",
                src.format()
            );
        }
    }
}

#[test]
fn rotate_90_moves_pixels() {
    for format in FORMATS {
        for (w, h) in SHAPES {
            let img = tagged(format, w, h);
            let out = rotate90(&img);
            // Clockwise: the top-left corner goes to the top-right.
            assert_moves(&img, &out, (h, w), |x, y| (h - 1 - y, x));
            assert_eq!(rotate(&img, Rotation::Cw90).data(), out.data());
        }
    }
    // A 2 x 1 picture: [a b] becomes a column with a on top.
    let img = tagged(PixelFormat::Rgba8, 2, 1);
    let out = rotate90(&img);
    assert_eq!((tag(&out, 0, 0), tag(&out, 0, 1)), ((0, 0), (1, 0)));
}

#[test]
fn rotate_180_moves_pixels() {
    for format in FORMATS {
        for (w, h) in SHAPES {
            let img = tagged(format, w, h);
            let out = rotate180(&img);
            assert_moves(&img, &out, (w, h), |x, y| (w - 1 - x, h - 1 - y));
            assert_eq!(rotate(&img, Rotation::Cw180).data(), out.data());
        }
    }
}

#[test]
fn rotate_270_moves_pixels() {
    for format in FORMATS {
        for (w, h) in SHAPES {
            let img = tagged(format, w, h);
            let out = rotate270(&img);
            // Counterclockwise: the top-left corner goes to the bottom-left.
            assert_moves(&img, &out, (h, w), |x, y| (y, w - 1 - x));
            assert_eq!(rotate(&img, Rotation::Cw270).data(), out.data());
        }
    }
}

#[test]
fn four_quarter_turns_are_identity() {
    for format in FORMATS {
        for (w, h) in SHAPES {
            let img = tagged(format, w, h);
            let mut turned = img.clone();
            for turn in 1..=4 {
                turned = rotate90(&turned);
                let expect = if turn % 2 == 1 { (h, w) } else { (w, h) };
                assert_eq!((turned.width(), turned.height()), expect);
            }
            assert_eq!(turned.data(), img.data(), "{format:?} {w}x{h}");
            for r in [Rotation::Cw90, Rotation::Cw180, Rotation::Cw270] {
                let back = rotate(&rotate(&img, r), r.inverse());
                assert_eq!(back.data(), img.data());
            }
            assert_eq!(rotate180(&rotate180(&img)).data(), img.data());
            // Two 270 turns equal one 180 turn.
            assert_eq!(rotate270(&rotate270(&img)).data(), rotate180(&img).data());
        }
    }
}

#[test]
fn flip_horizontal_moves_pixels() {
    for format in FORMATS {
        for (w, h) in SHAPES {
            let img = tagged(format, w, h);
            let out = flip_horizontal(&img);
            assert_moves(&img, &out, (w, h), |x, y| (w - 1 - x, y));
            assert_eq!(flip_horizontal(&out).data(), img.data());
        }
    }
}

#[test]
fn flip_vertical_moves_pixels() {
    for format in FORMATS {
        for (w, h) in SHAPES {
            let img = tagged(format, w, h);
            let out = flip_vertical(&img);
            assert_moves(&img, &out, (w, h), |x, y| (x, h - 1 - y));
            assert_eq!(flip_vertical(&out).data(), img.data());
            // Both flips together are a half turn.
            assert_eq!(flip_horizontal(&out).data(), rotate180(&img).data());
        }
    }
}

#[test]
fn crop_region_exact() {
    for format in FORMATS {
        let img = tagged(format, 9, 6);
        for rect in [
            PixelRect::new(2, 1, 4, 3),
            PixelRect::new(0, 0, 1, 1),
            PixelRect::new(8, 5, 1, 1),
            PixelRect::new(0, 2, 9, 1),
            PixelRect::new(3, 0, 1, 6),
        ] {
            let out = crop(&img, rect).unwrap();
            assert_eq!((out.width(), out.height()), (rect.width, rect.height));
            assert_eq!(out.format(), format);
            for y in 0..rect.height {
                for x in 0..rect.width {
                    assert_eq!(
                        tag(&out, x, y),
                        (rect.x + x, rect.y + y),
                        "{format:?} {rect:?}"
                    );
                }
            }
        }
        // The whole image is a cheap clone.
        let all = crop(&img, PixelRect::new(0, 0, 9, 6)).unwrap();
        assert!(std::ptr::eq(all.data(), img.data()));
    }
    // A selection in image coordinates snaps to whole pixels.
    let img = tagged(PixelFormat::Rgba8, 9, 6);
    let rect = PixelRect::from_selection(Rect::new(1.6, 0.4, 3.0, 2.2), 9, 6).unwrap();
    assert_eq!(rect, PixelRect::new(2, 0, 3, 3));
    assert_eq!(tag(&crop(&img, rect).unwrap(), 0, 0), (2, 0));
}

#[test]
fn crop_out_of_bounds_is_error() {
    let img = tagged(PixelFormat::Rgba16, 9, 6);
    for rect in [
        PixelRect::new(0, 0, 10, 6),
        PixelRect::new(0, 0, 9, 7),
        PixelRect::new(8, 0, 2, 1),
        PixelRect::new(0, 5, 1, 2),
        PixelRect::new(9, 0, 1, 1),
        PixelRect::new(0, 6, 1, 1),
        PixelRect::new(u32::MAX, 0, 2, 1),
        PixelRect::new(0, u32::MAX - 1, 1, 5),
        PixelRect::new(1, 1, 0, 3),
        PixelRect::new(1, 1, 3, 0),
    ] {
        assert!(
            matches!(crop(&img, rect), Err(Error::Invalid(_))),
            "{rect:?}"
        );
    }
    // A selection entirely outside the image gives no rectangle at all.
    assert_eq!(
        PixelRect::from_selection(Rect::new(20.0, 20.0, 5.0, 5.0), 9, 6),
        None
    );
}
