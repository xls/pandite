//! Gate G1: the base formats (JPEG, PNG, APNG, GIF, WebP, BMP) decode to
//! the expected pixels, including animation frames, timing, loop counts and
//! EXIF orientation.

mod common;

use common::hand::*;
use common::*;
use pandite_core::{
    Content, ContentKind, DecodeOptions, EncodeOptions, LoopCount, Orientation, PixelFormat,
};
use std::borrow::Cow;
use std::time::Duration;

const RED: [u8; 3] = [200, 30, 40];
const GREEN: [u8; 3] = [40, 190, 60];
const BLUE: [u8; 3] = [20, 60, 220];
const GRAY: [u8; 3] = [128, 128, 128];

fn jpeg(w: u16, h: u16, data: &[u8], color: jpeg_encoder::ColorType, q: u8) -> Vec<u8> {
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, q)
        .encode(data, w, h, color)
        .unwrap();
    out
}

fn anim(content: Content) -> pandite_core::Animation {
    match content {
        Content::Animation(a) => a,
        other => panic!("expected an animation, got {:?}", other.kind()),
    }
}

#[test]
fn decode_jpeg() {
    // Color blocks, 4:4:4 at quality 100: block centers come back within a
    // few levels of the source.
    let colors = [RED, GREEN, BLUE, GRAY];
    let (w, h, rgba) = blocks(&colors, 2, 16);
    let file = jpeg(
        w as u16,
        h as u16,
        &rgba,
        jpeg_encoder::ColorType::Rgba,
        100,
    );
    let img = still(&file, "blocks.jpg");
    assert_eq!((img.width(), img.height()), (32, 32));
    assert_eq!(img.format(), PixelFormat::Rgba8);
    for (i, c) in colors.iter().enumerate() {
        let (x, y) = ((i as u32 % 2) * 16 + 8, (i as u32 / 2) * 16 + 8);
        assert_near(px(&img, x, y), [c[0], c[1], c[2], 255], 3, "baseline block");
    }

    // Progressive coding decodes to the same pixels.
    let mut prog = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut prog, 100);
    enc.set_progressive(true);
    enc.encode(&rgba, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)
        .unwrap();
    let img = still(&prog, "prog.jpg");
    assert_near(
        px(&img, 24, 24),
        [128, 128, 128, 255],
        3,
        "progressive gray block",
    );
    assert_near(
        px(&img, 8, 8),
        [200, 30, 40, 255],
        3,
        "progressive red block",
    );

    // Grayscale JPEG expands to equal channels.
    let luma: Vec<u8> = (0..16 * 16)
        .map(|i| if i % 16 < 8 { 60 } else { 190 })
        .collect();
    let img = still(
        &jpeg(16, 16, &luma, jpeg_encoder::ColorType::Luma, 100),
        "g.jpg",
    );
    let p = px(&img, 2, 5);
    assert_eq!(p[0], p[1]);
    assert_eq!(p[1], p[2]);
    assert_near(p, [60, 60, 60, 255], 2, "gray left");
    assert_near(px(&img, 13, 5), [190, 190, 190, 255], 2, "gray right");

    // Adobe CMYK JPEG (inverted storage) converts to RGB.
    let mut cmyk = Vec::new();
    for y in 0..16 {
        for x in 0..32 {
            let c: [u8; 4] = match (x / 16, y / 8) {
                (0, 0) => [0, 255, 255, 0],
                (1, 0) => [0, 0, 0, 0],
                (0, 1) => [0, 0, 0, 255],
                _ => [255, 0, 255, 0],
            };
            cmyk.extend_from_slice(&c);
        }
    }
    let img = still(
        &jpeg(32, 16, &cmyk, jpeg_encoder::ColorType::Cmyk, 100),
        "c.jpg",
    );
    assert_near(px(&img, 4, 3), [255, 0, 0, 255], 4, "cmyk red");
    assert_near(px(&img, 20, 3), [255, 255, 255, 255], 4, "cmyk white");
    assert_near(px(&img, 4, 12), [0, 0, 0, 255], 4, "cmyk black");
    assert_near(px(&img, 20, 12), [0, 255, 0, 255], 4, "cmyk green");

    // Probe reads the size from the frame header.
    let meta = reg().probe(&source(&file, "blocks.jpg")).unwrap();
    assert_eq!((meta.width, meta.height, meta.format), (32, 32, "jpeg"));
}

#[test]
fn jpeg_target_size_uses_dct_scaling() {
    let (w, h, rgba) = blocks(&[RED, BLUE, GREEN, GRAY], 2, 32);
    let file = jpeg(w as u16, h as u16, &rgba, jpeg_encoder::ColorType::Rgba, 95);
    let options = DecodeOptions {
        target_size: Some((16, 16)),
        ..DecodeOptions::default()
    };
    let img = decode_with(&file, "s.jpg", &options);
    let img = img.first_image();
    // 64 x 64 scaled by 2/8 is the smallest DCT scale not below 16 x 16.
    assert_eq!((img.width(), img.height()), (16, 16));
    assert_near(px(img, 3, 3), [200, 30, 40, 255], 6, "scaled red");
    assert_near(px(img, 12, 12), [128, 128, 128, 255], 6, "scaled gray");
}

fn encode_png(w: u32, h: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(data).unwrap();
        wr.finish().unwrap();
    }
    out
}

#[test]
fn decode_png() {
    // RGBA8 decodes exactly.
    let data = pattern(13, 7, true);
    let file = encode_png(13, 7, png::ColorType::Rgba, png::BitDepth::Eight, &data);
    let img = still(&file, "a.png");
    assert_eq!(img.format(), PixelFormat::Rgba8);
    assert_eq!(img.data(), &data[..]);

    // 16-bit RGB keeps 16 bits per channel.
    let mut d16 = Vec::new();
    for v in [0x1234u16, 0xABCD, 0xFFFF, 0x0001, 0x8000, 0x7FFF] {
        d16.extend_from_slice(&v.to_be_bytes());
    }
    let file = encode_png(2, 1, png::ColorType::Rgb, png::BitDepth::Sixteen, &d16);
    let img = still(&file, "b.png");
    assert_eq!(img.format(), PixelFormat::Rgba16);
    assert_eq!(px16(&img, 0, 0), [0x1234, 0xABCD, 0xFFFF, 0xFFFF]);
    assert_eq!(px16(&img, 1, 0), [0x0001, 0x8000, 0x7FFF, 0xFFFF]);

    // 2-bit palette with tRNS: index 1 is half transparent.
    let plte = png_chunk(b"PLTE", &[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9]);
    let trns = png_chunk(b"tRNS", &[255, 128]);
    // Row: indices 0 1 2 3 packed MSB first, filter byte 0.
    let raw = [0u8, 0b0001_1011];
    let file = png_file(4, 1, 2, 3, 0, &[plte, trns], &raw);
    let img = still(&file, "p.png");
    assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&img, 1, 0), [0, 255, 0, 128]);
    assert_eq!(px(&img, 2, 0), [0, 0, 255, 255]);
    assert_eq!(px(&img, 3, 0), [9, 9, 9, 255]);

    // 1-bit grayscale and 8-bit gray with alpha.
    let file = png_file(8, 1, 1, 0, 0, &[], &[0, 0b1010_0000]);
    let img = still(&file, "g1.png");
    assert_eq!(px(&img, 0, 0), [255, 255, 255, 255]);
    assert_eq!(px(&img, 1, 0), [0, 0, 0, 255]);
    let file = encode_png(
        2,
        1,
        png::ColorType::GrayscaleAlpha,
        png::BitDepth::Eight,
        &[10, 20, 30, 40],
    );
    let img = still(&file, "ga.png");
    assert_eq!(img.data(), &[10, 10, 10, 20, 30, 30, 30, 40]);

    // Adam7 interlaced RGB reassembles every pass.
    let (w, h) = (11u32, 9u32);
    let rgb: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i * 3) as u8, (i * 7) as u8, (255 - i) as u8])
        .collect();
    let file = png_file(w, h, 8, 2, 1, &[], &adam7_rgb(w, h, &rgb));
    let img = still(&file, "i.png");
    for i in 0..(w * h) as usize {
        let p = &img.data()[i * 4..i * 4 + 4];
        assert_eq!(
            p,
            &[rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], 255],
            "pixel {i}"
        );
    }
}

#[test]
fn decode_apng_animated() {
    let (cw, ch) = (8u32, 8u32);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, cw, ch);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(3, 2).unwrap();
        let mut wr = enc.write_header().unwrap();
        // Frame 0: full red, kept.
        wr.set_frame_delay(1, 10).unwrap();
        wr.set_dispose_op(png::DisposeOp::None).unwrap();
        wr.set_blend_op(png::BlendOp::Source).unwrap();
        wr.write_image_data(&solid(8, 8, [255, 0, 0, 255])).unwrap();
        // Frame 1: half transparent blue blended over the middle, then
        // cleared to transparent.
        wr.set_frame_dimension(4, 4).unwrap();
        wr.set_frame_position(2, 2).unwrap();
        wr.set_frame_delay(20, 100).unwrap();
        wr.set_dispose_op(png::DisposeOp::Background).unwrap();
        wr.set_blend_op(png::BlendOp::Over).unwrap();
        wr.write_image_data(&solid(4, 4, [0, 0, 255, 128])).unwrap();
        // Frame 2: green corner, restored afterwards; delay 3/100 s.
        wr.set_frame_position(0, 0).unwrap();
        wr.set_frame_dimension(2, 2).unwrap();
        wr.set_frame_delay(3, 100).unwrap();
        wr.set_dispose_op(png::DisposeOp::Previous).unwrap();
        wr.set_blend_op(png::BlendOp::Source).unwrap();
        wr.write_image_data(&solid(2, 2, [0, 255, 0, 255])).unwrap();
        wr.finish().unwrap();
    }
    let a = anim(decode(&out, "a.png"));
    assert_eq!(a.loop_count(), LoopCount::Finite(2));
    let delays: Vec<Duration> = a.frames().iter().map(|f| f.delay).collect();
    assert_eq!(
        delays,
        [
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(30)
        ]
    );
    let f = |i: usize| &a.frames()[i].image;
    assert_eq!(px(f(0), 4, 4), [255, 0, 0, 255]);
    assert_near(px(f(1), 4, 4), [127, 0, 128, 255], 1, "blended center");
    assert_eq!(px(f(1), 0, 0), [255, 0, 0, 255]);
    assert_eq!(
        px(f(2), 4, 4),
        [0, 0, 0, 0],
        "frame 1 disposed to background"
    );
    assert_eq!(px(f(2), 1, 1), [0, 255, 0, 255]);
    assert_eq!(px(f(2), 7, 7), [255, 0, 0, 255]);

    // A default image outside the animation is skipped.
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 2, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_animated(2, 0).unwrap();
        enc.set_sep_def_img(true).unwrap();
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&solid(2, 2, [9, 9, 9, 255])).unwrap();
        wr.write_image_data(&solid(2, 2, [1, 2, 3, 255])).unwrap();
        wr.write_image_data(&solid(2, 2, [4, 5, 6, 255])).unwrap();
        wr.finish().unwrap();
    }
    let a = anim(decode(&out, "b.apng"));
    assert_eq!(a.loop_count(), LoopCount::Infinite);
    assert_eq!(a.frames().len(), 2);
    assert_eq!(px(&a.frames()[0].image, 0, 0), [1, 2, 3, 255]);
    assert_eq!(px(&a.frames()[1].image, 1, 1), [4, 5, 6, 255]);

    // 16-bit frames are composed at 8 bits per channel.
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 1, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Sixteen);
        enc.set_animated(2, 0).unwrap();
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&[0xFF, 0xFF, 0x80, 0x00, 0x00, 0x00, 0xFF, 0xFF])
            .unwrap();
        wr.write_image_data(&[0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF])
            .unwrap();
        wr.finish().unwrap();
    }
    let a = anim(decode(&out, "c.png"));
    assert_eq!(a.frames()[0].image.format(), PixelFormat::Rgba8);
    assert_eq!(px(&a.frames()[0].image, 0, 0), [255, 128, 0, 255]);
    assert_eq!(px(&a.frames()[1].image, 0, 0), [0, 0, 255, 255]);
}

/// A GIF frame from palette indices.
#[allow(clippy::too_many_arguments)]
fn gif_frame(
    left: u16,
    top: u16,
    w: u16,
    h: u16,
    indices: Vec<u8>,
    delay: u16,
    dispose: gif::DisposalMethod,
    transparent: Option<u8>,
    palette: Option<Vec<u8>>,
) -> gif::Frame<'static> {
    gif::Frame {
        left,
        top,
        width: w,
        height: h,
        buffer: Cow::Owned(indices),
        delay,
        dispose,
        transparent,
        palette,
        ..gif::Frame::default()
    }
}

/// A 6 x 4 GIF exercising every disposal method, transparency, a local
/// palette and a zero delay.
pub fn sample_gif() -> Vec<u8> {
    let palette = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, 6, 4, &palette).unwrap();
        enc.set_repeat(gif::Repeat::Finite(2)).unwrap();
        use gif::DisposalMethod::*;
        enc.write_frame(&gif_frame(0, 0, 6, 4, vec![1; 24], 10, Keep, None, None))
            .unwrap();
        enc.write_frame(&gif_frame(
            1,
            1,
            2,
            2,
            vec![2, 0, 2, 2],
            20,
            Background,
            Some(0),
            None,
        ))
        .unwrap();
        enc.write_frame(&gif_frame(4, 2, 2, 2, vec![3; 4], 5, Previous, None, None))
            .unwrap();
        let local = vec![9, 8, 7, 250, 250, 250];
        enc.write_frame(&gif_frame(0, 0, 1, 1, vec![1], 0, Keep, None, Some(local)))
            .unwrap();
    }
    out
}

#[test]
fn decode_gif_animated() {
    let file = sample_gif();
    let a = anim(decode(&file, "a.gif"));
    assert_eq!(a.frames().len(), 4);
    assert_eq!(
        a.loop_count(),
        LoopCount::Finite(3),
        "NETSCAPE 2 plays 3 times"
    );
    let delays: Vec<u64> = a
        .frames()
        .iter()
        .map(|f| f.delay.as_millis() as u64)
        .collect();
    assert_eq!(delays, [100, 200, 50, 100], "zero delay becomes 100 ms");
    let f = |i: usize| &a.frames()[i].image;
    let (red, green, blue, clear) = ([255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [0; 4]);
    assert_eq!(px(f(0), 3, 2), red);
    assert_eq!(px(f(1), 1, 1), green);
    assert_eq!(px(f(1), 2, 1), red, "transparent index keeps the canvas");
    assert_eq!(px(f(1), 2, 2), green);
    assert_eq!(px(f(2), 1, 1), clear, "background disposal clears");
    assert_eq!(px(f(2), 5, 3), blue);
    assert_eq!(px(f(2), 0, 3), red);
    assert_eq!(px(f(3), 5, 3), red, "previous disposal restores");
    assert_eq!(px(f(3), 2, 2), clear);
    assert_eq!(px(f(3), 0, 0), [250, 250, 250, 255], "local palette");
    let meta = reg().probe(&source(&file, "a.gif")).unwrap();
    assert_eq!((meta.kind, meta.frame_count), (ContentKind::Animation, 4));

    // A GIF that lacks only its trailer still decodes.
    let img = decode(&file[..file.len() - 1], "t.gif");
    assert_eq!(anim(img).frames().len(), 4);
}

#[test]
fn decode_webp_still() {
    // Lossless with alpha is exact.
    let data = pattern(17, 9, true);
    let file = webp_lossless(17, 9, data.clone());
    let img = still(&file, "a.webp");
    assert_eq!(img.data(), &data[..]);

    // Lossy (VP8) blocks come back close to the source colors.
    let img = still(&fixture("lossy.webp"), "lossy.webp");
    assert_eq!((img.width(), img.height()), (64, 32));
    assert_near(px(&img, 10, 16), [200, 40, 40, 255], 6, "lossy left");
    assert_near(px(&img, 50, 16), [30, 90, 210, 255], 6, "lossy right");

    // Lossy with a lossless alpha plane.
    let img = still(&fixture("lossy_alpha.webp"), "lossy_alpha.webp");
    assert_near(px(&img, 16, 4), [40, 180, 60, 255], 6, "opaque half");
    assert_eq!(px(&img, 16, 28)[3], 128, "alpha plane is exact");
}

#[test]
fn decode_webp_animated() {
    let frames = [
        WebpFrame {
            x: 0,
            y: 0,
            w: 8,
            h: 8,
            ms: 50,
            blend: false,
            dispose: false,
            rgba: solid(8, 8, [255, 0, 0, 255]),
        },
        WebpFrame {
            x: 2,
            y: 2,
            w: 4,
            h: 4,
            ms: 70,
            blend: true,
            dispose: true,
            rgba: solid(4, 4, [0, 0, 255, 128]),
        },
        WebpFrame {
            x: 0,
            y: 0,
            w: 2,
            h: 2,
            ms: 0,
            blend: false,
            dispose: false,
            rgba: solid(2, 2, [0, 255, 0, 255]),
        },
    ];
    let file = webp_animated(8, 8, 3, &frames);
    let a = anim(decode(&file, "a.webp"));
    assert_eq!(a.loop_count(), LoopCount::Finite(3));
    let delays: Vec<u64> = a
        .frames()
        .iter()
        .map(|f| f.delay.as_millis() as u64)
        .collect();
    assert_eq!(delays, [50, 70, 100]);
    let f = |i: usize| &a.frames()[i].image;
    assert_eq!(px(f(0), 4, 4), [255, 0, 0, 255]);
    assert_near(px(f(1), 4, 4), [127, 0, 128, 255], 1, "blended");
    assert_eq!(px(f(1), 1, 1), [255, 0, 0, 255]);
    assert_eq!(px(f(2), 4, 4), [0, 0, 0, 0], "disposed to transparent");
    assert_eq!(px(f(2), 1, 1), [0, 255, 0, 255]);
    assert_eq!(px(f(2), 7, 7), [255, 0, 0, 255]);

    // Lossy frames from an independent encoder.
    let a = anim(decode(&fixture("lossy_anim.webp"), "lossy_anim.webp"));
    assert_eq!(a.frames().len(), 2);
    assert_eq!(a.loop_count(), LoopCount::Infinite);
    assert_eq!(a.frames()[0].delay, Duration::from_millis(80));
    assert_eq!(a.frames()[1].delay, Duration::from_millis(120));
    assert_near(
        px(&a.frames()[0].image, 24, 24),
        [220, 30, 30, 255],
        8,
        "frame 0",
    );
    assert_near(
        px(&a.frames()[1].image, 24, 24),
        [20, 40, 230, 255],
        8,
        "frame 1 corner",
    );
    assert_near(
        px(&a.frames()[1].image, 4, 4),
        [220, 30, 30, 255],
        8,
        "frame 1 rest",
    );
}

#[test]
fn decode_bmp() {
    // The crate's encoder: 24-bit opaque and 32-bit with alpha, exact.
    let data = pattern(7, 5, false);
    let file = encode("bmp", &rgba8(7, 5, data.clone()), &EncodeOptions::default());
    assert_eq!(still(&file, "a.bmp").data(), &data[..]);
    let data = pattern(6, 3, true);
    let file = encode("bmp", &rgba8(6, 3, data.clone()), &EncodeOptions::default());
    assert_eq!(still(&file, "b.bmp").data(), &data[..]);

    // 8-bit palette, bottom-up, width 3 (rows padded to 4 bytes).
    let pal = bmp_palette(&[[10, 20, 30], [200, 100, 50]]);
    let pixels = [1, 0, 1, 0, /* bottom row */ 0, 0, 1, 0];
    let file = bmp_file(&bmp_info_header(3, 2, 8, 0, 2), &pal, &pixels);
    let img = still(&file, "p.bmp");
    assert_eq!(px(&img, 0, 0), [10, 20, 30, 255], "top row is stored last");
    assert_eq!(px(&img, 2, 0), [200, 100, 50, 255]);
    assert_eq!(px(&img, 0, 1), [200, 100, 50, 255]);

    // 1-bit, top-down.
    let pal = bmp_palette(&[[0, 0, 0], [255, 255, 255]]);
    let file = bmp_file(
        &bmp_info_header(3, -1, 1, 0, 0),
        &pal,
        &[0b1010_0000, 0, 0, 0],
    );
    let img = still(&file, "m.bmp");
    assert_eq!(
        [px(&img, 0, 0)[0], px(&img, 1, 0)[0], px(&img, 2, 0)[0]],
        [255, 0, 255]
    );

    // 16-bit 5-6-5 bit fields, top-down.
    let masks: Vec<u8> = [0xF800u32, 0x07E0, 0x001F]
        .iter()
        .flat_map(|m| m.to_le_bytes())
        .collect();
    let pixels = [0x00, 0xF8, 0xE0, 0x07, 0x1F, 0x00, 0x00, 0x00];
    let file = bmp_file(&bmp_info_header(3, -1, 16, 3, 0), &masks, &pixels);
    let img = still(&file, "f.bmp");
    assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]);
    assert_eq!(px(&img, 2, 0), [0, 0, 255, 255]);

    // RLE8 with a delta: skipped pixels stay transparent.
    let pal = bmp_palette(&[[0, 0, 0], [250, 0, 0], [0, 0, 250]]);
    let rle = [
        3, 1, 0, 0, // bottom row: three red, end of line
        0, 2, 1, 0, // delta: one right, same row (row 1)
        0, 3, 2, 1, 2, 0, // absolute run of 3 (padded)
        0, 1, // end of bitmap
    ];
    let file = bmp_file(&bmp_info_header(4, 2, 8, 1, 3), &pal, &rle);
    let img = still(&file, "r.bmp");
    assert_eq!(px(&img, 0, 1), [250, 0, 0, 255]);
    assert_eq!(px(&img, 3, 1), [0, 0, 0, 0], "not covered by the run");
    assert_eq!(px(&img, 0, 0), [0, 0, 0, 0], "skipped by the delta");
    assert_eq!(px(&img, 1, 0), [0, 0, 250, 255]);
    assert_eq!(px(&img, 2, 0), [250, 0, 0, 255]);
    assert_eq!(px(&img, 3, 0), [0, 0, 250, 255]);

    // RLE4: an encoded run alternates nibbles.
    let pal = bmp_palette(&[[0, 0, 0], [255, 255, 0], [0, 255, 255]]);
    let rle = [4, 0x12, 0, 1];
    let file = bmp_file(&bmp_info_header(4, 1, 4, 2, 3), &pal, &rle);
    let img = still(&file, "r4.bmp");
    assert_eq!(px(&img, 0, 0), [255, 255, 0, 255]);
    assert_eq!(px(&img, 1, 0), [0, 255, 255, 255]);
    assert_eq!(px(&img, 2, 0), [255, 255, 0, 255]);

    // OS/2 1.x core header, 24-bit.
    let mut core = Vec::new();
    core.extend_from_slice(&12u32.to_le_bytes());
    core.extend_from_slice(&2u16.to_le_bytes());
    core.extend_from_slice(&1u16.to_le_bytes());
    core.extend_from_slice(&1u16.to_le_bytes());
    core.extend_from_slice(&24u16.to_le_bytes());
    let file = bmp_file(&core, &[], &[1, 2, 3, 4, 5, 6, 0, 0]);
    let img = still(&file, "o.bmp");
    assert_eq!(img.data(), &[3, 2, 1, 255, 6, 5, 4, 255]);

    // 32-bit BI_RGB whose fourth byte is zero everywhere is opaque.
    let file = bmp_file(&bmp_info_header(1, 1, 32, 0, 0), &[], &[7, 8, 9, 0]);
    assert_eq!(still(&file, "x.bmp").data(), &[9, 8, 7, 255]);
}

#[test]
fn exif_orientation_applied() {
    // JPEG: left half red, right half blue, EXIF orientation 6 (rotate 90
    // degrees clockwise). Upright, the red half is on top.
    let (w, h, rgba) = blocks(&[RED, BLUE], 2, 16);
    let mut file = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut file, 100);
    enc.add_exif_metadata(&exif_orientation(6)).unwrap();
    enc.encode(&rgba, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)
        .unwrap();
    let meta = reg().probe(&source(&file, "o.jpg")).unwrap();
    assert_eq!((meta.width, meta.height), (16, 32));
    assert_eq!(meta.orientation, Orientation::Rotate90);
    let img = still(&file, "o.jpg");
    assert_eq!((img.width(), img.height()), (16, 32));
    assert_near(px(&img, 8, 6), [200, 30, 40, 255], 4, "red on top");
    assert_near(px(&img, 8, 26), [20, 60, 220, 255], 4, "blue below");

    // PNG with an eXIf chunk, orientation 3 (rotate 180).
    let raw = [0u8, 1, 2, 3, 4, 5, 6];
    let exif = png_chunk(b"eXIf", &exif_orientation(3));
    let file = png_file(2, 1, 8, 2, 0, &[exif], &raw);
    let img = still(&file, "o.png");
    assert_eq!(img.data(), &[4, 5, 6, 255, 1, 2, 3, 255]);

    // WebP with an EXIF chunk, orientation 8 (rotate 270): the right
    // column becomes the top row.
    let mut data = Vec::new();
    for c in [[1u8, 0, 0, 255], [2, 0, 0, 255]] {
        data.extend_from_slice(&c);
    }
    let lossless = webp_lossless(2, 1, data);
    let mut vp8x = vec![0x08, 0, 0, 0];
    vp8x.extend_from_slice(&[1, 0, 0, 0, 0, 0]);
    let mut body = riff_chunk(b"VP8X", &vp8x);
    body.extend(vp8l_chunk(&lossless));
    body.extend(riff_chunk(b"EXIF", &exif_orientation(8)));
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    file.extend_from_slice(b"WEBP");
    file.extend(body);
    let img = still(&file, "o.webp");
    assert_eq!((img.width(), img.height()), (1, 2));
    assert_eq!(px(&img, 0, 0)[0], 2);
    assert_eq!(px(&img, 0, 1)[0], 1);
}

#[test]
fn first_frame_only_returns_still() {
    let options = DecodeOptions {
        first_frame_only: true,
        ..DecodeOptions::default()
    };
    let gif = decode_with(&sample_gif(), "a.gif", &options);
    match gif {
        Content::Still(img) => assert_eq!(px(&img, 3, 2), [255, 0, 0, 255]),
        other => panic!("gif: {:?}", other.kind()),
    }

    let frames = [
        WebpFrame {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
            ms: 50,
            blend: false,
            dispose: false,
            rgba: solid(4, 4, [1, 2, 3, 255]),
        },
        WebpFrame {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
            ms: 50,
            blend: false,
            dispose: false,
            rgba: solid(4, 4, [9, 9, 9, 255]),
        },
    ];
    match decode_with(&webp_animated(4, 4, 0, &frames), "a.webp", &options) {
        Content::Still(img) => assert_eq!(px(&img, 2, 2), [1, 2, 3, 255]),
        other => panic!("webp: {:?}", other.kind()),
    }

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 3, 3);
        enc.set_color(png::ColorType::Rgba);
        enc.set_animated(2, 0).unwrap();
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&solid(3, 3, [5, 6, 7, 255])).unwrap();
        wr.write_image_data(&solid(3, 3, [8, 8, 8, 255])).unwrap();
        wr.finish().unwrap();
    }
    assert!(matches!(decode(&out, "a.png"), Content::Animation(_)));
    match decode_with(&out, "a.png", &options) {
        Content::Still(img) => assert_eq!(px(&img, 1, 1), [5, 6, 7, 255]),
        other => panic!("apng: {:?}", other.kind()),
    }
}

/// A smooth test picture: gradients plus a few soft features.
fn smooth(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let wave = ((fx * 9.0).sin() * (fy * 7.0).cos() * 0.5 + 0.5) * 255.0;
            rgb.extend_from_slice(&[(fx * 255.0) as u8, wave as u8, (fy * 255.0) as u8]);
        }
    }
    rgb
}

/// Average `img` over `f x f` blocks (an exact box downscale).
fn box_down(img: &pandite_core::ImageBuf, f: u32) -> Vec<[f64; 3]> {
    let (w, h) = (img.width() / f, img.height() / f);
    let mut out = Vec::with_capacity((w * h) as usize);
    for by in 0..h {
        for bx in 0..w {
            let mut s = [0f64; 3];
            for y in by * f..(by + 1) * f {
                for x in bx * f..(bx + 1) * f {
                    let p = px(img, x, y);
                    for c in 0..3 {
                        s[c] += f64::from(p[c]);
                    }
                }
            }
            out.push(s.map(|v| v / f64::from(f * f)));
        }
    }
    out
}

/// Mean and largest per-channel difference between a reduced decode and
/// the box-downscaled full decode.
fn reduced_error(reduced: &pandite_core::ImageBuf, reference: &[[f64; 3]]) -> (f64, f64) {
    let (mut sum, mut max) = (0f64, 0f64);
    let w = reduced.width();
    for (i, r) in reference.iter().enumerate() {
        let p = px(reduced, i as u32 % w, i as u32 / w);
        for c in 0..3 {
            let d = (f64::from(p[c]) - r[c]).abs();
            sum += d;
            max = max.max(d);
        }
    }
    (sum / (reference.len() * 3) as f64, max)
}

/// MPEntry attribute (CIPA DC-007) of the primary image: representative
/// image flag, JPEG data, MP type Baseline MP Primary Image.
const MP_PRIMARY: u32 = 0x2003_0000;
/// MPEntry attribute of a Large Thumbnail (full HD equivalent): the
/// preview cameras append after the main image.
const MP_LARGE_THUMBNAIL: u32 = 0x0001_0002;
/// MPEntry attribute with MP type Undefined: the type phones write for
/// the HDR gain map they append (Ultra HDR, Apple HDR).
const MP_UNDEFINED: u32 = 0;

/// A JPEG with a Multi-Picture Format index (CIPA DC-007) in APP2 that
/// points to `preview`, a Large Thumbnail appended after the main image.
fn with_mpf_preview(main: &[u8], preview: &[u8]) -> Vec<u8> {
    with_mpf(main, &[(MP_LARGE_THUMBNAIL, preview)])
}

/// A JPEG with a Multi-Picture Format index in APP2: the main image, then
/// each `(attribute, stream)` appended in order.
fn with_mpf(main: &[u8], images: &[(u32, &[u8])]) -> Vec<u8> {
    let entries = 1 + images.len() as u32;
    // TIFF header (8), IFD count (2), two entries (24), next IFD (4).
    let ifd_len = 8 + 2 + 24 + 4;
    let mp_len = ifd_len + 16 * entries as usize;
    let app2_len = 2 + 4 + mp_len;
    let mp_header_at = 2 + 2 + 2 + 4;
    let total_main = main.len() + 2 + app2_len;
    let mut seg = Vec::new();
    seg.extend_from_slice(&[0xFF, 0xE2]);
    seg.extend_from_slice(&(app2_len as u16).to_be_bytes());
    seg.extend_from_slice(b"MPF\0");
    seg.extend_from_slice(b"II*\0");
    seg.extend_from_slice(&8u32.to_le_bytes());
    seg.extend_from_slice(&2u16.to_le_bytes());
    // MPFVersion, UNDEFINED x 4, "0100".
    seg.extend_from_slice(&0xB000u16.to_le_bytes());
    seg.extend_from_slice(&7u16.to_le_bytes());
    seg.extend_from_slice(&4u32.to_le_bytes());
    seg.extend_from_slice(b"0100");
    // MPEntry, UNDEFINED x 32, stored after the IFD.
    seg.extend_from_slice(&0xB002u16.to_le_bytes());
    seg.extend_from_slice(&7u16.to_le_bytes());
    seg.extend_from_slice(&(16 * entries).to_le_bytes());
    seg.extend_from_slice(&(ifd_len as u32).to_le_bytes());
    seg.extend_from_slice(&0u32.to_le_bytes());
    // Entry 0: the primary image (offset 0). Then the appended images,
    // with offsets from the MP header.
    let mut list = vec![(MP_PRIMARY, total_main as u32, 0u32)];
    let mut at = total_main;
    for (attribute, stream) in images {
        list.push((*attribute, stream.len() as u32, (at - mp_header_at) as u32));
        at += stream.len();
    }
    for (attribute, size, offset) in list {
        seg.extend_from_slice(&attribute.to_le_bytes());
        seg.extend_from_slice(&size.to_le_bytes());
        seg.extend_from_slice(&offset.to_le_bytes());
        seg.extend_from_slice(&[0; 4]);
    }
    let mut out = main[..2].to_vec();
    out.extend_from_slice(&seg);
    out.extend_from_slice(&main[2..]);
    assert_eq!(out.len(), total_main);
    for (_, stream) in images {
        out.extend_from_slice(stream);
    }
    out
}

/// Mean of |R - G| + |G - B| per pixel: 0 for a gray picture.
fn colorfulness(img: &pandite_core::ImageBuf) -> f64 {
    let sum: u64 = img
        .data()
        .chunks_exact(4)
        .map(|p| u64::from(p[0].abs_diff(p[1])) + u64::from(p[1].abs_diff(p[2])))
        .sum();
    sum as f64 / f64::from(img.width() * img.height())
}

/// Every `f`-th pixel of every `f`-th row of `rgb`, as one gray channel
/// (Rec. 601 luma): the HDR gain map phones append to their JPEGs.
fn gray_subsample(rgb: &[u8], w: u32, h: u32, f: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(((w / f) * (h / f)) as usize);
    for y in 0..h / f {
        for x in 0..w / f {
            let i = ((y * f * w + x * f) * 3) as usize;
            let [r, g, b] = [rgb[i], rgb[i + 1], rgb[i + 2]].map(u32::from);
            out.push(((299 * r + 587 * g + 114 * b + 500) / 1000) as u8);
        }
    }
    out
}

/// The defect "JPEG files first appear in grayscale, then in color": a
/// phone JPEG carries its HDR gain map as a second Multi-Picture Format
/// image (one gray channel, a quarter of the size, MP type Undefined). The
/// reduced decode, which the viewer shows first, took it for an embedded
/// preview. Only Large Thumbnails are previews, and a gray stream never
/// stands in for a color image, whatever its MP type says.
#[test]
fn jpeg_reduced_decode_never_shows_a_grayscale_gain_map() {
    let (w, h) = (512u32, 384u32);
    let rgb = smooth(w, h);
    let main = jpeg(w as u16, h as u16, &rgb, jpeg_encoder::ColorType::Rgb, 95);
    let full = still(&main, "full.jpg");
    let full_color = colorfulness(&full);
    assert!(
        full_color > 50.0,
        "the test picture is colorful: {full_color}"
    );
    let gain_map = jpeg(
        (w / 4) as u16,
        (h / 4) as u16,
        &gray_subsample(&rgb, w, h, 4),
        jpeg_encoder::ColorType::Luma,
        90,
    );
    let half: Vec<u8> = (0..h / 2)
        .flat_map(|y| (0..w / 2).map(move |x| ((y * 2 * w + x * 2) * 3) as usize))
        .flat_map(|i| [rgb[i], rgb[i + 1], rgb[i + 2]])
        .collect();
    let preview = jpeg(
        (w / 2) as u16,
        (h / 2) as u16,
        &half,
        jpeg_encoder::ColorType::Rgb,
        95,
    );
    // The gain map covers the target: it was the smallest fitting image.
    let options = DecodeOptions {
        target_size: Some((100, 75)),
        ..DecodeOptions::default()
    };
    for (what, file, dims, factor) in [
        // A phone JPEG: DCT scaling of the main image to 1/4.
        (
            "gain map of type Undefined",
            with_mpf(&main, &[(MP_UNDEFINED, &gain_map)]),
            (w / 4, h / 4),
            4,
        ),
        // A color image of another MP type is another picture (such as a
        // three-channel gain map): not a preview either.
        (
            "color image of type Undefined",
            with_mpf(&main, &[(MP_UNDEFINED, &preview)]),
            (w / 4, h / 4),
            4,
        ),
        // A gray stream labeled as a preview is not one either.
        (
            "gray Large Thumbnail",
            with_mpf(&main, &[(MP_LARGE_THUMBNAIL, &gain_map)]),
            (w / 4, h / 4),
            4,
        ),
        // A camera preview beside a gain map: the preview stands in.
        (
            "color preview and gain map",
            with_mpf(
                &main,
                &[(MP_LARGE_THUMBNAIL, &preview), (MP_UNDEFINED, &gain_map)],
            ),
            (w / 2, h / 2),
            2,
        ),
    ] {
        let reduced = decode_with(&file, "phone.jpg", &options);
        let reduced = reduced.first_image();
        let color = colorfulness(reduced);
        assert!(
            color >= full_color * 0.9,
            "{what}: the first stage is gray (colorfulness {color:.1}, full image {full_color:.1})"
        );
        assert_eq!((reduced.width(), reduced.height()), dims, "{what}");
        let (mean, max) = reduced_error(reduced, &box_down(&full, factor));
        assert!(
            mean <= 6.0 && max <= 60.0,
            "{what}: mean {mean:.2} max {max:.1}"
        );
    }
    // A gray image keeps its gray preview.
    let gray_main = jpeg(
        w as u16,
        h as u16,
        &gray_subsample(&rgb, w, h, 1),
        jpeg_encoder::ColorType::Luma,
        95,
    );
    let gray_preview = jpeg(
        (w / 2) as u16,
        (h / 2) as u16,
        &gray_subsample(&rgb, w, h, 2),
        jpeg_encoder::ColorType::Luma,
        95,
    );
    let file = with_mpf(&gray_main, &[(MP_LARGE_THUMBNAIL, &gray_preview)]);
    let reduced = decode_with(&file, "gray.jpg", &options);
    assert_eq!(reduced.dimensions(), (w / 2, h / 2), "the gray preview");
}

/// Gate G3: the reduced JPEG decode matches the full decode scaled down.
/// A target size decodes with DCT scaling to the strongest power-of-two
/// reduction whose result still reaches the target on one axis (the
/// decoder contract: never smaller on both); an embedded Multi-Picture Format
/// preview that covers the target is used instead (and one with another
/// aspect ratio is not). Both match the full decode averaged over the
/// same pixel blocks within the error of the 8 x 8 DCT approximation.
#[test]
fn jpeg_reduced_decode_matches_full_downscale() {
    let (w, h) = (512u32, 384u32);
    let rgb = smooth(w, h);
    let main = jpeg(w as u16, h as u16, &rgb, jpeg_encoder::ColorType::Rgb, 95);
    let full = still(&main, "full.jpg");
    assert_eq!((full.width(), full.height()), (w, h));
    for (target, factor) in [
        ((256, 192), 2),
        ((128, 96), 4),
        ((64, 48), 8),
        ((100, 10), 8),
    ] {
        let options = DecodeOptions {
            target_size: Some(target),
            ..DecodeOptions::default()
        };
        let reduced = decode_with(&main, "r.jpg", &options);
        let reduced = reduced.first_image();
        assert_eq!(
            (reduced.width(), reduced.height()),
            (w / factor, h / factor),
            "target {target:?}"
        );
        let (mean, max) = reduced_error(reduced, &box_down(&full, factor));
        assert!(
            mean <= 2.5 && max <= 24.0,
            "DCT scale 1/{factor}: mean {mean:.2} max {max:.1}"
        );
    }

    // An embedded preview at half size, same picture.
    let (pw, ph) = (w / 2, h / 2);
    let mut small = Vec::with_capacity((pw * ph * 3) as usize);
    for y in 0..ph {
        for x in 0..pw {
            let i = ((y * 2 * w + x * 2) * 3) as usize;
            small.extend_from_slice(&rgb[i..i + 3]);
        }
    }
    let preview = jpeg(
        pw as u16,
        ph as u16,
        &small,
        jpeg_encoder::ColorType::Rgb,
        95,
    );
    let file = with_mpf_preview(&main, &preview);
    // The main image decodes as before.
    let whole = still(&file, "mpf.jpg");
    assert_eq!((whole.width(), whole.height()), (w, h));
    let options = DecodeOptions {
        target_size: Some((200, 150)),
        ..DecodeOptions::default()
    };
    let reduced = decode_with(&file, "mpf.jpg", &options);
    let reduced = reduced.first_image();
    assert_eq!((reduced.width(), reduced.height()), (pw, ph), "the preview");
    let (mean, max) = reduced_error(reduced, &box_down(&full, 2));
    assert!(
        mean <= 6.0 && max <= 60.0,
        "preview: mean {mean:.2} max {max:.1}"
    );
    // A target larger than the preview is decoded from the main image.
    let options = DecodeOptions {
        target_size: Some((400, 300)),
        ..DecodeOptions::default()
    };
    let big = decode_with(&file, "mpf.jpg", &options);
    assert_eq!(big.dimensions(), (w, h));
    // A preview of another aspect ratio stands in for nothing.
    let odd = jpeg(200, 50, &smooth(200, 50), jpeg_encoder::ColorType::Rgb, 90);
    let file = with_mpf_preview(&main, &odd);
    let options = DecodeOptions {
        target_size: Some((100, 25)),
        ..DecodeOptions::default()
    };
    let reduced = decode_with(&file, "odd.jpg", &options);
    assert_eq!(
        reduced.dimensions(),
        (w / 8, h / 8),
        "DCT scaling, not the odd preview"
    );
}
