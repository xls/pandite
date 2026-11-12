//! sRGB transfer helpers shared by the passes.

use pandite_core::Color;
use std::sync::OnceLock;

/// sRGB-encoded value (0 to 1) to linear light.
pub(crate) fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light (0 to 1) to the sRGB encoding.
#[cfg(test)]
pub(crate) fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear value of every 8-bit sRGB code, computed once.
fn decode_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0; 256];
        for (code, slot) in table.iter_mut().enumerate() {
            *slot = srgb_to_linear(code as f32 / 255.0);
        }
        table
    })
}

/// A straight-alpha sRGB color as linear RGB with straight alpha.
pub(crate) fn linear(c: Color) -> [f32; 4] {
    let table = decode_table();
    [
        table[usize::from(c.r)],
        table[usize::from(c.g)],
        table[usize::from(c.b)],
        f32::from(c.a) / 255.0,
    ]
}

/// A straight-alpha sRGB color as premultiplied linear RGBA.
pub(crate) fn linear_premultiplied(c: Color) -> [f32; 4] {
    let [r, g, b, a] = linear(c);
    [r * a, g * a, b * a, a]
}

/// The clear value for a color target: linear for sRGB formats (the
/// hardware encodes on store), the raw sRGB fraction otherwise.
pub(crate) fn clear_value(c: Color, srgb_target: bool) -> wgpu::Color {
    let [r, g, b, a] = if srgb_target {
        linear(c)
    } else {
        [
            f32::from(c.r) / 255.0,
            f32::from(c.g) / 255.0,
            f32::from(c.b) / 255.0,
            f32::from(c.a) / 255.0,
        ]
    };
    wgpu::Color {
        r: f64::from(r),
        g: f64::from(g),
        b: f64::from(b),
        a: f64::from(a),
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn srgb_round_trip_is_exact_for_8_bit_codes() {
        for code in 0..=255u8 {
            let lin = srgb_to_linear(f32::from(code) / 255.0);
            let back = (linear_to_srgb(lin) * 255.0).round() as u8;
            assert_eq!(code, back);
        }
        assert!((srgb_to_linear(188.0 / 255.0) - 0.5).abs() < 0.005);
    }
}
