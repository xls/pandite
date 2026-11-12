// Upload passes: convert staged pixels to premultiplied linear light, then
// build each mip level from the one above with an exact box filter. Every
// pass draws one triangle that covers the bound viewport.

struct ConvertParams {
    // Framebuffer position of the staging texture's top-left texel.
    origin: vec2<i32>,
    pad: vec2<i32>,
};

// Float staging data (rgba8unorm-srgb or rgba32float) and the mip source.
@group(0) @binding(0) var src_float: texture_2d<f32>;
// 16-bit staging data (rgba16uint).
@group(0) @binding(1) var src_uint: texture_2d<u32>;
@group(0) @binding(2) var<uniform> params: ConvertParams;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn source_texel(pos: vec4<f32>) -> vec2<i32> {
    return vec2<i32>(floor(pos.xy)) - params.origin;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn premultiply(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

// NaN becomes zero. Infinities are left for the clamp below.
fn not_nan(v: f32) -> f32 {
    let bits = bitcast<u32>(v) & 0x7fffffffu;
    return select(v, 0.0, bits > 0x7f800000u);
}

@fragment
fn fs_convert_srgb8(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // The staging view is rgba8unorm-srgb, so the load already decodes to linear.
    return premultiply(textureLoad(src_float, source_texel(pos), 0));
}

@fragment
fn fs_convert_rgba16(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let v = vec4<f32>(textureLoad(src_uint, source_texel(pos), 0)) / 65535.0;
    return premultiply(vec4<f32>(srgb_to_linear(v.rgb), v.a));
}

@fragment
fn fs_convert_float(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let raw = textureLoad(src_float, source_texel(pos), 0);
    let rgb = clamp(vec3<f32>(not_nan(raw.r), not_nan(raw.g), not_nan(raw.b)), vec3<f32>(0.0), vec3<f32>(65504.0));
    let a = clamp(not_nan(raw.a), 0.0, 1.0);
    return vec4<f32>(rgb * a, a);
}

// One destination texel is the area-weighted mean of the source texels it
// covers. Odd sizes give a ratio between 2 and 3, so edge texels count with
// their partial coverage instead of being dropped. Loads from an sRGB view
// return linear values and the sRGB target encodes again, so the averaging
// happens in linear light on premultiplied color.
@fragment
fn fs_mip(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let src_dim = vec2<i32>(textureDimensions(src_float, 0));
    let src = vec2<f32>(src_dim);
    let dst = max(floor(src * 0.5), vec2<f32>(1.0));
    let ratio = src / dst;
    let lo = floor(pos.xy) * ratio;
    let hi = lo + ratio;
    let first = vec2<i32>(floor(lo));
    let last = min(vec2<i32>(ceil(hi)) - vec2<i32>(1), src_dim - vec2<i32>(1));
    var sum = vec4<f32>(0.0);
    for (var y = first.y; y <= last.y; y = y + 1) {
        let wy = min(f32(y + 1), hi.y) - max(f32(y), lo.y);
        for (var x = first.x; x <= last.x; x = x + 1) {
            let wx = min(f32(x + 1), hi.x) - max(f32(x), lo.x);
            sum = sum + textureLoad(src_float, vec2<i32>(x, y), 0) * (wx * wy);
        }
    }
    return sum / (ratio.x * ratio.y);
}
