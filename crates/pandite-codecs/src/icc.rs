//! Conversion of embedded ICC profiles to sRGB.
//!
//! Decoders hand pixels plus the raw profile bytes to these functions. With
//! the `icc` feature the pixels are converted in place through `moxcms`;
//! profiles that already behave like sRGB are detected and skipped. Without
//! the feature, or when a profile cannot be parsed, pixels pass unchanged
//! (RGB) or the caller falls back to a naive conversion (CMYK).

#[cfg(feature = "icc")]
mod imp {
    use moxcms::{ColorProfile, DataColorSpace, Layout, TransformExecutor, TransformOptions};

    /// Pixels converted per call, so scratch memory stays small.
    const CHUNK: usize = 16 * 1024;

    fn parse(profile: Option<&[u8]>, space: DataColorSpace) -> Option<ColorProfile> {
        let p = ColorProfile::new_from_slice(profile?).ok()?;
        (p.color_space == space).then_some(p)
    }

    /// True when the transform leaves a grid of test colors within one step.
    fn is_near_identity(t: &(dyn TransformExecutor<u8> + Send + Sync)) -> bool {
        let mut src = Vec::with_capacity(5 * 5 * 5 * 4);
        for r in 0..5u8 {
            for g in 0..5u8 {
                for b in 0..5u8 {
                    src.extend_from_slice(&[r * 63, g * 63, b * 63, 255]);
                }
            }
        }
        let mut dst = vec![0u8; src.len()];
        if t.transform(&src, &mut dst).is_err() {
            return false;
        }
        src.iter().zip(&dst).all(|(a, b)| a.abs_diff(*b) <= 1)
    }

    pub(crate) fn to_srgb_rgba8(profile: Option<&[u8]>, rgba: &mut [u8]) {
        let Some(src) = parse(profile, DataColorSpace::Rgb) else {
            return;
        };
        let dst = ColorProfile::new_srgb();
        let Ok(t) = src.create_transform_8bit(
            Layout::Rgba,
            &dst,
            Layout::Rgba,
            TransformOptions::default(),
        ) else {
            return;
        };
        if is_near_identity(t.as_ref()) {
            return;
        }
        let mut tmp = vec![0u8; CHUNK * 4];
        for chunk in rgba.chunks_mut(CHUNK * 4) {
            let out = &mut tmp[..chunk.len()];
            if t.transform(chunk, out).is_ok() {
                chunk.copy_from_slice(out);
            }
        }
    }

    pub(crate) fn to_srgb_rgba16(profile: Option<&[u8]>, rgba: &mut [u16]) {
        let Some(src) = parse(profile, DataColorSpace::Rgb) else {
            return;
        };
        let dst = ColorProfile::new_srgb();
        if let Ok(t8) = src.create_transform_8bit(
            Layout::Rgba,
            &dst,
            Layout::Rgba,
            TransformOptions::default(),
        ) {
            if is_near_identity(t8.as_ref()) {
                return;
            }
        }
        let Ok(t) = src.create_transform_16bit(
            Layout::Rgba,
            &dst,
            Layout::Rgba,
            TransformOptions::default(),
        ) else {
            return;
        };
        let mut tmp = vec![0u16; CHUNK * 4];
        for chunk in rgba.chunks_mut(CHUNK * 4) {
            let out = &mut tmp[..chunk.len()];
            if t.transform(chunk, out).is_ok() {
                chunk.copy_from_slice(out);
            }
        }
    }

    pub(crate) fn cmyk_to_srgb_rgba8(profile: Option<&[u8]>, cmyk: &[u8], out: &mut [u8]) -> bool {
        let Some(src) = parse(profile, DataColorSpace::Cmyk) else {
            return false;
        };
        let dst = ColorProfile::new_srgb();
        let Ok(t) =
            src.create_transform_8bit(Layout::Rgba, &dst, Layout::Rgb, TransformOptions::default())
        else {
            return false;
        };
        let mut tmp = vec![0u8; CHUNK * 3];
        for (src_chunk, out_chunk) in cmyk.chunks(CHUNK * 4).zip(out.chunks_mut(CHUNK * 4)) {
            let px = src_chunk.len() / 4;
            let rgb = &mut tmp[..px * 3];
            if t.transform(&src_chunk[..px * 4], rgb).is_err() {
                return false;
            }
            for (o, c) in out_chunk.chunks_exact_mut(4).zip(rgb.chunks_exact(3)) {
                o.copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        true
    }
}

#[cfg(not(feature = "icc"))]
mod imp {
    pub(crate) fn to_srgb_rgba8(_profile: Option<&[u8]>, _rgba: &mut [u8]) {}

    pub(crate) fn to_srgb_rgba16(_profile: Option<&[u8]>, _rgba: &mut [u16]) {}

    pub(crate) fn cmyk_to_srgb_rgba8(
        _profile: Option<&[u8]>,
        _cmyk: &[u8],
        _out: &mut [u8],
    ) -> bool {
        false
    }
}

/// Convert RGBA8 pixels from the embedded RGB profile to sRGB in place.
/// No-op when the profile is absent, not RGB, unreadable or sRGB-like.
pub(crate) fn to_srgb_rgba8(profile: Option<&[u8]>, rgba: &mut [u8]) {
    imp::to_srgb_rgba8(profile, rgba);
}

/// Convert RGBA16 samples from the embedded RGB profile to sRGB in place.
pub(crate) fn to_srgb_rgba16(profile: Option<&[u8]>, rgba: &mut [u16]) {
    imp::to_srgb_rgba16(profile, rgba);
}

/// Convert true CMYK pixels (0 means no ink, 4 bytes each) to opaque RGBA8
/// through the embedded CMYK profile. Returns false when no usable profile
/// exists; the caller then applies a naive conversion.
pub(crate) fn cmyk_to_srgb_rgba8(profile: Option<&[u8]>, cmyk: &[u8], out: &mut [u8]) -> bool {
    imp::cmyk_to_srgb_rgba8(profile, cmyk, out)
}
