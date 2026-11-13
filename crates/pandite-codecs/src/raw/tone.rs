//! Default display rendering of developed camera RAW images.
//!
//! The sensor pipeline ends in linear sRGB scaled so that 1.0 is the
//! sensor's clipping point. Shown as it is, that image looks dark and flat
//! next to the camera's own JPEG: a camera meters a mid-gray subject a few
//! stops below clipping to keep highlight headroom, and its JPEG engine
//! then applies a tone curve. This module is pandite's own default
//! rendering, designed from first principles:
//!
//! - Exposure normalization: a mid-gray subject metered by the camera
//!   lands about 3.7 EV below sensor clipping (the headroom of standard
//!   output sensitivity metering, ISO 12232). The curve maps that point,
//!   linear 0.075, to display mid-gray 0.18 (sRGB code 118).
//! - Tone curve: the Hill (Naka-Rushton) response `x^n / (x^n + K^n)`, a
//!   logistic sigmoid over log exposure like the characteristic curve of
//!   film or the response of photoreceptors, normalized so that sensor
//!   clipping maps to display white: `y = (1 + K^n) x^n / (x^n + K^n)`
//!   with `n = 1.8` and `K = 0.18`. Its log-log slope is 1.5 at mid-gray
//!   (the extra contrast displays need in dim surroundings), it deepens
//!   the shadows (toe) and rolls highlights off smoothly into white with
//!   slope 0.08 at clipping (shoulder).
//! - Hue stability: the curve acts on luminance `L` (Rec. 709 weights of
//!   linear sRGB) and every channel is scaled by the same factor
//!   `y(L) / L`, so channel ratios, and with them hue and saturation, stay
//!   as developed. A pixel that would leave the gamut is moved toward the
//!   gray of its new luminance until its largest channel is 1: hue angle
//!   and luminance stay, chroma shrinks, the way highlights desaturate on
//!   film.
//!
//! Measured against the camera's embedded JPEG previews (one parameter set
//! for every file, nothing fitted per file): see the test
//! `raw_default_rendering_matches_preview_brightness` for the tolerance
//! and the figures.
//!
//! The curve is applied in linear light before the sRGB transfer, through
//! a table of the scale factor `y(L) / L`.

use std::sync::OnceLock;

/// Hill exponent n: contrast of the curve.
pub(crate) const HILL_N: f64 = 1.8;
/// Hill half-response point K (in units of sensor clipping): sets the
/// exposure normalization together with `HILL_N`.
pub(crate) const HILL_K: f64 = 0.18;

/// Entries of the scale table over luminance 0..=1.
pub(crate) const SCALE_STEPS: usize = 4096;
/// Entries of the linear to sRGB 8-bit table over 0..=1.
pub(crate) const SRGB8_STEPS: usize = 16384;

/// Rec. 709 luminance weights of linear sRGB.
pub(crate) const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The display tone curve for one exposure gain, as tables.
#[derive(Clone, Debug)]
pub(crate) struct Tone {
    /// `y(L) / L` at `L = i / (SCALE_STEPS - 1)`, plus one guard entry.
    scale: Vec<f32>,
}

/// The curve: input `x` in units of sensor clipping (already multiplied
/// by the exposure gain), normalized so that `gain` maps to 1.
pub(crate) fn curve(x: f64, gain: f64) -> f64 {
    let kn = HILL_K.powf(HILL_N);
    let f = |v: f64| {
        let p = v.max(0.0).powf(HILL_N);
        p / (p + kn)
    };
    let white = f(gain.max(1e-6));
    (f(x * gain) / white).clamp(0.0, 1.0)
}

impl Tone {
    /// The tables for an exposure gain (1 for ARW; `2^BaselineExposure`
    /// for DNG). Sensor clipping times `gain` maps to white.
    pub(crate) fn new(gain: f64) -> Self {
        let gain = if gain.is_finite() && gain > 0.0 {
            gain
        } else {
            1.0
        };
        let mut scale: Vec<f32> = (0..SCALE_STEPS)
            .map(|i| {
                let l = i as f64 / (SCALE_STEPS - 1) as f64;
                if i == 0 {
                    0.0
                } else {
                    (curve(l, gain) / l) as f32
                }
            })
            .collect();
        let last = scale[SCALE_STEPS - 1];
        scale.push(last);
        Tone { scale }
    }

    /// The shared tables for gain 1.
    pub(crate) fn unit() -> &'static Tone {
        static TONE: OnceLock<Tone> = OnceLock::new();
        TONE.get_or_init(|| Tone::new(1.0))
    }

    /// The scale table (with its guard entry), for the vector paths.
    pub(crate) fn table(&self) -> &[f32] {
        &self.scale
    }

    /// Map one linear sRGB pixel (channels 0..=1) to its display value
    /// (channels 0..=1, still linear). The reference implementation every
    /// vector path must match exactly.
    #[inline]
    #[allow(clippy::manual_clamp)]
    pub(crate) fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let l = LUMA[0] * rgb[0] + LUMA[1] * rgb[1] + LUMA[2] * rgb[2];
        // Lower bound first: a NaN becomes 0 like in the vector paths.
        let pos = l.max(0.0).min(1.0) * (SCALE_STEPS - 1) as f32;
        let i = pos as usize;
        let t = pos - i as f32;
        let a = self.scale[i];
        let b = self.scale[i + 1];
        let s = a + (b - a) * t;
        let mut out = [rgb[0] * s, rgb[1] * s, rgb[2] * s];
        let max = out[0].max(out[1]).max(out[2]);
        if max > 1.0 {
            // Out of gamut: keep hue and luminance, reduce chroma.
            let y = (l * s).min(1.0);
            let k = (1.0 - y) / (max - y);
            for c in &mut out {
                *c = y + (*c - y) * k;
            }
        }
        out
    }
}

/// A 3 x 3 color matrix applied to `v`, each result clipped to 0..=1 with
/// the lower bound first (a NaN becomes 0, the same in every vector
/// path). The sums run left to right, as the vector paths do them.
#[inline]
#[allow(clippy::manual_clamp)]
pub(crate) fn matrix_clip(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let mut o = [0f32; 3];
    for r in 0..3 {
        o[r] = (m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
            .max(0.0)
            .min(1.0);
    }
    o
}

/// sRGB 8-bit code of linear `i / (SRGB8_STEPS - 1)`, followed by
/// [`TABLE_PAD`] zero bytes so vector code may load 4 bytes at any index.
pub(crate) fn srgb8_table() -> &'static [u8] {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t: Vec<u8> = (0..SRGB8_STEPS)
            .map(|i| {
                let v = i as f32 / (SRGB8_STEPS - 1) as f32;
                (crate::util::linear_to_srgb(v) * 255.0 + 0.5) as u8
            })
            .collect();
        t.extend_from_slice(&[0; TABLE_PAD]);
        t
    })
}

/// Zero bytes after the 8-bit table.
pub(crate) const TABLE_PAD: usize = 4;

/// The table index of a linear value: `v` clipped to 0..=1 (lower bound
/// first, so NaN gives 0), scaled and rounded half up.
#[inline]
#[allow(clippy::manual_clamp)]
pub(crate) fn srgb8_index(v: f32) -> usize {
    (v.max(0.0).min(1.0) * (SRGB8_STEPS - 1) as f32 + 0.5) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_curve_anchors() {
        // Clipping maps to white, black to black, the curve rises.
        assert!((curve(1.0, 1.0) - 1.0).abs() < 1e-12);
        assert_eq!(curve(0.0, 1.0), 0.0);
        let mut last = 0.0;
        for i in 1..=1000 {
            let y = curve(f64::from(i) / 1000.0, 1.0);
            assert!(y > last, "monotonic at {i}");
            last = y;
        }
        // Mid-gray placement: 0.075 to display 0.18 within 0.005.
        let mid = curve(0.075, 1.0);
        assert!((mid - 0.18).abs() < 0.005, "mid-gray {mid}");
        // Contrast at mid-gray: log-log slope 1.5 within 0.05.
        let (a, b) = (0.075 * 0.99, 0.075 * 1.01);
        let slope = (curve(b, 1.0).ln() - curve(a, 1.0).ln()) / (b.ln() - a.ln());
        assert!((slope - 1.5).abs() < 0.05, "slope {slope}");
        // Highlight roll-off: the slope at clipping is small.
        let top = (curve(1.0, 1.0) - curve(0.99, 1.0)) / 0.01;
        assert!(top < 0.1, "shoulder slope {top}");
        // A gain moves the white point: clipping times gain maps to white.
        assert!((curve(1.0, 2.0) - 1.0).abs() < 1e-12);
        assert!(curve(0.075, 2.0) > curve(0.075, 1.0));
    }

    #[test]
    fn tone_keeps_hue_and_gamut() {
        let tone = Tone::unit();
        // Neutral stays neutral.
        let g = tone.apply([0.075, 0.075, 0.075]);
        assert!((g[0] - g[1]).abs() < 1e-6 && (g[1] - g[2]).abs() < 1e-6);
        assert!((g[0] - 0.18).abs() < 0.006, "{g:?}");
        // In gamut: channel ratios are kept exactly.
        let c = [0.02f32, 0.05, 0.08];
        let o = tone.apply(c);
        assert!((o[0] / o[1] - c[0] / c[1]).abs() < 1e-5);
        assert!((o[2] / o[1] - c[2] / c[1]).abs() < 1e-5);
        // Saturated blue that would clip: stays inside, hue order kept,
        // luminance equals the curve's.
        let c = [0.05f32, 0.1, 0.9];
        let o = tone.apply(c);
        assert!(o.iter().all(|v| (0.0..=1.0).contains(v)), "{o:?}");
        assert!(o[2] > o[1] && o[1] > o[0], "{o:?}");
        let l = LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2];
        let lo = LUMA[0] * o[0] + LUMA[1] * o[1] + LUMA[2] * o[2];
        assert!(
            (f64::from(lo) - curve(f64::from(l), 1.0)).abs() < 0.01,
            "{lo}"
        );
        // The 8-bit table: black, mid-gray, white.
        let t = srgb8_table();
        assert_eq!(t[srgb8_index(0.0)], 0);
        assert_eq!(t[srgb8_index(1.0)], 255);
        assert_eq!(t[srgb8_index(0.18)], 118);
        assert_eq!(t[srgb8_index(f32::NAN)], 0);
    }
}
