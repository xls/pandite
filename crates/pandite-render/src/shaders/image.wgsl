// Content image pass. One quad per texture tile. The texture holds
// premultiplied linear color with a full mip chain.

struct FrameParams {
    target_size: vec2<f32>,
    // Linear multiplier applied to high dynamic range content (2^EV).
    exposure: f32,
    // 1 when the color target is not an sRGB format and the shader encodes.
    encode_srgb: u32,
    checker_a: vec4<f32>,
    checker_b: vec4<f32>,
    checker_cell: f32,
    lod_bias: f32,
    pad0: f32,
    pad1: f32,
};

struct DrawParams {
    // Destination rectangle in target pixels: x, y, width, height.
    dst: vec4<f32>,
    // Texture coordinates of the destination corners: u0, v0, u1, v1.
    uv: vec4<f32>,
    tex_size: vec2<f32>,
    // Screen position the checkerboard is anchored to (image top-left).
    checker_origin: vec2<f32>,
    // 1 selects crisp texel sampling for strong magnification.
    crisp: u32,
    // 1 applies exposure and tone mapping (high dynamic range content).
    hdr: u32,
    // Explicit level of detail for the resample pass.
    lod: f32,
    pad: f32,
};

@group(0) @binding(0) var<uniform> frame: FrameParams;
@group(0) @binding(1) var image_sampler: sampler;
@group(1) @binding(0) var image_texture: texture_2d<f32>;
@group(1) @binding(1) var<uniform> draw: DrawParams;

@vertex
fn vs_quad(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    let p = draw.dst.xy + corner * draw.dst.zw;
    let ndc = p / frame.target_size * 2.0 - vec2<f32>(1.0);
    return vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
}

// Texture coordinates come from the fragment position, not from vertex
// interpolation, so deep zoom on huge images keeps full precision.
fn image_uv(pos: vec2<f32>) -> vec2<f32> {
    let t = (pos - draw.dst.xy) / draw.dst.zw;
    return mix(draw.uv.xy, draw.uv.zw, t);
}

fn sample_image(pos: vec2<f32>) -> vec4<f32> {
    let uv = image_uv(pos);
    let texel = uv * draw.tex_size;
    // Texels per screen pixel. Computed before any branch (uniform control flow).
    let footprint = max(fwidth(texel), vec2<f32>(1.0e-6));
    if (draw.crisp != 0u) {
        // Crisp magnification: hold each texel flat and blend only across a
        // one screen pixel wide seam, so edges are sharp but not jagged at
        // fractional zoom factors.
        let seam = floor(texel + vec2<f32>(0.5));
        let snapped = seam + clamp((texel - seam) / footprint, vec2<f32>(-0.5), vec2<f32>(0.5));
        return textureSampleLevel(image_texture, image_sampler, snapped / draw.tex_size, 0.0);
    }
    // Trilinear and anisotropic minification, bilinear magnification.
    return textureSampleBias(image_texture, image_sampler, uv, frame.lod_bias);
}

// Identity below the knee, then an exponential shoulder that reaches 1.0
// asymptotically. Applied to the largest channel and scaled back, so hue is
// kept and standard range content passes unchanged.
const KNEE: f32 = 0.75;

fn tone_map(rgb: vec3<f32>) -> vec3<f32> {
    let peak = max(max(rgb.r, rgb.g), rgb.b);
    if (peak <= KNEE) {
        return rgb;
    }
    let span = 1.0 - KNEE;
    let mapped = KNEE + span * (1.0 - exp(-(peak - KNEE) / span));
    return rgb * (mapped / peak);
}

fn display_color(c: vec4<f32>) -> vec4<f32> {
    if (draw.hdr == 0u || c.a <= 0.0) {
        return c;
    }
    let straight = c.rgb / c.a;
    return vec4<f32>(tone_map(straight * frame.exposure) * c.a, c.a);
}

fn checker(pos: vec2<f32>) -> vec3<f32> {
    let cell = floor((pos - draw.checker_origin) / frame.checker_cell);
    let odd = (i32(cell.x) + i32(cell.y)) & 1;
    return select(frame.checker_a.rgb, frame.checker_b.rgb, odd != 0);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = x * 12.92;
    let high = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, x <= vec3<f32>(0.0031308));
}

@fragment
fn fs_image(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = display_color(sample_image(pos.xy));
    // Premultiplied "over" onto the checkerboard, in linear light.
    var rgb = c.rgb + checker(pos.xy) * (1.0 - c.a);
    if (frame.encode_srgb != 0u) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}

// Copies a tile into the downscaled overview texture at a fixed mip level.
@fragment
fn fs_resample(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureSampleLevel(image_texture, image_sampler, image_uv(pos.xy), draw.lod);
}
