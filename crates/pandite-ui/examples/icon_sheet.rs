//! Render every interface icon at 16, 20, 24 and 40 pixels into one PNG
//! sheet, magnified with square pixels, for review. Also prints the
//! 16-pixel versions as text.
//!
//! Usage: `cargo run -p pandite-ui --example icon_sheet -- <out.png>`

use pandite_ui::Icon;

const SIZES: [u32; 4] = [16, 20, 24, 40];
const ZOOM: u32 = 4;
const PAD: u32 = 6;
const BG: u8 = 28;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "icon_sheet.png".to_string());
    let cell = SIZES.iter().max().copied().unwrap_or(40) + PAD;
    let cols = Icon::ALL.len() as u32;
    let rows = SIZES.len() as u32;
    let w = cols * cell + PAD;
    let h = rows * cell + PAD;
    let mut px = vec![BG; (w * h) as usize];
    for (r, &size) in SIZES.iter().enumerate() {
        for (c, icon) in Icon::ALL.iter().enumerate() {
            let mask = icon.coverage(size);
            let x0 = PAD + c as u32 * cell;
            let y0 = PAD + r as u32 * cell;
            // A slightly lighter square shows the icon bounds.
            for y in 0..size {
                for x in 0..size {
                    let a = u32::from(mask[(y * size + x) as usize]);
                    let base = 40u32;
                    let v = (base * (255 - a) + 255 * a) / 255;
                    px[((y0 + y) * w + x0 + x) as usize] = v as u8;
                }
            }
        }
    }
    // Magnify.
    let (zw, zh) = (w * ZOOM, h * ZOOM);
    let mut big = vec![0u8; (zw * zh) as usize];
    for y in 0..zh {
        for x in 0..zw {
            big[(y * zw + x) as usize] = px[((y / ZOOM) * w + x / ZOOM) as usize];
        }
    }
    std::fs::write(&out, png_gray(zw, zh, &big)).expect("write sheet");
    println!("wrote {out} ({zw} x {zh})");
    for icon in Icon::ALL {
        println!("{}:", icon.name());
        let mask = icon.coverage(16);
        for y in 0..16 {
            let line: String = (0..16)
                .map(|x| match mask[y * 16 + x] {
                    0..=40 => ' ',
                    41..=140 => '+',
                    _ => '#',
                })
                .collect();
            println!("  |{line}|");
        }
    }
}

/// A grayscale PNG with stored (uncompressed) deflate blocks.
fn png_gray(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&px[(y * w) as usize..((y + 1) * w) as usize]);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(block) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &v in &raw {
        a = (a + u32::from(v)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
