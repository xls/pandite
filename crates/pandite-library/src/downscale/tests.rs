use super::*;

fn rgba8(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> ImageBuf {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&f(x, y));
        }
    }
    ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

#[test]
fn fit_size_keeps_aspect_and_never_enlarges() {
    assert_eq!(fit_size(4000, 3000, 256), (256, 192));
    assert_eq!(fit_size(3000, 4000, 256), (192, 256));
    assert_eq!(fit_size(100, 50, 256), (100, 50));
    assert_eq!(fit_size(10000, 1, 256), (256, 1));
    assert_eq!(fit_size(5, 5, 0), (1, 1));
}

#[test]
fn box_filter_averages_exact_areas() {
    // 2x1 blocks of black and white become mid gray.
    let img = rgba8(4, 2, |x, _| {
        if x % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let out = box_downscale(&img, 2);
    assert_eq!((out.width(), out.height()), (2, 1));
    for px in out.data().chunks_exact(4) {
        assert_eq!(px, &[128, 128, 128, 255]);
    }
    // Non-integer ratio 3 -> 2: weights 2/3 and 1/3 split the middle pixel.
    let img = rgba8(3, 1, |x, _| {
        [[0, 0, 0, 255], [90, 90, 90, 255], [180, 180, 180, 255]][x as usize]
    });
    let out = box_downscale(&img, 2);
    assert_eq!((out.width(), out.height()), (2, 1));
    assert_eq!(&out.data()[0..4], &[30, 30, 30, 255]);
    assert_eq!(&out.data()[4..8], &[150, 150, 150, 255]);
}

#[test]
fn box_filter_weights_color_by_alpha() {
    // A transparent black pixel next to opaque red stays red, half alpha.
    let img = rgba8(2, 1, |x, _| {
        if x == 0 {
            [0, 0, 0, 0]
        } else {
            [255, 0, 0, 255]
        }
    });
    let out = box_downscale(&img, 1);
    assert_eq!(out.data(), &[255, 0, 0, 128]);
}

#[test]
fn box_filter_handles_16_bit_and_float() {
    let mut data = Vec::new();
    for v in [0u16, 65535, 65535, 65535, 65535, 65535, 65535, 65535] {
        data.extend_from_slice(&v.to_ne_bytes());
    }
    let img = ImageBuf::new(2, 1, PixelFormat::Rgba16, data).unwrap();
    let out = box_downscale(&img, 1);
    assert_eq!(out.format(), PixelFormat::Rgba16);
    let r = u16::from_ne_bytes([out.data()[0], out.data()[1]]);
    assert_eq!(r, 32768);
    let mut data = Vec::new();
    for v in [4.0f32, 0.0, 0.0, 1.0, 0.0, f32::NAN, 0.0, 1.0] {
        data.extend_from_slice(&v.to_ne_bytes());
    }
    let img = ImageBuf::new(2, 1, PixelFormat::RgbaF32, data).unwrap();
    let out = box_downscale(&img, 1);
    let px: Vec<f32> = out
        .data()
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    assert_eq!(px, vec![2.0, 0.0, 0.0, 1.0]);
}

#[test]
fn small_images_are_returned_as_is() {
    let img = rgba8(3, 2, |_, _| [1, 2, 3, 4]);
    let out = box_downscale(&img, 8);
    assert_eq!(out.data(), img.data());
    assert_eq!((out.width(), out.height()), (3, 2));
}
