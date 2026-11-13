//! Readers shared by the providers' [`pandite_core::DecoderProvider::facts`]:
//! ICC profile descriptions, EXIF camera settings and TIFF code names.
//!
//! Everything here is bounds-checked header parsing: no allocation beyond
//! the strings returned, no offset followed outside the data, no pixel
//! decoded. Missing or malformed fields give `None`.

use crate::exif::{tag, Entry, Tiff};
use crate::util::be32;
use pandite_core::CameraFacts;

/// Longest text kept from a file (profile names, camera strings).
const MAX_TEXT: usize = 96;

/// Printable text from file bytes: up to the first NUL, control
/// characters dropped, trimmed, at most [`MAX_TEXT`] characters.
pub(crate) fn text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    clean(&String::from_utf8_lossy(&bytes[..end]))
}

/// `s` without control characters, trimmed and capped.
fn clean(s: &str) -> Option<String> {
    let kept: String = s
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect();
    let kept = kept.trim();
    (!kept.is_empty()).then(|| kept.to_string())
}

/// The description (`desc` tag) of an ICC profile: version 2 text
/// descriptions, version 4 localized texts (English first) and plain text.
pub(crate) fn icc_description(profile: &[u8]) -> Option<String> {
    if profile.get(36..40)? != b"acsp" {
        return None;
    }
    let count = be32(profile, 128)? as usize;
    (0..count.min(512)).find_map(|i| {
        let at = 132 + i * 12;
        if profile.get(at..at + 4)? != b"desc" {
            return None;
        }
        let off = be32(profile, at + 4)? as usize;
        let len = be32(profile, at + 8)? as usize;
        icc_text(profile.get(off..off.checked_add(len)?)?)
    })
}

fn icc_text(tag: &[u8]) -> Option<String> {
    match tag.get(..4)? {
        b"desc" => {
            let n = be32(tag, 8)? as usize;
            text(tag.get(12..12usize.checked_add(n)?)?)
        }
        b"text" => text(tag.get(8..)?),
        b"mluc" => {
            let records = be32(tag, 8)? as usize;
            let size = be32(tag, 12)? as usize;
            if size < 12 {
                return None;
            }
            let mut first = None;
            for i in 0..records.min(64) {
                let r = 16 + i * size;
                let (Some(lang), Some(len), Some(off)) =
                    (tag.get(r..r + 2), be32(tag, r + 4), be32(tag, r + 8))
                else {
                    break;
                };
                let (off, len) = (off as usize, len as usize);
                let Some(raw) = off.checked_add(len).and_then(|end| tag.get(off..end)) else {
                    continue;
                };
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                let Some(s) = clean(&String::from_utf16_lossy(&units)) else {
                    continue;
                };
                if lang == b"en" {
                    return Some(s);
                }
                first.get_or_insert(s);
            }
            first
        }
        _ => None,
    }
}

/// The ASCII value of `tag` in `entries`.
fn ascii(t: &Tiff<'_>, entries: &[Entry], tag: u16) -> Option<String> {
    let e = entries.iter().find(|e| e.tag == tag && e.typ == 2)?;
    text(t.bytes(e)?)
}

/// The first RATIONAL value of `tag`: numerator and denominator.
fn rational(t: &Tiff<'_>, entries: &[Entry], tag: u16) -> Option<(u32, u32)> {
    let e = entries
        .iter()
        .find(|e| e.tag == tag && matches!(e.typ, 5 | 10))?;
    let (n, d) = (t.u32(e.value_at)?, t.u32(e.value_at + 4)?);
    if e.typ == 10 && (n as i32 <= 0 || d as i32 <= 0) {
        return None;
    }
    (d != 0).then_some((n, d))
}

/// The first RATIONAL value of `tag` as a positive number.
fn real(t: &Tiff<'_>, entries: &[Entry], tag: u16) -> Option<f32> {
    let (n, d) = rational(t, entries, tag)?;
    let v = (f64::from(n) / f64::from(d)) as f32;
    (v.is_finite() && v > 0.0).then_some(v)
}

/// An EXIF date ("YYYY:MM:DD HH:MM:SS") as "YYYY-MM-DD HH:MM:SS". `None`
/// for any other shape and for the all-zero "unknown" date.
pub(crate) fn exif_date(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let shape = b.len() >= 19
        && b[4] == b':'
        && b[7] == b':'
        && b[10] == b' '
        && b[13] == b':'
        && b[16] == b':'
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .all(|&i| b[i].is_ascii_digit());
    if !shape || s.starts_with("0000") {
        return None;
    }
    Some(format!("{}-{}-{}", &s[..4], &s[5..7], &s[8..19]))
}

/// Camera settings from a TIFF structure: Make, Model and DateTime of
/// IFD0, the rest from the EXIF IFD it points to.
pub(crate) fn camera(t: &Tiff<'_>) -> CameraFacts {
    let mut c = CameraFacts::default();
    let Some((ifd0, _)) = t.first_ifd().and_then(|o| t.ifd(o)) else {
        return c;
    };
    c.make = ascii(t, &ifd0, 0x010F);
    c.model = ascii(t, &ifd0, 0x0110);
    let exif = t
        .get(&ifd0, tag::EXIF_IFD)
        .and_then(|o| t.ifd(o as usize))
        .map(|(e, _)| e);
    if let Some(exif) = &exif {
        c.exposure = rational(t, exif, 0x829A).filter(|&(n, _)| n > 0);
        c.f_number = real(t, exif, 0x829D);
        c.iso = t.get(exif, 0x8827).filter(|&v| v > 0);
        c.focal_length = real(t, exif, 0x920A);
        c.captured = ascii(t, exif, 0x9003)
            .or_else(|| ascii(t, exif, 0x9004))
            .and_then(|s| exif_date(&s));
        c.lens = ascii(t, exif, 0xA434);
    }
    if c.captured.is_none() {
        c.captured = ascii(t, &ifd0, 0x0132).and_then(|s| exif_date(&s));
    }
    c
}

/// Camera settings from an EXIF payload, with or without the `Exif\0\0`
/// prefix.
pub(crate) fn camera_from_exif(data: &[u8]) -> CameraFacts {
    Tiff::from_exif(data)
        .map(|t| camera(&t))
        .unwrap_or_default()
}

/// The color space the EXIF ColorSpace tag declares: only sRGB (1) has a
/// name; 65535 means "see the ICC profile".
pub(crate) fn exif_color_space(data: &[u8]) -> Option<String> {
    let t = Tiff::from_exif(data)?;
    let (ifd0, _) = t.ifd(t.first_ifd()?)?;
    let (exif, _) = t.ifd(t.get(&ifd0, tag::EXIF_IFD)? as usize)?;
    (t.get(&exif, 0xA001)? == 1).then(|| "sRGB".to_string())
}

/// The name of a TIFF Compression code.
pub(crate) fn tiff_compression(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "Uncompressed",
        2 => "CCITT modified Huffman",
        3 => "CCITT Group 3 fax",
        4 => "CCITT Group 4 fax",
        5 => "LZW",
        6 => "JPEG (old style)",
        7 => "JPEG",
        8 | 32946 => "Deflate",
        32773 => "PackBits",
        34712 => "JPEG 2000",
        34887 => "LERC",
        34892 => "Lossy JPEG",
        34925 => "LZMA",
        50000 => "Zstandard",
        50001 => "WebP",
        _ => return None,
    })
}

/// A color space name from CICP color primaries and transfer
/// characteristics (ITU-T H.273), for the common combinations only.
pub(crate) fn cicp_name(primaries: u16, transfer: u16) -> Option<String> {
    let gamut = match primaries {
        1 => "BT.709",
        9 => "BT.2020",
        11 => "DCI-P3",
        12 => "Display P3",
        _ => return None,
    };
    Some(match (primaries, transfer) {
        (1, 13) => "sRGB".to_string(),
        (12, 13) => "Display P3".to_string(),
        (_, 16) => format!("{gamut}, PQ"),
        (_, 18) => format!("{gamut}, HLG"),
        (_, 8) => format!("{gamut}, linear"),
        _ => gamut.to_string(),
    })
}

/// Chroma subsampling from the luma sampling factors relative to chroma.
pub(crate) fn subsampling(h: u8, v: u8) -> Option<&'static str> {
    Some(match (h, v) {
        (1, 1) => "4:4:4",
        (2, 1) => "4:2:2",
        (2, 2) => "4:2:0",
        (1, 2) => "4:4:0",
        (4, 1) => "4:1:1",
        (4, 2) => "4:1:0",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ICC profile with one `desc` tag of the given body.
    fn profile_with_desc(body: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 128];
        p[36..40].copy_from_slice(b"acsp");
        p.extend_from_slice(&1u32.to_be_bytes());
        p.extend_from_slice(b"desc");
        p.extend_from_slice(&144u32.to_be_bytes());
        p.extend_from_slice(&(body.len() as u32).to_be_bytes());
        p.extend_from_slice(body);
        let len = p.len() as u32;
        p[..4].copy_from_slice(&len.to_be_bytes());
        p
    }

    #[test]
    fn icc_descriptions_in_both_versions() {
        let mut v2 = b"desc\0\0\0\0".to_vec();
        v2.extend_from_slice(&12u32.to_be_bytes());
        v2.extend_from_slice(b"Display P3\0\0");
        assert_eq!(
            icc_description(&profile_with_desc(&v2)).as_deref(),
            Some("Display P3")
        );
        let name: Vec<u8> = "sRGB IEC61966-2.1"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let mut v4 = b"mluc\0\0\0\0".to_vec();
        v4.extend_from_slice(&1u32.to_be_bytes());
        v4.extend_from_slice(&12u32.to_be_bytes());
        v4.extend_from_slice(b"enUS");
        v4.extend_from_slice(&(name.len() as u32).to_be_bytes());
        v4.extend_from_slice(&28u32.to_be_bytes());
        v4.extend_from_slice(&name);
        assert_eq!(
            icc_description(&profile_with_desc(&v4)).as_deref(),
            Some("sRGB IEC61966-2.1")
        );
        // Hostile input: cut anywhere, nothing panics.
        let p = profile_with_desc(&v4);
        for cut in 0..p.len() {
            let _ = icc_description(&p[..cut]);
        }
        assert_eq!(icc_description(b"not a profile"), None);
    }

    #[test]
    fn dates_and_names() {
        assert_eq!(
            exif_date("2024:05:01 12:34:56").as_deref(),
            Some("2024-05-01 12:34:56")
        );
        assert_eq!(exif_date("0000:00:00 00:00:00"), None);
        assert_eq!(exif_date("yesterday"), None);
        assert_eq!(text(b"  SONY \0junk").as_deref(), Some("SONY"));
        assert_eq!(text(b"\0"), None);
        assert_eq!(tiff_compression(5), Some("LZW"));
        assert_eq!(cicp_name(1, 13).as_deref(), Some("sRGB"));
        assert_eq!(cicp_name(9, 16).as_deref(), Some("BT.2020, PQ"));
        assert_eq!(subsampling(2, 2), Some("4:2:0"));
    }
}
