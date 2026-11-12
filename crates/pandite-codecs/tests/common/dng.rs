//! Hand-built DNG and lossless JPEG files for the RAW decoder tests.
#![allow(dead_code)]

/// One TIFF directory entry value.
pub enum V {
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
    Byte(Vec<u8>),
    Ascii(&'static str),
    /// A LONG offset to data placed later in the file.
    Data(Vec<u8>),
    /// A LONG offset to another directory (SubIFDs).
    Ifd(usize),
}

/// A TIFF directory under construction: (tag, value) pairs.
pub type Ifd = Vec<(u16, V)>;

/// Serialize a little-endian TIFF with IFD0 and extra directories that IFD0
/// (or each other) reference through `V::Ifd(index into extra)`.
pub fn tiff(ifd0: Ifd, extra: Vec<Ifd>) -> Vec<u8> {
    let mut all = vec![ifd0];
    all.extend(extra);
    // Pass 1: compute directory offsets; data blobs follow all directories.
    let dir_size = |d: &Ifd| 2 + 12 * d.len() + 4;
    let mut offsets = Vec::new();
    let mut at = 8;
    for d in &all {
        offsets.push(at);
        at += dir_size(d);
    }
    let mut blobs: Vec<u8> = Vec::new();
    let blob_base = at;
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&(offsets[0] as u32).to_le_bytes());
    let mut dirs = Vec::new();
    for d in &all {
        let mut entries: Vec<&(u16, V)> = d.iter().collect();
        entries.sort_by_key(|(t, _)| *t);
        let mut bytes = (entries.len() as u16).to_le_bytes().to_vec();
        for (tag, v) in entries {
            let (typ, count, payload): (u16, u32, Vec<u8>) = match v {
                V::Short(x) => (
                    3,
                    x.len() as u32,
                    x.iter().flat_map(|v| v.to_le_bytes()).collect(),
                ),
                V::Long(x) => (
                    4,
                    x.len() as u32,
                    x.iter().flat_map(|v| v.to_le_bytes()).collect(),
                ),
                V::Rational(x) => (
                    5,
                    x.len() as u32,
                    x.iter()
                        .flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
                        .collect(),
                ),
                V::SRational(x) => (
                    10,
                    x.len() as u32,
                    x.iter()
                        .flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
                        .collect(),
                ),
                V::Byte(x) => (1, x.len() as u32, x.clone()),
                V::Ascii(s) => {
                    let mut b = s.as_bytes().to_vec();
                    b.push(0);
                    (2, b.len() as u32, b)
                }
                V::Data(x) => {
                    let off = blob_base + blobs.len();
                    blobs.extend_from_slice(x);
                    if blobs.len() % 2 == 1 {
                        blobs.push(0);
                    }
                    (4, 1, (off as u32).to_le_bytes().to_vec())
                }
                V::Ifd(i) => (4, 1, (offsets[*i + 1] as u32).to_le_bytes().to_vec()),
            };
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&typ.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            if payload.len() <= 4 {
                let mut p = payload.clone();
                p.resize(4, 0);
                bytes.extend_from_slice(&p);
            } else {
                let off = blob_base + blobs.len();
                blobs.extend_from_slice(&payload);
                if blobs.len() % 2 == 1 {
                    blobs.push(0);
                }
                bytes.extend_from_slice(&(off as u32).to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        dirs.push(bytes);
    }
    for d in dirs {
        out.extend(d);
    }
    assert_eq!(out.len(), blob_base);
    out.extend(blobs);
    out
}

/// Encode samples (interleaved, `comps` per pixel) as a lossless JPEG with
/// predictor 1 and a single Huffman table.
pub fn ljpeg(samples: &[u16], w: usize, h: usize, comps: usize, precision: u8) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    // DHT: 17 symbols (categories 0 to 16), all 5-bit codes.
    let mut dht = vec![0x00];
    let mut bits = [0u8; 16];
    bits[4] = 17;
    dht.extend_from_slice(&bits);
    dht.extend(0u8..17);
    out.extend_from_slice(&[0xFF, 0xC4]);
    out.extend_from_slice(&((dht.len() + 2) as u16).to_be_bytes());
    out.extend(dht);
    // SOF3.
    let mut sof = vec![precision];
    sof.extend_from_slice(&(h as u16).to_be_bytes());
    sof.extend_from_slice(&(w as u16).to_be_bytes());
    sof.push(comps as u8);
    for c in 0..comps {
        sof.extend_from_slice(&[c as u8 + 1, 0x11, 0]);
    }
    out.extend_from_slice(&[0xFF, 0xC3]);
    out.extend_from_slice(&((sof.len() + 2) as u16).to_be_bytes());
    out.extend(sof);
    // SOS: predictor 1, no point transform.
    let mut sos = vec![comps as u8];
    for c in 0..comps {
        sos.extend_from_slice(&[c as u8 + 1, 0x00]);
    }
    sos.extend_from_slice(&[1, 0, 0]);
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&((sos.len() + 2) as u16).to_be_bytes());
    out.extend(sos);
    // Entropy-coded data.
    struct W {
        acc: u64,
        n: u32,
        data: Vec<u8>,
    }
    impl W {
        fn put(&mut self, v: u32, len: u32) {
            self.acc = (self.acc << len) | u64::from(v);
            self.n += len;
            while self.n >= 8 {
                self.n -= 8;
                let b = (self.acc >> self.n) as u8;
                self.data.push(b);
                if b == 0xFF {
                    self.data.push(0);
                }
            }
        }
    }
    let mut wr = W {
        acc: 0,
        n: 0,
        data: Vec::new(),
    };
    let row = w * comps;
    for y in 0..h {
        for x in 0..w {
            for c in 0..comps {
                let i = y * row + x * comps + c;
                let pred: i32 = if y == 0 && x == 0 {
                    1 << (precision - 1)
                } else if x == 0 {
                    i32::from(samples[i - row])
                } else {
                    i32::from(samples[i - comps])
                };
                let diff = i32::from(samples[i]) - pred;
                let s = 32 - diff.unsigned_abs().leading_zeros();
                wr.put(s, 5);
                if s > 0 {
                    let extra = if diff >= 0 { diff } else { diff + (1 << s) - 1 };
                    wr.put(extra as u32, s);
                }
            }
        }
    }
    let pad = (8 - wr.n % 8) % 8;
    if pad > 0 {
        wr.put((1 << pad) - 1, pad);
    }
    let data = wr.data;
    out.extend(data);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// Linear sRGB 0..1 to XYZ matrix rows, scaled for SRATIONAL (x 10000),
/// inverted: XYZ to sRGB, as a DNG ColorMatrix that makes camera RGB equal
/// linear sRGB.
pub fn xyz_to_srgb_matrix() -> Vec<(i32, i32)> {
    [
        3.240_454_2,
        -1.537_138_5,
        -0.498_531_4,
        -0.969_266,
        1.876_010_8,
        0.041_556,
        0.055_643_4,
        -0.204_025_9,
        1.057_225_2,
    ]
    .iter()
    .map(|v: &f64| ((v * 10000.0).round() as i32, 10000))
    .collect()
}

/// Sensor values for a 2 x 2 RGGB mosaic from per-pixel camera RGB
/// (normalized 0..1), black 64 and white 4095.
pub fn mosaic(w: usize, h: usize, color: impl Fn(usize, usize) -> [f64; 3]) -> Vec<u16> {
    let mut v = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let c = match (y % 2, x % 2) {
                (0, 0) => 0,
                (1, 1) => 2,
                _ => 1,
            };
            v.push((64.0 + color(x, y)[c] * 4031.0).round() as u16);
        }
    }
    v
}
