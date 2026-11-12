// Interface pass. Every DrawCmd becomes one instanced quad. Shapes use a
// signed distance to a rounded box for one pixel wide antialiased edges.
// Colors arrive premultiplied in linear light and blend with "over".

struct UiFrame {
    target_size: vec2<f32>,
    // 1 when the color target is not an sRGB format and the shader encodes.
    encode_srgb: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> ui: UiFrame;
@group(0) @binding(1) var ui_sampler: sampler;
@group(1) @binding(0) var ui_texture: texture_2d<f32>;

const KIND_FILL: u32 = 0u;
const KIND_BORDER: u32 = 1u;
const KIND_IMAGE: u32 = 2u;
const KIND_MASK: u32 = 3u;
const KIND_GLYPH: u32 = 4u;

// Coverage exponent that makes linear-light text blending look like the
// gamma-space blending fonts are designed for.
const TEXT_GAMMA: f32 = 1.8;

struct Instance {
    // x, y, width, height in target pixels.
    @location(0) rect: vec4<f32>,
    // u0, v0, u1, v1: normalized for images, atlas texels for glyphs.
    @location(1) uv: vec4<f32>,
    // Premultiplied linear color or tint.
    @location(2) color: vec4<f32>,
    // Corner radius, border width, kind, unused.
    @location(3) params: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) uv: vec4<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs_ui(@builtin(vertex_index) index: u32, inst: Instance) -> VsOut {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    let kind = u32(inst.params.z + 0.5);
    // Grow shapes by one pixel so the antialiased fringe is rasterized.
    // Glyph quads are pixel aligned and need no fringe.
    let grow = select(1.0, 0.0, kind == KIND_GLYPH);
    let lo = inst.rect.xy - vec2<f32>(grow);
    let size = inst.rect.zw + vec2<f32>(2.0 * grow);
    let p = lo + corner * size;
    let ndc = p / ui.target_size * 2.0 - vec2<f32>(1.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.rect = inst.rect;
    out.uv = inst.uv;
    out.color = inst.color;
    out.params = inst.params;
    return out;
}

fn rounded_box_distance(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let r = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn text_coverage(c: f32, color: vec4<f32>) -> f32 {
    var straight = vec3<f32>(0.0);
    if (color.a > 0.0) {
        straight = color.rgb / color.a;
    }
    let luma = pow(clamp(dot(straight, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0), 1.0 / 2.2);
    let light = pow(c, TEXT_GAMMA);
    let dark = 1.0 - pow(1.0 - c, TEXT_GAMMA);
    return mix(dark, light, luma);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = x * 12.92;
    let high = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, x <= vec3<f32>(0.0031308));
}

@fragment
fn fs_ui(in: VsOut) -> @location(0) vec4<f32> {
    let p = in.pos.xy;
    let kind = u32(in.params.z + 0.5);
    let half_size = in.rect.zw * 0.5;
    let center = in.rect.xy + half_size;
    let outer = clamp(0.5 - rounded_box_distance(p - center, half_size, in.params.x), 0.0, 1.0);

    // Sample for every kind so the implicit derivatives stay in uniform
    // control flow. Shapes ignore the result.
    let t = clamp((p - in.rect.xy) / max(in.rect.zw, vec2<f32>(1.0e-6)), vec2<f32>(0.0), vec2<f32>(1.0));
    let dims = vec2<f32>(textureDimensions(ui_texture, 0));
    let raw_uv = mix(in.uv.xy, in.uv.zw, t);
    let uv = select(raw_uv, raw_uv / dims, kind == KIND_GLYPH);
    let texel = textureSample(ui_texture, ui_sampler, uv);

    var color = in.color;
    var coverage = outer;
    if (kind == KIND_BORDER) {
        let width = in.params.y;
        let inner_half = half_size - vec2<f32>(width);
        var inner = 0.0;
        if (inner_half.x > 0.0 && inner_half.y > 0.0) {
            let inner_radius = max(in.params.x - width, 0.0);
            inner = clamp(0.5 - rounded_box_distance(p - center, inner_half, inner_radius), 0.0, 1.0);
        }
        coverage = clamp(outer - inner, 0.0, 1.0);
    } else if (kind == KIND_IMAGE) {
        color = texel * in.color;
    } else if (kind == KIND_MASK) {
        color = in.color * texel.r;
    } else if (kind == KIND_GLYPH) {
        coverage = text_coverage(texel.r, in.color);
    }
    var out = color * coverage;
    if (ui.encode_srgb != 0u && out.a > 0.0) {
        out = vec4<f32>(linear_to_srgb(out.rgb / out.a) * out.a, out.a);
    }
    return out;
}
