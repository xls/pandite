//! The information panel: the facts of the image on screen as label and
//! value rows.
//!
//! [`read`] runs on a job thread. It looks at the file (size and
//! modification time) and parses the header facts of its first
//! [`HEAD_BYTES`] with the matching decoder
//! ([`pandite_core::DecoderProvider::facts`]); no pixel is decoded. [`rows`]
//! turns those facts, the size and animation timing of the decoded image
//! and the decode time into the rows the interface draws. A fact that is
//! not known has no row.

use crate::disk::Disk;
use pandite_core::{CameraFacts, ImageFacts, LoopCount, Registry, Source};
use pandite_ui::InfoRow;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Bytes read from the start of a file for its header facts. Every format
/// pandite reads keeps its headers near the start; a TIFF that stores its
/// directory behind more pixel data than this reports file facts only.
pub const HEAD_BYTES: usize = 16 << 20;

/// What a facts job found out about one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileReport {
    /// Length in bytes.
    pub size: Option<u64>,
    /// Last modification time.
    pub modified: Option<SystemTime>,
    /// Header facts, when the file could be read and a decoder matched.
    pub facts: Option<ImageFacts>,
}

/// Look at `path` and read its header facts. Blocks on the disk: call it
/// on a job thread. A file that is not stored locally (a cloud
/// placeholder) reports its size and time only, so no download starts.
pub fn read(disk: &dyn Disk, registry: &Registry, path: &Path) -> FileReport {
    let Ok(info) = disk.file_info(path) else {
        return FileReport::default();
    };
    let mut report = FileReport {
        size: Some(info.stamp.len),
        modified: info.stamp.modified,
        facts: None,
    };
    if !(info.is_file && info.local) {
        return report;
    }
    let Ok(bytes) = disk.read_head(path, HEAD_BYTES) else {
        return report;
    };
    let source = Source::from_vec(bytes, Some(path.to_path_buf()));
    let Some(decoder) = registry.decoder_for(&source) else {
        return report;
    };
    // A header the decoder rejects still names the format.
    let parsed = catch_unwind(AssertUnwindSafe(|| decoder.facts(&source)));
    report.facts = Some(match parsed {
        Ok(Ok(facts)) => facts,
        _ => {
            let name = decoder.name();
            ImageFacts::named(name.strip_suffix(" image").unwrap_or(name))
        }
    });
    report
}

/// The animation timing of the decoded image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Number of frames.
    pub frames: usize,
    /// Length of one pass through every frame.
    pub duration: Duration,
    /// How often the animation plays.
    pub loops: LoopCount,
}

/// Everything the rows are built from. Each part is optional.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input<'a> {
    /// The file on screen.
    pub path: Option<&'a Path>,
    /// Width and height of the decoded image, upright.
    pub dims: Option<(u32, u32)>,
    /// What the facts job found.
    pub report: Option<&'a FileReport>,
    /// Frame count and timing, for an animation.
    pub animation: Option<Timing>,
    /// How long the full decode took.
    pub decode_time: Option<Duration>,
}

/// The rows of the panel, in display order. Only known facts get a row,
/// and no row has an empty value.
pub fn rows(input: &Input<'_>) -> Vec<InfoRow> {
    let mut out = Rows(Vec::new());
    if let Some(path) = input.path {
        if let Some(name) = path.file_name() {
            out.push("File", name.to_string_lossy().into_owned());
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            out.push("Folder", crate::app::display_path(dir));
        }
    }
    let size = input.report.and_then(|r| r.size);
    if let Some(n) = size {
        out.push("File size", file_size(n));
    }
    if let Some(t) = input.report.and_then(|r| r.modified).and_then(utc_time) {
        out.push("Modified", t);
    }
    let pixels = input.dims.filter(|&(w, h)| w > 0 && h > 0);
    if let Some((w, h)) = pixels {
        out.push("Pixels", format!("{w} x {h}"));
        out.push("Megapixels", megapixels(w, h));
        out.push("Aspect ratio", aspect(w, h));
    }
    let facts = input.report.and_then(|r| r.facts.as_ref());
    if let Some(f) = facts {
        out.opt("Format", &f.container);
        out.opt("Compression", &f.compression);
        out.opt("FourCC", &f.fourcc);
        let channels = match (&f.channels, f.channel_count) {
            (Some(name), Some(n)) => Some(format!("{name} ({n})")),
            (Some(name), None) => Some(name.clone()),
            (None, Some(n)) => Some(n.to_string()),
            (None, None) => None,
        };
        out.opt("Channels", &channels);
        out.opt(
            "Bits per channel",
            &f.bits_per_channel.map(|b| b.to_string()),
        );
        out.opt(
            "Bits per pixel",
            &f.stored_bits_per_pixel().map(|b| b.to_string()),
        );
        out.opt("Chroma subsampling", &f.subsampling);
        out.opt("Color space", &f.color_space);
        out.opt("ICC profile", &f.icc_profile);
    }
    let frames = input.animation.map_or(1, |a| a.frames.max(1)) as u128;
    if let Some((w, h)) = pixels {
        let area = u128::from(w) * u128::from(h) * frames;
        if let Some(bpp) = facts.and_then(ImageFacts::stored_bits_per_pixel) {
            let raw = (area * u128::from(bpp)).div_ceil(8);
            let raw = u64::try_from(raw).unwrap_or(u64::MAX);
            out.push("Uncompressed", human(raw));
            if let Some(n) = size.filter(|&n| n > 0) {
                out.push(
                    "Compression ratio",
                    format!("{}:1", decimal(raw as f64 / n as f64)),
                );
            }
        }
        if let Some(n) = size {
            let bits = n as f64 * 8.0 / area as f64;
            out.push("Bits per pixel on disk", format!("{bits:.2}"));
        }
    }
    if let Some(a) = input.animation {
        out.push("Frames", a.frames.to_string());
        if !a.duration.is_zero() {
            out.push("Duration", format!("{:.2} s", a.duration.as_secs_f64()));
            let fps = a.frames as f64 / a.duration.as_secs_f64();
            out.push("Frame rate", format!("{fps:.1} fps"));
        }
        out.push(
            "Loop count",
            match a.loops {
                LoopCount::Infinite => "Forever".to_string(),
                LoopCount::Finite(1) => "1 play".to_string(),
                LoopCount::Finite(n) => format!("{n} plays"),
            },
        );
    }
    if let Some(d) = input.decode_time {
        out.push("Decode time", milliseconds(d));
    }
    if let Some(f) = facts {
        camera_rows(&mut out, &f.camera);
    }
    out.0
}

/// Rows under construction; empty values are dropped.
struct Rows(Vec<InfoRow>);

impl Rows {
    fn push(&mut self, label: &str, value: String) {
        let value = value.trim();
        if !value.is_empty() {
            self.0.push(InfoRow::new(label, value));
        }
    }

    fn opt(&mut self, label: &str, value: &Option<String>) {
        if let Some(v) = value {
            self.push(label, v.clone());
        }
    }
}

fn camera_rows(out: &mut Rows, c: &CameraFacts) {
    let make = c.make.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let model = c.model.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let camera = match (make, model) {
        // Many models already start with the maker's name.
        (Some(make), Some(model))
            if model
                .to_ascii_lowercase()
                .starts_with(&make.to_ascii_lowercase()) =>
        {
            Some(model.to_string())
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (one, other) => one.or(other).map(str::to_string),
    };
    out.opt("Camera", &camera);
    out.opt("Lens", &c.lens);
    out.opt("Exposure", &c.exposure.and_then(exposure));
    out.opt(
        "Aperture",
        &c.f_number
            .filter(|f| f.is_finite() && *f > 0.0)
            .map(|f| format!("f/{}", decimal(f64::from(f)))),
    );
    out.opt("ISO", &c.iso.map(|v| v.to_string()));
    out.opt(
        "Focal length",
        &c.focal_length
            .filter(|f| f.is_finite() && *f > 0.0)
            .map(|f| format!("{} mm", decimal(f64::from(f)))),
    );
    out.opt("Taken", &c.captured);
}

/// A number with one decimal, without a trailing ".0".
fn decimal(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// `n` with a comma between groups of three digits.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A byte count in the largest unit that keeps it at 1 or more:
/// "512 bytes", "2.4 MB" (units of 1024).
pub fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return format!("{n} bytes");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[unit])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// A file size in human units plus the exact byte count.
pub fn file_size(n: u64) -> String {
    if n < 1024 {
        format!("{n} bytes")
    } else {
        format!("{} ({} bytes)", human(n), grouped(n))
    }
}

/// `t` as "YYYY-MM-DD HH:MM:SS UTC", or `None` before 1970.
pub fn utc_time(t: SystemTime) -> Option<String> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let (y, m, d) = civil_date(i64::try_from(secs / 86_400).ok()?);
    let rem = secs % 86_400;
    Some(format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ))
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Megapixels with the precision small images need.
pub fn megapixels(w: u32, h: u32) -> String {
    let mp = f64::from(w) * f64::from(h) / 1_000_000.0;
    if mp >= 1.0 {
        format!("{mp:.1} MP")
    } else if mp >= 0.01 {
        format!("{mp:.2} MP")
    } else {
        "less than 0.01 MP".to_string()
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The aspect ratio: reduced terms when they are small ("3:2 (1.50:1)"),
/// else the decimal ratio ("1.47:1").
pub fn aspect(w: u32, h: u32) -> String {
    let g = gcd(w, h).max(1);
    let (a, b) = (w / g, h / g);
    let ratio = f64::from(w) / f64::from(h);
    if a <= 32 && b <= 32 {
        if b == 1 {
            format!("{a}:1")
        } else {
            format!("{a}:{b} ({ratio:.2}:1)")
        }
    } else {
        format!("{ratio:.2}:1")
    }
}

/// A duration in milliseconds ("42 ms", "3.4 ms"), or seconds from 10 s.
pub fn milliseconds(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms >= 10_000.0 {
        format!("{:.1} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// An exposure time: "1/250 s" below one second, "2.5 s" from one second.
pub fn exposure((num, den): (u32, u32)) -> Option<String> {
    if num == 0 || den == 0 {
        return None;
    }
    let g = gcd(num, den).max(1);
    let (n, d) = (num / g, den / g);
    if n >= d {
        return Some(format!("{} s", decimal(f64::from(n) / f64::from(d))));
    }
    if n == 1 {
        return Some(format!("1/{d} s"));
    }
    let inverse = (f64::from(d) / f64::from(n)).round();
    Some(format!("1/{inverse:.0} s"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn numbers_read_like_people_write_them() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(2_512_345), "2,512,345");
        assert_eq!(file_size(512), "512 bytes");
        assert_eq!(file_size(2_512_345), "2.4 MB (2,512,345 bytes)");
        assert_eq!(human(150 * 1024), "150 KB");
        assert_eq!(megapixels(6000, 4000), "24.0 MP");
        assert_eq!(megapixels(300, 200), "0.06 MP");
        assert_eq!(megapixels(4, 2), "less than 0.01 MP");
        assert_eq!(aspect(6000, 4000), "3:2 (1.50:1)");
        assert_eq!(aspect(1920, 1080), "16:9 (1.78:1)");
        assert_eq!(aspect(100, 100), "1:1");
        assert_eq!(aspect(1001, 683), "1.47:1");
        assert_eq!(milliseconds(Duration::from_micros(3_400)), "3.4 ms");
        assert_eq!(milliseconds(Duration::from_millis(42)), "42 ms");
        assert_eq!(exposure((1, 250)).as_deref(), Some("1/250 s"));
        assert_eq!(exposure((10, 2500)).as_deref(), Some("1/250 s"));
        assert_eq!(exposure((5, 2)).as_deref(), Some("2.5 s"));
        assert_eq!(exposure((0, 1)), None);
        assert_eq!(decimal(2.8), "2.8");
        assert_eq!(decimal(8.0), "8");
    }

    #[test]
    fn utc_dates_follow_the_calendar() {
        let at = |s: u64| utc_time(UNIX_EPOCH + Duration::from_secs(s)).unwrap();
        assert_eq!(at(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(at(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(at(1_700_000_000), "2023-11-14 22:13:20 UTC");
    }

    #[test]
    fn rows_leave_out_what_is_unknown() {
        let path = PathBuf::from("pics").join("a.png");
        let only_name = rows(&Input {
            path: Some(&path),
            ..Input::default()
        });
        let labels: Vec<&str> = only_name.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["File", "Folder"]);
        let mut facts = ImageFacts::named("PNG");
        facts.camera.make = Some("SONY".into());
        facts.camera.model = Some("SONY ILCE-7M4".into());
        facts.camera.f_number = Some(f32::NAN);
        let report = FileReport {
            size: Some(1000),
            modified: None,
            facts: Some(facts),
        };
        let all = rows(&Input {
            dims: Some((10, 10)),
            report: Some(&report),
            ..Input::default()
        });
        let find = |label: &str| all.iter().find(|r| r.label == label).map(|r| &r.value);
        assert_eq!(find("Format").map(String::as_str), Some("PNG"));
        assert_eq!(find("Camera").map(String::as_str), Some("SONY ILCE-7M4"));
        for missing in [
            "Compression",
            "Channels",
            "Uncompressed",
            "Aperture",
            "Frames",
        ] {
            assert!(find(missing).is_none(), "{missing}");
        }
        assert_eq!(
            find("Bits per pixel on disk").map(String::as_str),
            Some("80.00")
        );
        assert!(all.iter().all(|r| !r.value.trim().is_empty()));
    }
}
