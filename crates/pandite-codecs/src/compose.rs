//! Canvas compositing for frame animations (GIF, APNG, animated WebP).
//!
//! All canvases are RGBA8 with straight alpha. Frames are drawn into a
//! sub-rectangle that is clipped to the canvas.

/// A rectangle on the canvas in pixels, already clipped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Region {
    /// Clip a frame rectangle to a canvas of `cw` x `ch`.
    pub(crate) fn clip(x: u32, y: u32, w: u32, h: u32, cw: usize, ch: usize) -> Region {
        let (x, y) = (x as usize, y as usize);
        if x >= cw || y >= ch {
            return Region::default();
        }
        Region {
            x,
            y,
            w: (w as usize).min(cw - x),
            h: (h as usize).min(ch - y),
        }
    }
}

/// Blend straight-alpha `src` over straight-alpha `dst` (both RGBA8).
#[inline]
pub(crate) fn blend_over(dst: &mut [u8], src: &[u8]) {
    let sa = u32::from(src[3]);
    if sa == 255 {
        dst[..4].copy_from_slice(&src[..4]);
        return;
    }
    if sa == 0 {
        return;
    }
    let da = u32::from(dst[3]);
    if da == 0 {
        dst[..4].copy_from_slice(&src[..4]);
        return;
    }
    // Work in units of 255 * 255 to keep precision without floats.
    let dw = da * (255 - sa);
    let out_a = sa * 255 + dw;
    for c in 0..3 {
        let v = (u32::from(src[c]) * sa * 255 + u32::from(dst[c]) * dw + out_a / 2) / out_a;
        dst[c] = v.min(255) as u8;
    }
    dst[3] = ((out_a + 127) / 255).min(255) as u8;
}

/// Set every pixel of `region` to transparent black.
pub(crate) fn clear(canvas: &mut [u8], cw: usize, region: Region) {
    for row in region.y..region.y + region.h {
        let start = (row * cw + region.x) * 4;
        canvas[start..start + region.w * 4].fill(0);
    }
}

/// Copy `region` out of the canvas (for "restore previous" disposal).
pub(crate) fn save(canvas: &[u8], cw: usize, region: Region) -> Vec<u8> {
    let mut out = Vec::with_capacity(region.w * region.h * 4);
    for row in region.y..region.y + region.h {
        let start = (row * cw + region.x) * 4;
        out.extend_from_slice(&canvas[start..start + region.w * 4]);
    }
    out
}

/// Put back a region saved with [`save`].
pub(crate) fn restore(canvas: &mut [u8], cw: usize, region: Region, saved: &[u8]) {
    for (i, row) in (region.y..region.y + region.h).enumerate() {
        let start = (row * cw + region.x) * 4;
        canvas[start..start + region.w * 4]
            .copy_from_slice(&saved[i * region.w * 4..(i + 1) * region.w * 4]);
    }
}

/// Draw an RGBA8 frame of `fw` pixels per row into `region`, either
/// replacing the canvas pixels (`blend == false`) or blending over them.
pub(crate) fn draw(
    canvas: &mut [u8],
    cw: usize,
    region: Region,
    frame: &[u8],
    fw: usize,
    blend: bool,
) {
    for row in 0..region.h {
        let src = &frame[row * fw * 4..row * fw * 4 + region.w * 4];
        let start = ((region.y + row) * cw + region.x) * 4;
        let dst = &mut canvas[start..start + region.w * 4];
        if blend {
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                blend_over(d, s);
            }
        } else {
            dst.copy_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_over_matches_porter_duff() {
        let mut d = [0, 0, 255, 255];
        blend_over(&mut d, &[255, 0, 0, 128]);
        assert_eq!(d, [128, 0, 127, 255]);
        let mut d = [10, 20, 30, 0];
        blend_over(&mut d, &[1, 2, 3, 77]);
        assert_eq!(d, [1, 2, 3, 77]);
        let mut d = [10, 20, 30, 40];
        blend_over(&mut d, &[1, 2, 3, 0]);
        assert_eq!(d, [10, 20, 30, 40]);
        // Half over half: alpha 0.75, color weighted 2:1 toward the source.
        let mut d = [0, 0, 0, 128];
        blend_over(&mut d, &[255, 255, 255, 128]);
        assert_eq!(d[3], 192);
        assert!((169..=171).contains(&d[0]), "{d:?}");
    }

    #[test]
    fn region_clips_to_canvas() {
        assert_eq!(
            Region::clip(3, 3, 10, 10, 5, 4),
            Region {
                x: 3,
                y: 3,
                w: 2,
                h: 1
            }
        );
        assert_eq!(Region::clip(9, 0, 1, 1, 5, 4), Region::default());
        let mut canvas = vec![9u8; 5 * 4 * 4];
        let r = Region::clip(1, 1, 2, 2, 5, 4);
        let saved = save(&canvas, 5, r);
        clear(&mut canvas, 5, r);
        assert_eq!(canvas[(5 + 1) * 4..(5 + 3) * 4], [0u8; 8]);
        restore(&mut canvas, 5, r, &saved);
        assert!(canvas.iter().all(|&b| b == 9));
    }
}
