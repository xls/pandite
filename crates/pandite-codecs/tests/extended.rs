//! Gate G2: extended formats decode to the expected pixels. Every test is
//! compiled only with its format's feature, so reduced builds still pass.

mod common;

#[allow(unused_imports)]
use common::*;
#[allow(unused_imports)]
use pandite_core::{Content, DecodeOptions, Orientation, PixelFormat};

#[cfg(feature = "tiff")]
#[test]
fn decode_tiff() {
    use std::io::Cursor;
    use tiff::encoder::{colortype, Compression, TiffEncoder};
    use tiff::tags::Tag;

    // RGB8 with LZW and an orientation tag (rotate 180).
    let data: Vec<u8> = (0..6u8).flat_map(|i| [i * 40, 255 - i * 40, i]).collect();
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut buf)
            .unwrap()
            .with_compression(Compression::Lzw);
        let mut img = enc.new_image::<colortype::RGB8>(3, 2).unwrap();
        img.encoder().write_tag(Tag::Orientation, 3u16).unwrap();
        img.write_data(&data).unwrap();
    }
    let file = buf.into_inner();
    let out = still(&file, "a.tif");
    assert_eq!((out.width(), out.height()), (3, 2));
    // Rotated 180 degrees: the last stored pixel comes first.
    assert_eq!(px(&out, 0, 0), [200, 55, 5, 255]);
    assert_eq!(px(&out, 2, 1), [0, 255, 0, 255]);

    // RGBA16 with Deflate keeps 16 bits.
    let d16: Vec<u16> = vec![1, 2, 3, 4, 60000, 50000, 40000, 30000];
    let mut buf = Cursor::new(Vec::new());
    TiffEncoder::new(&mut buf)
        .unwrap()
        .with_compression(Compression::Deflate(Default::default()))
        .write_image::<colortype::RGBA16>(2, 1, &d16)
        .unwrap();
    let out = still(&buf.into_inner(), "b.tiff");
    assert_eq!(out.format(), PixelFormat::Rgba16);
    assert_eq!(px16(&out, 0, 0), [1, 2, 3, 4]);
    assert_eq!(px16(&out, 1, 0), [60000, 50000, 40000, 30000]);

    // Gray8 with PackBits, and 32-bit float RGB as linear float.
    let mut buf = Cursor::new(Vec::new());
    TiffEncoder::new(&mut buf)
        .unwrap()
        .with_compression(Compression::Packbits)
        .write_image::<colortype::Gray8>(4, 1, &[0, 85, 170, 255])
        .unwrap();
    let out = still(&buf.into_inner(), "g.tif");
    assert_eq!(px(&out, 1, 0), [85, 85, 85, 255]);
    let mut buf = Cursor::new(Vec::new());
    TiffEncoder::new(&mut buf)
        .unwrap()
        .write_image::<colortype::RGB32Float>(1, 1, &[2.5, 0.25, 0.0])
        .unwrap();
    let out = still(&buf.into_inner(), "f.tif");
    assert_eq!(pxf(&out, 0, 0), [2.5, 0.25, 0.0, 1.0]);

    // CMYK8: cyan ink only.
    let mut buf = Cursor::new(Vec::new());
    TiffEncoder::new(&mut buf)
        .unwrap()
        .write_image::<colortype::CMYK8>(1, 1, &[255, 0, 0, 0])
        .unwrap();
    assert_eq!(
        px(&still(&buf.into_inner(), "c.tif"), 0, 0),
        [0, 255, 255, 255]
    );

    // JPEG-compressed YCbCr TIFF written by libtiff.
    let out = still(&fixture("jpeg.tif"), "jpeg.tif");
    assert_near(px(&out, 10, 16), [230, 30, 30, 255], 6, "tiff jpeg red");
    assert_near(px(&out, 50, 16), [30, 40, 220, 255], 6, "tiff jpeg blue");

    // A hand-built 4-bit palette image (photometric 3), which the tiff crate
    // itself rejects.
    for compression in [1, 5] {
        let file = palette_tiff_with(compression);
        let out = still(&file, "p.tif");
        assert_eq!(
            px(&out, 0, 0),
            [255, 0, 0, 255],
            "compression {compression}"
        );
        assert_eq!(
            px(&out, 1, 0),
            [0, 0, 255, 255],
            "compression {compression}"
        );
        assert_eq!(
            px(&out, 2, 0),
            [255, 0, 0, 255],
            "compression {compression}"
        );
    }
}

/// A classic little-endian TIFF: 3 x 1, 4-bit palette, one strip, with
/// compression 1 (none) or 5 (LZW).
#[cfg(feature = "tiff")]
fn palette_tiff_with(compression: u16) -> Vec<u8> {
    let indices = [0x12u8, 0x10];
    let strip: Vec<u8> = if compression == 5 {
        weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .encode(&indices)
            .unwrap()
    } else {
        indices.to_vec()
    };
    let mut f = b"II*\0".to_vec();
    f.extend_from_slice(&8u32.to_le_bytes());
    let entries = 9u16;
    let ifd_len = 2 + 12 * entries as usize + 4;
    let map_at = 8 + ifd_len;
    let strip_at = map_at + 3 * 16 * 2;
    let mut ifd = entries.to_le_bytes().to_vec();
    let mut entry = |tag: u16, typ: u16, count: u32, value: u32| {
        ifd.extend_from_slice(&tag.to_le_bytes());
        ifd.extend_from_slice(&typ.to_le_bytes());
        ifd.extend_from_slice(&count.to_le_bytes());
        ifd.extend_from_slice(&value.to_le_bytes());
    };
    entry(256, 3, 1, 3);
    entry(257, 3, 1, 1);
    entry(258, 3, 1, 4);
    entry(259, 3, 1, u32::from(compression));
    entry(262, 3, 1, 3);
    entry(273, 4, 1, strip_at as u32);
    entry(278, 3, 1, 1);
    entry(279, 4, 1, strip.len() as u32);
    entry(320, 3, 48, map_at as u32);
    ifd.extend_from_slice(&0u32.to_le_bytes());
    f.extend(ifd);
    // Color map: 16 reds, 16 greens, 16 blues. Index 1 red, 2 blue.
    let mut map = [0u16; 48];
    map[1] = 0xFFFF;
    map[32 + 2] = 0xFFFF;
    for v in map {
        f.extend_from_slice(&v.to_le_bytes());
    }
    // Indices 1, 2, 1 packed high nibble first.
    f.extend_from_slice(&strip);
    f
}

#[cfg(feature = "tga")]
#[test]
fn decode_tga() {
    fn tga(
        image_type: u8,
        bits: u8,
        desc: u8,
        w: u16,
        h: u16,
        map: Option<(u16, u8, &[u8])>,
        body: &[u8],
    ) -> Vec<u8> {
        let mut v = vec![0u8, u8::from(map.is_some()), image_type];
        let (first, entry_bits, map_bytes) = map.unwrap_or((0, 0, &[]));
        let entries = if entry_bits > 0 {
            map_bytes.len() / usize::from(entry_bits / 8)
        } else {
            0
        };
        v.extend_from_slice(&first.to_le_bytes());
        v.extend_from_slice(&(entries as u16).to_le_bytes());
        v.push(entry_bits);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.push(bits);
        v.push(desc);
        v.extend_from_slice(map_bytes);
        v.extend_from_slice(body);
        v
    }
    // Uncompressed 24-bit, bottom-up: the first stored row is the bottom row.
    let body = [1, 2, 3, 4, 5, 6, /* top row */ 7, 8, 9, 10, 11, 12];
    let img = still(&tga(2, 24, 0, 2, 2, None, &body), "a.tga");
    assert_eq!(px(&img, 0, 0), [9, 8, 7, 255]);
    assert_eq!(px(&img, 1, 1), [6, 5, 4, 255]);

    // RLE 32-bit top-down with alpha; the run crosses into the second row.
    let body = [0x82, 10, 20, 30, 128, /* raw */ 0x00, 1, 2, 3, 255];
    let img = still(&tga(10, 32, 0x28, 2, 2, None, &body), "b.tga");
    assert_eq!(px(&img, 0, 0), [30, 20, 10, 128]);
    assert_eq!(
        px(&img, 0, 1),
        [30, 20, 10, 128],
        "run continues on the next row"
    );
    assert_eq!(px(&img, 1, 1), [3, 2, 1, 255]);

    // Color-mapped 8-bit with a map starting at index 5, right-to-left.
    let map = [0u8, 0, 255, 0, 255, 0];
    let img = still(
        &tga(1, 8, 0x30, 2, 1, Some((5, 24, &map)), &[5, 6]),
        "c.tga",
    );
    assert_eq!(px(&img, 0, 0), [0, 255, 0, 255]);
    assert_eq!(px(&img, 1, 0), [255, 0, 0, 255]);

    // RLE grayscale and 16-bit 5-5-5 with the attribute bit as alpha.
    let img = still(&tga(11, 8, 0x20, 3, 1, None, &[0x82, 77]), "d.tga");
    assert_eq!(px(&img, 2, 0), [77, 77, 77, 255]);
    let px16le = |v: u16| v.to_le_bytes();
    let mut body = Vec::new();
    body.extend_from_slice(&px16le(0x8000 | (31 << 10)));
    body.extend_from_slice(&px16le(31));
    let img = still(&tga(2, 16, 0x21, 2, 1, None, &body), "e.tga");
    assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
    assert_eq!(
        px(&img, 1, 0),
        [0, 0, 255, 0],
        "attribute bit clear is transparent"
    );
}

#[cfg(feature = "ico")]
#[test]
fn decode_ico() {
    use common::hand::*;
    use pandite_core::EncodeOptions;

    fn ico(kind: u16, images: &[(u8, u8, u16, Vec<u8>)]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(&(images.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * images.len();
        for (w, h, bits, data) in images {
            v.extend_from_slice(&[*w, *h, 0, 0]);
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&bits.to_le_bytes());
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, _, _, data) in images {
            v.extend_from_slice(data);
        }
        v
    }

    // A 2 x 2 24-bit DIB with an AND mask hiding the top-left pixel.
    let mut dib = bmp_info_header(2, 4, 24, 0, 0);
    // Color rows (bottom-up, padded to 8 bytes): bottom then top.
    dib.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
    dib.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]);
    // AND mask rows (bottom-up, 4 bytes each): top row has bit 0x80 set.
    dib.extend_from_slice(&[0, 0, 0, 0]);
    dib.extend_from_slice(&[0x80, 0, 0, 0]);
    // A 32 x 32 PNG entry, larger, so it is chosen.
    let big = pattern(32, 32, true);
    let png = encode(
        "png",
        &rgba8(32, 32, big.clone()),
        &EncodeOptions::default(),
    );
    let file = ico(1, &[(2, 2, 24, dib.clone()), (32, 32, 32, png)]);
    let img = still(&file, "a.ico");
    assert_eq!((img.width(), img.height()), (32, 32));
    assert_eq!(img.data(), &big[..]);

    // The DIB alone (as a cursor) uses its mask.
    let file = ico(2, &[(2, 2, 24, dib)]);
    let img = still(&file, "a.cur");
    assert_eq!(px(&img, 0, 0)[3], 0, "masked pixel is transparent");
    assert_eq!(px(&img, 1, 0), [255, 255, 255, 255]);
    assert_eq!(px(&img, 0, 1), [255, 0, 0, 255]);
    assert_eq!(px(&img, 1, 1), [0, 255, 0, 255]);

    // 32-bit DIB with real alpha ignores the mask.
    let mut dib = bmp_info_header(1, 2, 32, 0, 0);
    dib.extend_from_slice(&[10, 20, 30, 40]);
    dib.extend_from_slice(&[0x80, 0, 0, 0]);
    let img = still(&ico(1, &[(1, 1, 32, dib)]), "b.ico");
    assert_eq!(px(&img, 0, 0), [30, 20, 10, 40]);
}

#[cfg(feature = "qoi")]
#[test]
fn decode_qoi() {
    for alpha in [true, false] {
        let data = pattern(19, 7, alpha);
        let src: Vec<u8> = if alpha {
            data.clone()
        } else {
            data.chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        };
        let file = qoi::encode_to_vec(&src, 19, 7).unwrap();
        let img = still(&file, "a.qoi");
        assert_eq!(img.data(), &data[..], "alpha {alpha}");
    }
}

#[cfg(feature = "hdr")]
#[test]
fn decode_hdr() {
    // Flat RGBE, 3 x 2: values 1.0 gray, 2.0 red, 0, then 0.5 blue.
    let mut f = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\nEXPOSURE=1.0\n\n-Y 2 +X 3\n".to_vec();
    f.extend_from_slice(&[128, 128, 128, 129, 128, 0, 0, 130, 0, 0, 0, 0]);
    f.extend_from_slice(&[0, 0, 128, 128, 64, 64, 64, 129, 255, 255, 255, 136]);
    let img = still(&f, "a.hdr");
    assert_eq!(img.format(), PixelFormat::RgbaF32);
    assert_eq!(pxf(&img, 0, 0), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(pxf(&img, 1, 0), [2.0, 0.0, 0.0, 1.0]);
    assert_eq!(pxf(&img, 2, 0), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(pxf(&img, 0, 1), [0.0, 0.0, 0.5, 1.0]);
    assert_eq!(pxf(&img, 1, 1), [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(pxf(&img, 2, 1), [255.0, 255.0, 255.0, 1.0]);

    // Adaptive RLE scanlines, 8 wide, stored bottom-up (+Y).
    let mut f = b"#?RGBE\n\n+Y 2 +X 8\n".to_vec();
    for row in 0..2u8 {
        f.extend_from_slice(&[2, 2, 0, 8]);
        // R: run of 8; G: literal 8; B: run of 8; E: run of 8.
        f.extend_from_slice(&[128 + 8, 64 * (row + 1)]);
        f.push(8);
        f.extend_from_slice(&[0, 16, 32, 48, 64, 80, 96, 112]);
        f.extend_from_slice(&[128 + 8, 0]);
        f.extend_from_slice(&[128 + 8, 129]);
    }
    let img = still(&f, "b.hdr");
    assert_eq!((img.width(), img.height()), (8, 2));
    // The second stored row (row index 1) is the top row upright.
    assert_eq!(pxf(&img, 0, 0)[0], 1.0);
    assert_eq!(pxf(&img, 0, 1)[0], 0.5);
    assert_eq!(pxf(&img, 3, 0)[1], 48.0 / 128.0);
}

#[cfg(feature = "psd")]
mod psd_files {
    /// Build a PSD: header, empty color mode data unless given, resources,
    /// layer section, then image data with the given compression.
    #[allow(clippy::too_many_arguments)]
    pub fn psd(
        channels: u16,
        w: u32,
        h: u32,
        depth: u16,
        mode: u16,
        layer_count: Option<i16>,
        compression: u16,
        data: &[u8],
    ) -> Vec<u8> {
        let mut v = b"8BPS".to_vec();
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&[0; 6]);
        v.extend_from_slice(&channels.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&depth.to_be_bytes());
        v.extend_from_slice(&mode.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        match layer_count {
            Some(n) => {
                // Layer and mask section holding only a layer count.
                let mut info = (n as u16).to_be_bytes().to_vec();
                info.extend_from_slice(&[0, 0]);
                let mut section = (info.len() as u32).to_be_bytes().to_vec();
                section.extend(info);
                section.extend_from_slice(&0u32.to_be_bytes());
                v.extend_from_slice(&(section.len() as u32).to_be_bytes());
                v.extend(section);
            }
            None => v.extend_from_slice(&0u32.to_be_bytes()),
        }
        v.extend_from_slice(&compression.to_be_bytes());
        v.extend_from_slice(data);
        v
    }
}

#[cfg(feature = "psd")]
#[test]
fn decode_psd() {
    use psd_files::psd;
    // RGB 8-bit raw, planar: R plane, G plane, B plane (2 x 1).
    let file = psd(3, 2, 1, 8, 3, None, 0, &[10, 20, 30, 40, 50, 60]);
    let img = still(&file, "a.psd");
    assert_eq!(px(&img, 0, 0), [10, 30, 50, 255]);
    assert_eq!(px(&img, 1, 0), [20, 40, 60, 255]);

    // RGBA with merged transparency (negative layer count), RLE compressed.
    // Each channel row is one PackBits run of two equal bytes: (-1, value).
    let mut data = Vec::new();
    for _ in 0..4 {
        data.extend_from_slice(&2u16.to_be_bytes());
    }
    for v in [200u8, 100, 50, 128] {
        data.extend_from_slice(&[0xFF, v]);
    }
    let file = psd(4, 2, 1, 8, 3, Some(-1), 1, &data);
    let img = still(&file, "b.psd");
    assert_eq!(px(&img, 1, 0), [200, 100, 50, 128]);

    // The same document without merged transparency ignores channel 4.
    let file = psd(4, 2, 1, 8, 3, Some(1), 1, &data);
    assert_eq!(px(&still(&file, "c.psd"), 0, 0), [200, 100, 50, 255]);

    // Grayscale 16-bit and CMYK 8-bit (stored inverted).
    let file = psd(1, 1, 1, 16, 1, None, 0, &[0x12, 0x34]);
    let img = still(&file, "d.psd");
    assert_eq!(px16(&img, 0, 0), [0x1234, 0x1234, 0x1234, 0xFFFF]);
    let file = psd(4, 1, 1, 8, 4, None, 0, &[0, 255, 255, 255]);
    assert_eq!(
        px(&still(&file, "e.psd"), 0, 0),
        [0, 255, 255, 255],
        "full cyan"
    );

    // 32-bit RGB is linear float.
    let mut data = Vec::new();
    for v in [1.5f32, 0.25, 0.0] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let img = still(&psd(3, 1, 1, 32, 3, None, 0, &data), "f.psd");
    assert_eq!(pxf(&img, 0, 0), [1.5, 0.25, 0.0, 1.0]);
}

/// A DDS file with a legacy header, optional DX10 header, and payload.
#[cfg(feature = "dds")]
#[allow(clippy::too_many_arguments)]
fn dds(
    w: u32,
    h: u32,
    pf_flags: u32,
    fourcc: &[u8; 4],
    bits: u32,
    masks: [u32; 4],
    dx10: Option<u32>,
    data: &[u8],
) -> Vec<u8> {
    let mut v = b"DDS ".to_vec();
    let mut hdr = [0u32; 31];
    hdr[0] = 124;
    hdr[1] = 0x1007;
    hdr[2] = h;
    hdr[3] = w;
    hdr[18] = 32;
    hdr[19] = pf_flags;
    hdr[20] = u32::from_le_bytes(*fourcc);
    hdr[21] = bits;
    hdr[22..26].copy_from_slice(&masks);
    hdr[26] = 0x1000;
    for x in hdr {
        v.extend_from_slice(&x.to_le_bytes());
    }
    if let Some(fmt) = dx10 {
        for x in [fmt, 3, 0, 1, 0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
    }
    v.extend_from_slice(data);
    v
}

/// One BC1 block: endpoints red and blue, all pixels index `i`, except
/// the first row which uses index 1.
#[cfg(any(feature = "dds", feature = "ktx2"))]
fn bc1_block() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0xF800u16.to_le_bytes());
    b.extend_from_slice(&0x001Fu16.to_le_bytes());
    // Row 0: index 1 (blue). Rows 1-3: index 0 (red).
    b.extend_from_slice(&[0b0101_0101, 0, 0, 0]);
    b
}

#[cfg(feature = "dds")]
#[test]
fn decode_dds() {
    // Uncompressed 32-bit BGRA with masks.
    let data = [1, 2, 3, 4, 5, 6, 7, 8];
    let file = dds(
        2,
        1,
        0x41,
        b"\0\0\0\0",
        32,
        [0xFF0000, 0xFF00, 0xFF, 0xFF00_0000],
        None,
        &data,
    );
    let img = still(&file, "a.dds");
    assert_eq!(px(&img, 0, 0), [3, 2, 1, 4]);
    assert_eq!(px(&img, 1, 0), [7, 6, 5, 8]);

    // DXT1 (BC1), 4 x 4.
    let file = dds(4, 4, 0x4, b"DXT1", 0, [0; 4], None, &bc1_block());
    let img = still(&file, "b.dds");
    assert_eq!(px(&img, 0, 0), [0, 0, 255, 255]);
    assert_eq!(px(&img, 3, 3), [255, 0, 0, 255]);

    // BC1 with a size that is not a multiple of 4 is clipped.
    let file = dds(3, 2, 0x4, b"DXT1", 0, [0; 4], None, &bc1_block());
    let img = still(&file, "c.dds");
    assert_eq!((img.width(), img.height()), (3, 2));
    assert_eq!(px(&img, 2, 1), [255, 0, 0, 255]);

    // DXT2 (BC2 with premultiplied color): alpha 136, stored red 132
    // becomes straight red 248.
    let mut block = vec![0x88u8; 8];
    block.extend_from_slice(&[0x00, 0x80, 0, 0, 0, 0, 0, 0]);
    let file = dds(4, 4, 0x4, b"DXT2", 0, [0; 4], None, &block);
    assert_near(
        px(&still(&file, "p.dds"), 1, 1),
        [248, 0, 0, 136],
        1,
        "dxt2",
    );

    // DX10 R8G8B8A8_UNORM and R32G32B32A32_FLOAT.
    let file = dds(1, 1, 0x4, b"DX10", 0, [0; 4], Some(28), &[9, 8, 7, 6]);
    assert_eq!(px(&still(&file, "d.dds"), 0, 0), [9, 8, 7, 6]);
    let mut f32s = Vec::new();
    for v in [4.0f32, 0.5, 0.125, 1.0] {
        f32s.extend_from_slice(&v.to_le_bytes());
    }
    let file = dds(1, 1, 0x4, b"DX10", 0, [0; 4], Some(2), &f32s);
    let img = still(&file, "e.dds");
    assert_eq!(img.format(), PixelFormat::RgbaF32);
    assert_eq!(pxf(&img, 0, 0), [4.0, 0.5, 0.125, 1.0]);
}

/// A KTX2 file with one level.
#[cfg(feature = "ktx2")]
fn ktx2(vk: u32, w: u32, h: u32, scheme: u32, kvd: &[u8], level: &[u8], raw_len: usize) -> Vec<u8> {
    let mut v = vec![
        0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
    ];
    let kvd_at = 80 + 24;
    let level_at = (kvd_at + kvd.len()).next_multiple_of(16);
    for x in [
        vk,
        1,
        w,
        h,
        0,
        0,
        1,
        1,
        scheme,
        0,
        0,
        kvd_at as u32,
        kvd.len() as u32,
    ] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&0u64.to_le_bytes());
    v.extend_from_slice(&0u64.to_le_bytes());
    for x in [level_at as u64, level.len() as u64, raw_len as u64] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(kvd);
    v.resize(level_at, 0);
    v.extend_from_slice(level);
    v
}

#[cfg(feature = "ktx2")]
#[test]
fn decode_ktx2() {
    // R8G8B8A8_UNORM, 2 x 2, origin bottom-left (KTXorientation "ru").
    let key = b"KTXorientation\0ru\0";
    let mut kvd = (key.len() as u32).to_le_bytes().to_vec();
    kvd.extend_from_slice(key);
    kvd.resize(kvd.len().next_multiple_of(4), 0);
    let data = [
        1, 2, 3, 4, 5, 6, 7, 8, /* top row */ 9, 10, 11, 12, 13, 14, 15, 16,
    ];
    let file = ktx2(37, 2, 2, 0, &kvd, &data, data.len());
    let img = still(&file, "a.ktx2");
    assert_eq!(
        px(&img, 0, 0),
        [9, 10, 11, 12],
        "flipped to top-left origin"
    );
    assert_eq!(px(&img, 1, 1), [5, 6, 7, 8]);

    // BC1 with Zstandard supercompression.
    let block = bc1_block();
    let z =
        ruzstd::encoding::compress_to_vec(&block[..], ruzstd::encoding::CompressionLevel::Fastest);
    let file = ktx2(131, 4, 4, 2, &[], &z, block.len());
    let img = still(&file, "b.ktx2");
    assert_eq!(px(&img, 2, 0), [0, 0, 255, 255]);
    assert_eq!(px(&img, 2, 2), [255, 0, 0, 255]);

    // R16G16B16A16_SFLOAT decodes to linear float.
    let halfs: Vec<u8> = [0x3C00u16, 0x4000, 0x3800, 0x3C00]
        .iter()
        .flat_map(|h| h.to_le_bytes())
        .collect();
    let file = ktx2(97, 1, 1, 0, &[], &halfs, halfs.len());
    let img = still(&file, "c.ktx2");
    assert_eq!(pxf(&img, 0, 0), [1.0, 2.0, 0.5, 1.0]);

    // Basis Universal payloads are reported as unsupported, not misread.
    let file = ktx2(0, 4, 4, 1, &[], &[0; 16], 16);
    assert!(matches!(
        reg().decode(&source(&file, "d.ktx2"), &DecodeOptions::default()),
        Err(pandite_core::Error::Unsupported(_))
    ));
}

#[cfg(feature = "exr")]
#[test]
fn decode_exr() {
    use exr::prelude::*;
    use std::io::Cursor;
    // RGBA float, 3 x 2.
    let px_at = |x: usize, y: usize| (x as f32 * 0.5, y as f32 * 2.0, 4.0, 1.0 - x as f32 * 0.25);
    let image = Image::from_channels((3, 2), SpecificChannels::rgba(|Vec2(x, y)| px_at(x, y)));
    let mut buf = Cursor::new(Vec::new());
    image.write().to_buffered(&mut buf).unwrap();
    let img = still(&buf.into_inner(), "a.exr");
    assert_eq!(img.format(), PixelFormat::RgbaF32);
    assert_eq!((img.width(), img.height()), (3, 2));
    // EXR stores premultiplied color: (1, 2, 4) at alpha 0.5 is straight
    // (2, 4, 8).
    assert_eq!(pxf(&img, 2, 1), [2.0, 4.0, 8.0, 0.5]);
    assert_eq!(pxf(&img, 0, 0), [0.0, 0.0, 4.0, 1.0]);

    // Luminance only, half floats.
    let y: Vec<f16> = [0.5f32, 8.0].iter().map(|&v| f16::from_f32(v)).collect();
    let layer = Layer::new(
        (2, 1),
        LayerAttributes::default(),
        Encoding::default(),
        AnyChannels::sort(vec![AnyChannel::new("Y", FlatSamples::F16(y))].into()),
    );
    let mut buf = Cursor::new(Vec::new());
    Image::from_layer(layer)
        .write()
        .to_buffered(&mut buf)
        .unwrap();
    let img = still(&buf.into_inner(), "b.exr");
    assert_eq!(pxf(&img, 1, 0), [8.0, 8.0, 8.0, 1.0]);
}

#[cfg(feature = "svg")]
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50">
  <rect x="0" y="0" width="50" height="50" fill="#ff0000"/>
  <rect x="50" y="0" width="50" height="50" fill="#0000ff" fill-opacity="0.5"/>
</svg>"##;

#[cfg(feature = "svg")]
#[test]
fn decode_svg() {
    let img = still(SVG.as_bytes(), "a.svg");
    assert_eq!((img.width(), img.height()), (100, 50));
    assert_eq!(px(&img, 25, 25), [255, 0, 0, 255]);
    assert_near(
        px(&img, 75, 25),
        [0, 0, 255, 128],
        1,
        "half transparent blue",
    );
    let meta = reg().probe(&source(SVG.as_bytes(), "a.svg")).unwrap();
    assert_eq!((meta.width, meta.height), (100, 50));
    // Sniffed without an extension, even behind an XML declaration.
    let doc = format!("<?xml version=\"1.0\"?>\n{SVG}");
    let src = pandite_core::Source::from_bytes(doc.into_bytes(), None);
    assert_eq!(reg().decoder_for(&src).unwrap().id(), "svg");
}

#[cfg(feature = "svg")]
#[test]
fn svg_honors_target_size() {
    // Larger than intrinsic: rendered sharp at 400 x 200, not upscaled.
    let options = DecodeOptions {
        target_size: Some((400, 400)),
        ..DecodeOptions::default()
    };
    let img = decode_with(SVG.as_bytes(), "a.svg", &options);
    let img = img.first_image();
    assert_eq!((img.width(), img.height()), (400, 200));
    assert_eq!(px(img, 199, 100), [255, 0, 0, 255], "edge stays crisp");
    assert_near(px(img, 200, 100), [0, 0, 255, 128], 1, "next pixel is blue");
    // Smaller than intrinsic.
    let options = DecodeOptions {
        target_size: Some((20, 20)),
        ..DecodeOptions::default()
    };
    let img = decode_with(SVG.as_bytes(), "a.svg", &options);
    assert_eq!(img.dimensions(), (20, 10));
}

#[cfg(feature = "jxl")]
#[test]
fn decode_jxl() {
    // Lossless fixture: left half opaque red, right half half-transparent blue.
    let img = still(&fixture("blocks.jxl"), "blocks.jxl");
    assert_eq!((img.width(), img.height()), (40, 24));
    assert_eq!(img.format(), PixelFormat::Rgba8);
    assert_eq!(px(&img, 5, 5), [230, 30, 30, 255]);
    assert_eq!(px(&img, 30, 20), [30, 40, 220, 128]);
    let meta = reg()
        .probe(&source(&fixture("blocks.jxl"), "x.jxl"))
        .unwrap();
    assert_eq!((meta.width, meta.height, meta.has_alpha), (40, 24, true));

    // Lossy (XYB) codestream.
    let img = still(&fixture("lossy.jxl"), "lossy.jxl");
    assert_eq!(img.format(), PixelFormat::Rgba8);
    assert_near(px(&img, 5, 12), [230, 30, 30, 255], 6, "lossy red");
    assert_near(px(&img, 34, 12), [30, 40, 220, 255], 6, "lossy blue");

    // Two-frame animation at 5 frames per second.
    match decode(&fixture("anim.jxl"), "anim.jxl") {
        Content::Animation(a) => {
            assert_eq!(a.frames().len(), 2);
            for f in a.frames() {
                assert_eq!(f.delay, std::time::Duration::from_millis(200));
            }
            assert_eq!(px(&a.frames()[0].image, 8, 8), [220, 30, 30, 255]);
            assert_eq!(px(&a.frames()[1].image, 8, 8), [20, 40, 230, 255]);
        }
        other => panic!("expected an animation, got {:?}", other.kind()),
    }
}

#[cfg(feature = "avif")]
#[test]
fn decode_avif() {
    // 8-bit 4:4:4 lossy fixture of four color blocks.
    let img = still(&fixture("blocks.avif"), "blocks.avif");
    assert_eq!((img.width(), img.height()), (64, 64));
    assert_eq!(img.format(), PixelFormat::Rgba8);
    assert_near(px(&img, 10, 10), [230, 30, 30, 255], 6, "red block");
    assert_near(px(&img, 50, 10), [30, 200, 40, 255], 6, "green block");
    assert_near(px(&img, 10, 50), [30, 40, 220, 255], 6, "blue block");
    assert_near(px(&img, 50, 50), [240, 240, 240, 255], 6, "white block");

    // Alpha auxiliary image: top half opaque, bottom half alpha 64.
    let img = still(&fixture("alpha.avif"), "alpha.avif");
    assert_near(px(&img, 16, 4), [250, 200, 20, 255], 6, "opaque half");
    let low = px(&img, 16, 28);
    assert!(low[3].abs_diff(64) <= 3, "alpha {low:?}");
    assert_near(low, [250, 200, 20, 64], 8, "translucent half");
    let meta = reg()
        .probe(&source(&fixture("alpha.avif"), "x.avif"))
        .unwrap();
    assert_eq!((meta.width, meta.height, meta.has_alpha), (32, 32, true));

    // irot (stored 64 x 32, left red, right blue; turned 90 degrees
    // clockwise): upright it is 32 x 64 with red on top.
    let img = still(&fixture("rotated.avif"), "rotated.avif");
    assert_eq!((img.width(), img.height()), (32, 64));
    assert_near(px(&img, 16, 10), [230, 30, 30, 255], 8, "red on top");
    assert_near(px(&img, 16, 54), [30, 40, 220, 255], 8, "blue below");
    let meta = reg()
        .probe(&source(&fixture("rotated.avif"), "r.avif"))
        .unwrap();
    assert_eq!((meta.width, meta.height), (32, 64));

    // 10-bit 4:2:0 decodes to 16 bits per channel.
    let img = still(&fixture("deep.avif"), "deep.avif");
    assert_eq!(img.format(), PixelFormat::Rgba16);
    let to8 = |p: [u16; 4]| p.map(|v| ((u32::from(v) + 128) / 257) as u8);
    assert_near(to8(px16(&img, 10, 10)), [230, 30, 30, 255], 8, "deep red");
    assert_near(to8(px16(&img, 50, 10)), [30, 200, 40, 255], 8, "deep green");
    assert_near(to8(px16(&img, 10, 50)), [30, 40, 220, 255], 8, "deep blue");
    assert_near(
        to8(px16(&img, 50, 50)),
        [240, 240, 240, 255],
        8,
        "deep white",
    );

    // A 2 x 1 grid of 32 x 32 tiles (red, blue) cropped to 60 x 32 by the
    // grid's output size.
    let img = still(&fixture("grid.avif"), "grid.avif");
    assert_eq!((img.width(), img.height()), (60, 32));
    assert_near(px(&img, 10, 16), [230, 30, 30, 255], 6, "left tile");
    assert_near(px(&img, 50, 16), [30, 40, 220, 255], 6, "right tile");
}

#[cfg(feature = "raw")]
mod raw_files {
    use crate::common::dng::*;
    use crate::common::*;

    /// A preview JPEG of solid green, 8 x 4.
    pub fn green_preview() -> Vec<u8> {
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 100)
            .encode(
                &solid(8, 4, [0, 255, 0, 255]),
                8,
                4,
                jpeg_encoder::ColorType::Rgba,
            )
            .unwrap();
        out
    }

    /// Camera RGB that develops to the given linear sRGB with an as-shot
    /// neutral of (0.5, 1, 0.5).
    pub fn camera(lin: [f64; 3]) -> [f64; 3] {
        [lin[0] * 0.5, lin[1], lin[2] * 0.5]
    }

    pub const LEFT: [f64; 3] = [0.6, 0.1, 0.05];
    pub const RIGHT: [f64; 3] = [0.05, 0.2, 0.5];

    /// A 16 x 8 RGGB DNG: left half LEFT, right half RIGHT.
    pub fn dng(compression: u16, sensor: Vec<u8>, extra: Ifd, orientation: u16) -> Vec<u8> {
        let preview = green_preview();
        let ifd0: Ifd = vec![
            (254, V::Long(vec![1])),
            (256, V::Long(vec![8])),
            (257, V::Long(vec![4])),
            (258, V::Short(vec![8, 8, 8])),
            (259, V::Short(vec![7])),
            (262, V::Short(vec![6])),
            (274, V::Short(vec![orientation])),
            (277, V::Short(vec![3])),
            (279, V::Long(vec![preview.len() as u32])),
            (273, V::Data(preview)),
            (330, V::Ifd(0)),
            (50706, V::Byte(vec![1, 4, 0, 0])),
            (50708, V::Ascii("pandite test camera")),
            (50721, V::SRational(xyz_to_srgb_matrix())),
            (50728, V::Rational(vec![(1, 2), (1, 1), (1, 2)])),
            (50778, V::Short(vec![21])),
        ];
        let mut raw: Ifd = vec![
            (254, V::Long(vec![0])),
            (256, V::Long(vec![16])),
            (257, V::Long(vec![8])),
            (258, V::Short(vec![16])),
            (259, V::Short(vec![compression])),
            (262, V::Short(vec![32803])),
            (277, V::Short(vec![1])),
            (278, V::Long(vec![8])),
            (279, V::Long(vec![sensor.len() as u32])),
            (273, V::Data(sensor)),
            (33421, V::Short(vec![2, 2])),
            (33422, V::Byte(vec![0, 1, 1, 2])),
            (50714, V::Long(vec![64])),
            (50717, V::Long(vec![4095])),
        ];
        raw.extend(extra);
        tiff(ifd0, vec![raw])
    }

    pub fn sensor() -> Vec<u16> {
        mosaic(16, 8, |x, _| camera(if x < 8 { LEFT } else { RIGHT }))
    }

    /// 16-bit sRGB code of a linear value.
    pub fn code(lin: f64) -> u16 {
        let e = if lin <= 0.003_130_8 {
            lin * 12.92
        } else {
            1.055 * lin.powf(1.0 / 2.4) - 0.055
        };
        (e * 65535.0).round() as u16
    }

    pub fn assert_color(p: [u16; 4], lin: [f64; 3], what: &str) {
        for c in 0..3 {
            let want = code(lin[c]);
            assert!(
                p[c].abs_diff(want) <= 200,
                "{what}: channel {c} is {} want {want} ({p:?})",
                p[c]
            );
        }
        assert_eq!(p[3], u16::MAX);
    }
}

#[cfg(feature = "raw")]
#[test]
fn decode_raw() {
    use common::dng::*;
    use raw_files::*;

    // Uncompressed DNG: developed from sensor data, 16-bit sRGB.
    let bytes: Vec<u8> = sensor().iter().flat_map(|v| v.to_le_bytes()).collect();
    let file = dng(1, bytes, vec![], 1);
    let img = still(&file, "a.dng");
    assert_eq!((img.width(), img.height()), (16, 8));
    assert_eq!(img.format(), PixelFormat::Rgba16);
    for (x, y) in [(2, 3), (4, 4), (5, 2)] {
        assert_color(px16(&img, x, y), LEFT, "left half");
    }
    for (x, y) in [(10, 3), (12, 5), (13, 2)] {
        assert_color(px16(&img, x, y), RIGHT, "right half");
    }
    let meta = reg().probe(&source(&file, "a.dng")).unwrap();
    assert_eq!((meta.width, meta.height, meta.format), (16, 8, "raw"));

    // Lossless JPEG DNG (two interleaved components per row, the usual DNG
    // layout), with a default crop and orientation 6 (rotate 90 clockwise).
    let lj = ljpeg(&sensor(), 8, 8, 2, 12);
    let crop: Ifd = vec![(50719, V::Long(vec![2, 2])), (50720, V::Long(vec![12, 4]))];
    let file = dng(7, lj, crop, 6);
    let img = still(&file, "b.dng");
    assert_eq!((img.width(), img.height()), (4, 12), "cropped and rotated");
    assert_color(px16(&img, 1, 2), LEFT, "rotated left part");
    assert_color(px16(&img, 2, 10), RIGHT, "rotated right part");

    // A small target uses the embedded preview.
    let options = DecodeOptions {
        target_size: Some((4, 4)),
        ..DecodeOptions::default()
    };
    let bytes: Vec<u8> = sensor().iter().flat_map(|v| v.to_le_bytes()).collect();
    let file = dng(1, bytes, vec![], 1);
    let img = decode_with(&file, "c.dng", &options);
    let img = img.first_image();
    assert_eq!(img.format(), PixelFormat::Rgba8);
    assert_near(px(img, 1, 1), [0, 255, 0, 255], 3, "preview pixels");

    // A proprietary RAW (sensor data in a vendor compression) shows its
    // preview, also when the TIFF signature routes it through TIFF.
    let file = dng(34713, vec![0; 64], vec![], 1);
    let img = still(&file, "d.nef");
    assert_eq!((img.width(), img.height()), (8, 4));
    assert_near(px(&img, 3, 2), [0, 255, 0, 255], 3, "nef preview");

    // CR3 (ISO base media) with an embedded JPEG.
    let mut cr3 = vec![0, 0, 0, 0x18];
    cr3.extend_from_slice(b"ftypcrx \0\0\0\x01crx isom");
    cr3.extend_from_slice(&[0u8; 40]);
    cr3.extend(green_preview());
    cr3.extend_from_slice(&[0u8; 16]);
    let img = still(&cr3, "e.cr3");
    assert_near(px(&img, 0, 0), [0, 255, 0, 255], 3, "cr3 preview");
}

#[cfg(feature = "heic")]
#[test]
fn heic_provider_registered() {
    let found = reg().decoders().iter().find(|d| d.id() == "heic");
    if cfg!(any(windows, target_os = "macos")) {
        let d = found.expect("the HEIC provider is registered on Windows and macOS");
        assert_eq!(d.extensions(), &["heic", "heif"]);
        // HEIF brands are claimed, AVIF brands are left to the AVIF decoder.
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
        assert!(d.sniff(heic));
        assert!(!d.sniff(b"\0\0\0\x18ftypavif\0\0\0\0mif1avif"));
        let src = source(heic, "photo.HEIC");
        assert_eq!(reg().decoder_for(&src).map(|p| p.id()), Some("heic"));
        assert!(reg().can_open_path(std::path::Path::new("x.heif")));
    } else {
        assert!(found.is_none(), "no HEIC provider without an OS codec");
    }
}

/// Decodes a real HEIC file through the operating system codec. Opt-in:
/// Windows needs the HEIF Image Extensions and HEVC Video Extensions from
/// the Microsoft Store, which build machines usually lack.
#[cfg(all(feature = "heic", any(windows, target_os = "macos")))]
#[test]
#[ignore = "needs the HEIF and HEVC codecs of the operating system"]
fn heic_decodes_through_os_codec() {
    let img = still(&fixture("blocks.heic"), "blocks.heic");
    assert_eq!((img.width(), img.height()), (64, 64));
    assert_near(px(&img, 10, 10), [230, 30, 30, 255], 12, "red block");
    assert_near(px(&img, 50, 10), [30, 200, 40, 255], 12, "green block");
    assert_near(px(&img, 10, 50), [30, 40, 220, 255], 12, "blue block");
    assert_near(px(&img, 50, 50), [240, 240, 240, 255], 12, "white block");
}

/// An embedded ICC profile converts pixels to sRGB: the same numbers mean
/// a more saturated red in Display P3 than in sRGB.
#[cfg(feature = "icc")]
#[test]
fn icc_profile_converts_to_srgb() {
    use common::hand::*;
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let mut iccp = b"Display P3\0\0".to_vec();
    iccp.extend(zlib_stored(&profile));
    let raw = [0u8, 200, 100, 100, 128, 128, 128];
    let plain = still(&png_file(2, 1, 8, 2, 0, &[], &raw), "plain.png");
    assert_eq!(px(&plain, 0, 0), [200, 100, 100, 255]);
    let file = png_file(2, 1, 8, 2, 0, &[png_chunk(b"iCCP", &iccp)], &raw);
    let img = still(&file, "p3.png");
    // Expected from the P3 to sRGB matrix: (0.578, 0.127, 0.127) linear P3
    // becomes (0.680, 0.108, 0.118) linear sRGB, which encodes to
    // (215.6, 92.8, 96.5).
    assert_near(px(&img, 0, 0), [216, 93, 97, 255], 2, "converted red");
    // Neutral gray stays neutral.
    assert_near(px(&img, 1, 0), [128, 128, 128, 255], 1, "gray");
}

/// Text renders with the system fonts (Windows and macOS always ship some).
#[cfg(all(feature = "svg-text", any(windows, target_os = "macos")))]
#[test]
fn svg_renders_text_with_system_fonts() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60">
      <rect width="200" height="60" fill="#fff"/>
      <text x="10" y="45" font-family="Arial, Helvetica, sans-serif" font-size="40" fill="#000">Hello</text>
    </svg>"##;
    let img = still(doc.as_bytes(), "text.svg");
    let dark = img
        .data()
        .chunks_exact(4)
        .filter(|p| p[0] < 64 && p[3] == 255)
        .count();
    // "Hello" at 40 px covers well over a few hundred pixels.
    assert!(dark > 300, "only {dark} dark pixels: text did not render");
    // The area right of the word stays white.
    assert_eq!(px(&img, 195, 30), [255, 255, 255, 255]);
}
