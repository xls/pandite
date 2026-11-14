//! User settings: a small `key = value` text file in the per-user
//! configuration folder, plus the window placement math that uses it.
//!
//! - Windows: `%APPDATA%\pandite\settings.ini` (inside an MSIX package the
//!   system redirects this to the package's private folder).
//! - macOS: `~/Library/Application Support/pandite/settings.ini`.
//! - Linux and other Unix: `$XDG_CONFIG_HOME/pandite/settings.ini`, or
//!   `~/.config/pandite/settings.ini`.
//!
//! Loading and saving touch the disk and run on worker threads. A file that
//! does not parse is renamed to `settings.ini.bad` and the defaults are
//! used; the program never fails because of its settings. Writes are
//! atomic (see [`crate::disk::write_atomic`]).

use crate::disk::Disk;
use pandite_core::geom::initial_window_rect;
use pandite_core::{Rect, Size};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Name of the settings file inside the configuration folder.
pub const FILE_NAME: &str = "settings.ini";

/// Name the settings file gets when it is set aside as unreadable.
pub const BAD_SUFFIX: &str = ".bad";

/// Folder name under the per-user configuration folder.
pub const APP_DIR: &str = "pandite";

/// Which colors the interface uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeChoice {
    /// Follow the operating system setting (dark when unknown).
    #[default]
    System,
    /// Always dark.
    Dark,
    /// Always light.
    Light,
}

impl ThemeChoice {
    fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "system" => Some(ThemeChoice::System),
            "dark" => Some(ThemeChoice::Dark),
            "light" => Some(ThemeChoice::Light),
            _ => None,
        }
    }
}

/// Whether the viewer reads text in images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextRecognition {
    /// Read the text of the image on screen when the platform has a
    /// recognizer.
    #[default]
    Auto,
    /// Never read text.
    Off,
}

impl TextRecognition {
    fn as_str(self) -> &'static str {
        match self {
            TextRecognition::Auto => "auto",
            TextRecognition::Off => "off",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "auto" => Some(TextRecognition::Auto),
            "off" => Some(TextRecognition::Off),
            _ => None,
        }
    }
}

/// Where the window was, in physical pixels of the virtual desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowPlacement {
    /// Left edge of the outer window frame.
    pub x: i32,
    /// Top edge of the outer window frame.
    pub y: i32,
    /// Width of the client area.
    pub width: u32,
    /// Height of the client area.
    pub height: u32,
    /// The window was maximized (the rectangle is the restored one).
    pub maximized: bool,
    /// The window was in full screen (the rectangle is the restored one).
    pub fullscreen: bool,
}

impl WindowPlacement {
    /// The rectangle as floats.
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        )
    }
}

/// Everything pandite remembers between runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The last window placement, `None` before the first run ends.
    pub window: Option<WindowPlacement>,
    /// Show the gallery strip.
    pub gallery_visible: bool,
    /// Show the information panel (`info_panel = on | off`).
    pub info_panel: bool,
    /// Interface colors.
    pub theme: ThemeChoice,
    /// Share of the screen (0.1 to 1.0 on each axis) a first-open window
    /// may take while it best fits the image.
    pub first_open_fraction: f32,
    /// Size the window to the opened image on every launch, not only the
    /// first one.
    pub size_window_to_image: bool,
    /// Memory budget for decoded images, in MiB.
    pub image_cache_mb: u32,
    /// Memory budget for gallery thumbnails, in MiB.
    pub thumbnail_cache_mb: u32,
    /// Images decoded ahead on each side of the current one.
    pub read_ahead: u32,
    /// Next on the last image goes to the first one, and back.
    pub wrap_around: bool,
    /// JPEG quality for Save As, 1 to 100.
    pub jpeg_quality: u8,
    /// Text recognition (`ocr = auto | off`).
    pub ocr: TextRecognition,
    /// Windows: when the window closes, stay in the background without
    /// any image in memory, so the next image opens at once.
    pub stay_ready: bool,
    /// The last start drew with a discrete GPU. The next start then waits
    /// a moment for it when the system leaves it out of the adapter list.
    pub gpu_discrete_seen: bool,
    /// The folder chosen for the last Move to, where its folder picker
    /// starts the next time.
    pub last_move_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window: None,
            gallery_visible: true,
            info_panel: false,
            theme: ThemeChoice::System,
            first_open_fraction: 0.8,
            size_window_to_image: false,
            image_cache_mb: 512,
            thumbnail_cache_mb: 64,
            read_ahead: 2,
            wrap_around: true,
            jpeg_quality: 90,
            ocr: TextRecognition::Auto,
            stay_ready: true,
            gpu_discrete_seen: false,
            last_move_dir: None,
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_num<T: std::str::FromStr>(v: &str) -> Option<T> {
    v.parse().ok()
}

impl Settings {
    /// The file text. Human readable; comments explain each key.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# pandite settings. Delete this file to reset.");
        if let Some(w) = &self.window {
            let _ = writeln!(s, "\n# Window placement in physical pixels.");
            let _ = writeln!(s, "window_x = {}", w.x);
            let _ = writeln!(s, "window_y = {}", w.y);
            let _ = writeln!(s, "window_width = {}", w.width);
            let _ = writeln!(s, "window_height = {}", w.height);
            let _ = writeln!(s, "window_maximized = {}", w.maximized);
            let _ = writeln!(s, "window_fullscreen = {}", w.fullscreen);
        }
        let _ = writeln!(s, "\n# Interface.");
        let _ = writeln!(s, "gallery_visible = {}", self.gallery_visible);
        let _ = writeln!(s, "# The information panel (key I): on or off.");
        let _ = writeln!(
            s,
            "info_panel = {}",
            if self.info_panel { "on" } else { "off" }
        );
        let _ = writeln!(s, "# system, dark or light");
        let _ = writeln!(s, "theme = {}", self.theme.as_str());
        let _ = writeln!(
            s,
            "\n# Share of the screen a new window may take (0.1 to 1.0)."
        );
        let _ = writeln!(s, "first_open_fraction = {}", self.first_open_fraction);
        let _ = writeln!(s, "# Size the window to the image on every launch.");
        let _ = writeln!(s, "size_window_to_image = {}", self.size_window_to_image);
        let _ = writeln!(s, "\n# Memory and read-ahead.");
        let _ = writeln!(s, "image_cache_mb = {}", self.image_cache_mb);
        let _ = writeln!(s, "thumbnail_cache_mb = {}", self.thumbnail_cache_mb);
        let _ = writeln!(s, "read_ahead = {}", self.read_ahead);
        let _ = writeln!(s, "wrap_around = {}", self.wrap_around);
        let _ = writeln!(s, "\n# Save As.");
        let _ = writeln!(s, "jpeg_quality = {}", self.jpeg_quality);
        let _ = writeln!(
            s,
            "\n# Read text in images so it can be selected and copied: auto or off."
        );
        let _ = writeln!(s, "ocr = {}", self.ocr.as_str());
        let _ = writeln!(
            s,
            "\n# Windows: stay in the background when the window closes (no image is"
        );
        let _ = writeln!(
            s,
            "# kept in memory), so the next image opens at once. pandite --quit ends it."
        );
        let _ = writeln!(s, "stay_ready = {}", self.stay_ready);
        let _ = writeln!(
            s,
            "
# Written by pandite: the last start used a discrete GPU. When true,
# a start waits up to half a second for that GPU to be listed."
        );
        let _ = writeln!(s, "gpu_discrete_seen = {}", self.gpu_discrete_seen);
        // A path that cannot be one line of text is not kept.
        let dir = self.last_move_dir.as_ref().and_then(|d| d.to_str());
        if let Some(dir) = dir.filter(|d| !d.contains(['\n', '\r']) && d.trim() == *d) {
            let _ = writeln!(s, "\n# Written by pandite: the folder of the last Move to.");
            let _ = writeln!(s, "last_move_dir = {dir}");
        }
        s
    }

    /// Parse the file text. Unknown keys are ignored and missing keys keep
    /// their defaults, so files from older and newer versions load. A line
    /// that is not `key = value`, a repeated key or a value that does not
    /// fit its key makes the whole file invalid.
    pub fn parse(text: &str) -> Result<Settings, String> {
        let mut s = Settings::default();
        let mut window = [None::<i64>; 4];
        let mut maximized = false;
        let mut fullscreen = false;
        let mut seen = std::collections::HashSet::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("line {}: expected key = value", n + 1));
            };
            let (key, value) = (key.trim(), value.trim());
            if !seen.insert(key.to_string()) {
                return Err(format!("line {}: {key} appears twice", n + 1));
            }
            let bad = || format!("line {}: bad value for {key}: {value}", n + 1);
            match key {
                "window_x" => window[0] = Some(parse_num(value).ok_or_else(bad)?),
                "window_y" => window[1] = Some(parse_num(value).ok_or_else(bad)?),
                "window_width" => window[2] = Some(parse_num(value).ok_or_else(bad)?),
                "window_height" => window[3] = Some(parse_num(value).ok_or_else(bad)?),
                "window_maximized" => maximized = parse_bool(value).ok_or_else(bad)?,
                "window_fullscreen" => fullscreen = parse_bool(value).ok_or_else(bad)?,
                "gallery_visible" => s.gallery_visible = parse_bool(value).ok_or_else(bad)?,
                "info_panel" => s.info_panel = parse_bool(value).ok_or_else(bad)?,
                "theme" => s.theme = ThemeChoice::parse(value).ok_or_else(bad)?,
                "first_open_fraction" => {
                    let f: f32 = parse_num(value).ok_or_else(bad)?;
                    if !(0.1..=1.0).contains(&f) {
                        return Err(bad());
                    }
                    s.first_open_fraction = f;
                }
                "size_window_to_image" => {
                    s.size_window_to_image = parse_bool(value).ok_or_else(bad)?;
                }
                "image_cache_mb" => {
                    s.image_cache_mb = parse_num::<u32>(value)
                        .filter(|v| (16..=65_536).contains(v))
                        .ok_or_else(bad)?;
                }
                "thumbnail_cache_mb" => {
                    s.thumbnail_cache_mb = parse_num::<u32>(value)
                        .filter(|v| (4..=8192).contains(v))
                        .ok_or_else(bad)?;
                }
                "read_ahead" => {
                    s.read_ahead = parse_num::<u32>(value)
                        .filter(|v| *v <= 16)
                        .ok_or_else(bad)?;
                }
                "wrap_around" => s.wrap_around = parse_bool(value).ok_or_else(bad)?,
                "stay_ready" => s.stay_ready = parse_bool(value).ok_or_else(bad)?,
                "jpeg_quality" => {
                    s.jpeg_quality = parse_num::<u8>(value)
                        .filter(|v| (1..=100).contains(v))
                        .ok_or_else(bad)?;
                }
                "ocr" => s.ocr = TextRecognition::parse(value).ok_or_else(bad)?,
                "gpu_discrete_seen" => s.gpu_discrete_seen = parse_bool(value).ok_or_else(bad)?,
                "last_move_dir" => {
                    s.last_move_dir = (!value.is_empty()).then(|| PathBuf::from(value));
                }
                _ => {}
            }
        }
        match window {
            [Some(x), Some(y), Some(w), Some(h)] => {
                let ok = |v: i64| (-1_000_000..=1_000_000).contains(&v);
                if !(ok(x) && ok(y) && (1..=100_000).contains(&w) && (1..=100_000).contains(&h)) {
                    return Err("window placement is out of range".into());
                }
                s.window = Some(WindowPlacement {
                    x: x as i32,
                    y: y as i32,
                    width: w as u32,
                    height: h as u32,
                    maximized,
                    fullscreen,
                });
            }
            [None, None, None, None] => {}
            _ => return Err("window placement is incomplete".into()),
        }
        Ok(s)
    }
}

/// How loading the settings file went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadNote {
    /// The file was read.
    Loaded,
    /// There is no file yet (first run).
    Missing,
    /// The file could not be read or parsed; defaults are used. `kept` is
    /// where the bad file was moved, when that worked.
    Corrupt {
        /// What was wrong.
        reason: String,
        /// The bad file's new name.
        kept: Option<PathBuf>,
    },
}

/// Load settings from `path`. Never fails: a missing file gives the
/// defaults, an unreadable or invalid one is renamed aside and gives the
/// defaults. Call on a worker thread.
pub fn load(disk: &dyn Disk, path: &Path) -> (Settings, LoadNote) {
    let bytes = match disk.read_small(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (Settings::default(), LoadNote::Missing);
        }
        Err(err) => {
            return (
                Settings::default(),
                LoadNote::Corrupt {
                    reason: format!("cannot read: {err}"),
                    kept: None,
                },
            );
        }
    };
    let parsed = std::str::from_utf8(&bytes)
        .map_err(|_| "not UTF-8 text".to_string())
        .and_then(Settings::parse);
    match parsed {
        Ok(settings) => (settings, LoadNote::Loaded),
        Err(reason) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(BAD_SUFFIX);
            let bad = PathBuf::from(bad);
            let kept = disk.rename(path, &bad).ok().map(|()| bad);
            (Settings::default(), LoadNote::Corrupt { reason, kept })
        }
    }
}

/// Write `settings` to `path` atomically, creating the folder. Call on a
/// worker thread.
pub fn save(disk: &dyn Disk, path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        disk.create_dir_all(dir)?;
    }
    disk.write_atomic(path, settings.to_text().as_bytes())
}

/// The per-user configuration folder for pandite, from the environment.
/// Pure: reads environment variables only.
pub fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env("PANDITE_CONFIG_DIR") {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env("APPDATA").or_else(|| env("USERPROFILE").map(|h| h.join("AppData").join("Roaming")))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| env("HOME").map(|h| h.join(".config")))
    }?;
    Some(base.join(APP_DIR))
}

/// The settings file path: in `override_dir` when given, else in
/// [`config_dir`].
pub fn settings_path(override_dir: Option<&Path>) -> Option<PathBuf> {
    match override_dir {
        Some(dir) => Some(dir.join(FILE_NAME)),
        None => config_dir().map(|d| d.join(FILE_NAME)),
    }
}

/// One display, in physical pixels of the virtual desktop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorArea {
    /// The whole monitor.
    pub bounds: Rect,
    /// The part not covered by task bars and docks.
    pub work: Rect,
}

/// Height of the strip along a window's top edge that must stay reachable
/// on some monitor (the title bar).
const GRAB_STRIP: f32 = 32.0;
/// Width of that strip that must be visible.
const GRAB_WIDTH: f32 = 96.0;

/// Move a saved placement back onto the screen when its monitor is gone.
///
/// The placement is kept when the top strip of the window (where the title
/// bar is) overlaps some monitor's work area by at least a grab-sized
/// piece. Otherwise the window moves to the monitor it overlaps most (the
/// first monitor, normally the primary, when it overlaps none), shrinks to
/// fit its work area and is centered there. `monitors` lists the primary
/// monitor first. An empty list keeps the placement.
pub fn ensure_on_screen(placement: WindowPlacement, monitors: &[MonitorArea]) -> WindowPlacement {
    if monitors.is_empty() {
        return placement;
    }
    let rect = placement.rect();
    let strip = Rect::new(rect.x, rect.y, rect.w, GRAB_STRIP.min(rect.h.max(1.0)));
    let reachable = monitors.iter().any(|m| {
        m.work
            .intersect(strip)
            .is_some_and(|i| i.w >= GRAB_WIDTH.min(rect.w) && i.h >= strip.h.min(8.0))
    });
    if reachable {
        return placement;
    }
    let area = |m: &MonitorArea| m.bounds.intersect(rect).map_or(0.0, |i| i.w * i.h);
    let target = monitors
        .iter()
        .copied()
        .fold(None::<MonitorArea>, |best, m| match best {
            Some(b) if area(&b) >= area(&m) => Some(b),
            _ => Some(m),
        })
        .filter(|m| area(m) > 0.0)
        .unwrap_or(monitors[0]);
    let w = rect.w.min(target.work.w).max(1.0);
    let h = rect.h.min(target.work.h).max(1.0);
    let placed = target.work.centered(Size::new(w, h));
    WindowPlacement {
        x: placed.x.round() as i32,
        y: placed.y.round() as i32,
        width: w.round() as u32,
        height: h.round() as u32,
        ..placement
    }
}

/// The client rectangle for a window opened on `work` (a monitor work
/// area): the image best fitted inside `fraction` of the area, never
/// enlarged above 1:1 and never smaller than `min`, centered, with `extra`
/// added below the image for the gallery strip. `image` `None` (size not
/// known yet) gives the full fraction-sized region.
pub fn first_open_rect(
    work: Rect,
    image: Option<(u32, u32)>,
    fraction: f32,
    extra: f32,
    min: Size,
) -> Rect {
    let extra = extra.max(0.0);
    let image_area = Rect::new(work.x, work.y, work.w, (work.h - extra).max(1.0));
    let size = image.map_or(Size::default(), |(w, h)| Size::new(w as f32, h as f32));
    let mut rect = initial_window_rect(image_area, size, fraction, min);
    if image.is_some() {
        rect.h = (rect.h + extra).min(work.h);
    }
    let centered = work.centered(rect.size());
    Rect::new(
        centered.x.round(),
        centered.y.round(),
        rect.w.round(),
        rect.h.round(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip_keeps_every_field() {
        let s = Settings {
            window: Some(WindowPlacement {
                x: -1200,
                y: 40,
                width: 1600,
                height: 900,
                maximized: true,
                fullscreen: false,
            }),
            gallery_visible: false,
            info_panel: true,
            theme: ThemeChoice::Light,
            first_open_fraction: 0.65,
            size_window_to_image: true,
            image_cache_mb: 1024,
            thumbnail_cache_mb: 128,
            read_ahead: 3,
            wrap_around: false,
            jpeg_quality: 77,
            ocr: TextRecognition::Off,
            stay_ready: false,
            gpu_discrete_seen: true,
            last_move_dir: Some(PathBuf::from("D:/Photos/kept = best")),
        };
        assert_eq!(Settings::parse(&s.to_text()).unwrap(), s);
        // A folder that cannot be one line of text is not kept.
        let odd = Settings {
            last_move_dir: Some(PathBuf::from("/tmp/two\nlines")),
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&odd.to_text()).unwrap().last_move_dir, None);
        assert_eq!(
            Settings::parse(&Settings::default().to_text()).unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn parse_accepts_unknown_keys_and_rejects_garbage() {
        let s = Settings::parse("future_key = 3\n  theme = DARK  \n").unwrap();
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(Settings::parse("theme dark").is_err());
        assert!(Settings::parse("theme = purple").is_err());
        assert!(Settings::parse("first_open_fraction = 2").is_err());
        assert!(Settings::parse("window_x = 1\nwindow_y = 2").is_err());
        assert!(Settings::parse("theme = dark\ntheme = light").is_err());
        assert_eq!(
            Settings::parse("ocr = OFF").unwrap().ocr,
            TextRecognition::Off
        );
        assert_eq!(Settings::parse("").unwrap().ocr, TextRecognition::Auto);
        assert!(Settings::parse("ocr = maybe").is_err());
    }

    #[test]
    fn first_open_rect_adds_the_gallery_and_centers() {
        let work = Rect::new(0.0, 0.0, 1920.0, 1040.0);
        let min = Size::new(400.0, 300.0);
        let r = first_open_rect(work, Some((4000, 1000)), 0.8, 100.0, min);
        assert_eq!(r.w, 1536.0);
        assert_eq!(r.h, 384.0 + 100.0);
        assert_eq!(r.center(), work.center());
        let r = first_open_rect(work, None, 0.5, 100.0, min);
        assert_eq!((r.w, r.h), (960.0, 470.0));
    }
}
