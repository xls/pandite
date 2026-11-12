//! Apply an EXIF orientation to a decoded image so it comes out upright.

use crate::util::alloc;
use pandite_core::{ImageBuf, Orientation, Result};

/// Return `image` transformed so that a viewer shows it upright.
pub(crate) fn apply(image: ImageBuf, orientation: Orientation) -> Result<ImageBuf> {
    if orientation == Orientation::Normal {
        return Ok(image);
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    let bpp = image.format().bytes_per_pixel();
    let src = image.data();
    let (ow, oh) = if orientation.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    };
    let mut out = alloc(src.len(), image.width(), image.height())?;
    for oy in 0..oh {
        let row = &mut out[oy * ow * bpp..(oy + 1) * ow * bpp];
        for ox in 0..ow {
            let (sx, sy) = match orientation {
                Orientation::Normal => (ox, oy),
                Orientation::FlipH => (w - 1 - ox, oy),
                Orientation::Rotate180 => (w - 1 - ox, h - 1 - oy),
                Orientation::FlipV => (ox, h - 1 - oy),
                Orientation::Transpose => (oy, ox),
                Orientation::Rotate90 => (oy, h - 1 - ox),
                Orientation::Transverse => (w - 1 - oy, h - 1 - ox),
                Orientation::Rotate270 => (w - 1 - oy, ox),
            };
            let s = (sy * w + sx) * bpp;
            row[ox * bpp..(ox + 1) * bpp].copy_from_slice(&src[s..s + bpp]);
        }
    }
    ImageBuf::new(ow as u32, oh as u32, image.format(), out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::PixelFormat;

    /// A 3 x 2 image whose red channel holds the pixel index:
    /// row 0 = 0 1 2, row 1 = 3 4 5.
    fn sample() -> ImageBuf {
        let mut d = Vec::new();
        for i in 0..6u8 {
            d.extend_from_slice(&[i, 0, 0, 255]);
        }
        ImageBuf::new(3, 2, PixelFormat::Rgba8, d).unwrap()
    }

    fn reds(img: &ImageBuf) -> Vec<u8> {
        img.data().chunks_exact(4).map(|p| p[0]).collect()
    }

    #[test]
    fn every_orientation_maps_pixels() {
        let cases: [(Orientation, (u32, u32), [u8; 6]); 8] = [
            (Orientation::Normal, (3, 2), [0, 1, 2, 3, 4, 5]),
            (Orientation::FlipH, (3, 2), [2, 1, 0, 5, 4, 3]),
            (Orientation::Rotate180, (3, 2), [5, 4, 3, 2, 1, 0]),
            (Orientation::FlipV, (3, 2), [3, 4, 5, 0, 1, 2]),
            (Orientation::Transpose, (2, 3), [0, 3, 1, 4, 2, 5]),
            // Rotate 90 degrees clockwise: the left column becomes the top row.
            (Orientation::Rotate90, (2, 3), [3, 0, 4, 1, 5, 2]),
            (Orientation::Transverse, (2, 3), [5, 2, 4, 1, 3, 0]),
            (Orientation::Rotate270, (2, 3), [2, 5, 1, 4, 0, 3]),
        ];
        for (o, dims, expected) in cases {
            let out = apply(sample(), o).unwrap();
            assert_eq!((out.width(), out.height()), dims, "{o:?}");
            assert_eq!(reds(&out), expected, "{o:?}");
        }
    }
}
