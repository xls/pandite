//! Hand-built file builders for formats whose encoders cannot produce the
//! variant a test needs (interlaced PNG, RLE BMP, and similar).
#![allow(dead_code)]

/// CRC-32 (IEEE) as PNG uses it.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &b in *part {
            crc ^= u32::from(b);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }
    !crc
}

/// Adler-32 as zlib uses it.
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A zlib stream holding `data` in stored (uncompressed) deflate blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(65535).collect()
    };
    for (i, c) in chunks.iter().enumerate() {
        out.push(u8::from(i + 1 == chunks.len()));
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// One PNG chunk with length and CRC.
pub fn png_chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v.extend_from_slice(&crc32(&[kind, body]).to_be_bytes());
    v
}

/// A PNG file from IHDR fields, extra chunks before IDAT, and raw filtered
/// scanline data (filter bytes included).
pub fn png_file(
    w: u32,
    h: u32,
    depth: u8,
    color: u8,
    interlace: u8,
    extra: &[Vec<u8>],
    raw: &[u8],
) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[depth, color, 0, 0, interlace]);
    v.extend(png_chunk(b"IHDR", &ihdr));
    for c in extra {
        v.extend_from_slice(c);
    }
    v.extend(png_chunk(b"IDAT", &zlib_stored(raw)));
    v.extend(png_chunk(b"IEND", &[]));
    v
}

/// Adam7 pass origins and steps: (x0, y0, dx, dy).
pub const ADAM7: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// Raw scanlines of an 8-bit RGB image in Adam7 order, filter type 0.
pub fn adam7_rgb(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in ADAM7 {
        if x0 >= w || y0 >= h {
            continue;
        }
        let mut y = y0;
        while y < h {
            raw.push(0);
            let mut x = x0;
            while x < w {
                let i = ((y * w + x) * 3) as usize;
                raw.extend_from_slice(&rgb[i..i + 3]);
                x += dx;
            }
            y += dy;
        }
    }
    raw
}

/// A BMP file from a DIB header, palette or mask bytes, and pixel data.
pub fn bmp_file(dib_header: &[u8], between: &[u8], pixels: &[u8]) -> Vec<u8> {
    let offset = 14 + dib_header.len() + between.len();
    let size = offset + pixels.len();
    let mut v = b"BM".to_vec();
    v.extend_from_slice(&(size as u32).to_le_bytes());
    v.extend_from_slice(&[0, 0, 0, 0]);
    v.extend_from_slice(&(offset as u32).to_le_bytes());
    v.extend_from_slice(dib_header);
    v.extend_from_slice(between);
    v.extend_from_slice(pixels);
    v
}

/// A BITMAPINFOHEADER (40 bytes).
pub fn bmp_info_header(w: i32, h: i32, bpp: u16, compression: u32, colors: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&bpp.to_le_bytes());
    v.extend_from_slice(&compression.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&2835i32.to_le_bytes());
    v.extend_from_slice(&2835i32.to_le_bytes());
    v.extend_from_slice(&colors.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

/// A BMP palette (BGRX entries) from RGB colors.
pub fn bmp_palette(colors: &[[u8; 3]]) -> Vec<u8> {
    colors.iter().flat_map(|c| [c[2], c[1], c[0], 0]).collect()
}
