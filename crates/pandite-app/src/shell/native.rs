//! The platform traits implemented with winit, the system clipboard
//! (arboard) and native file dialogs (rfd).

use crate::platform::{Clipboard, DialogKind, DialogReply, Dialogs, Filter, WindowControl};
use crate::settings::MonitorArea;
use pandite_core::Rect;
use pandite_ui::CursorShape;
use std::sync::Arc;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::monitor::MonitorHandle;
use winit::window::{CursorIcon, Fullscreen, Window};

/// The system clipboard. Opened on first use and kept open: on Linux the
/// copied text lives as long as the clipboard object.
#[derive(Default)]
pub struct SystemClipboard {
    inner: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn get(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.inner.is_none() {
            self.inner = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        self.inner
            .as_mut()
            .ok_or_else(|| "no clipboard".to_string())
    }
}

impl Clipboard for SystemClipboard {
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        self.get()?
            .set_text(text.to_owned())
            .map_err(|e| e.to_string())
    }

    fn get_text(&mut self) -> Result<String, String> {
        self.get()?.get_text().map_err(|e| e.to_string())
    }
}

/// Native file dialogs, each shown on its own thread so the event loop
/// keeps running and drawing. The dialog is modal to the window where the
/// platform supports it.
pub struct NativeDialogs {
    window: Arc<Window>,
}

impl NativeDialogs {
    /// Dialogs owned by `window`.
    pub fn new(window: Arc<Window>) -> Self {
        Self { window }
    }
}

fn with_filters(mut dialog: rfd::FileDialog, filters: &[Filter]) -> rfd::FileDialog {
    for f in filters {
        dialog = dialog.add_filter(&f.name, &f.extensions);
    }
    dialog
}

impl Dialogs for NativeDialogs {
    fn show(&mut self, kind: DialogKind, reply: DialogReply) {
        let mut dialog = rfd::FileDialog::new().set_parent(&*self.window);
        let start = match &kind {
            DialogKind::OpenFile { start_dir, .. }
            | DialogKind::OpenFiles { start_dir, .. }
            | DialogKind::PickFolder { start_dir, .. }
            | DialogKind::SaveFile { start_dir, .. } => start_dir.clone(),
        };
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        let dialog = match &kind {
            DialogKind::OpenFile { filters, .. } => {
                with_filters(dialog.set_title("Open image"), filters)
            }
            DialogKind::OpenFiles { filters, .. } => {
                with_filters(dialog.set_title("Choose images"), filters)
            }
            DialogKind::PickFolder { title, .. } => dialog.set_title(title),
            DialogKind::SaveFile {
                file_name, filters, ..
            } => with_filters(
                dialog.set_title("Save image as").set_file_name(file_name),
                filters,
            ),
        };
        let run = move || {
            let paths = match kind {
                DialogKind::OpenFile { .. } => dialog.pick_file().into_iter().collect(),
                DialogKind::OpenFiles { .. } => dialog.pick_files().unwrap_or_default(),
                DialogKind::PickFolder { .. } => dialog.pick_folder().into_iter().collect(),
                DialogKind::SaveFile { .. } => dialog.save_file().into_iter().collect(),
            };
            reply(paths);
        };
        if let Err(err) = std::thread::Builder::new()
            .name("pandite-dialog".into())
            .spawn(run)
        {
            crate::console::err(&format!("pandite: cannot show a dialog: {err}"));
        }
    }
}

/// The application window.
pub struct NativeWindow {
    window: Arc<Window>,
}

impl NativeWindow {
    /// Control `window`.
    pub fn new(window: Arc<Window>) -> Self {
        Self { window }
    }
}

fn cursor_icon(shape: CursorShape) -> CursorIcon {
    match shape {
        CursorShape::Default | CursorShape::Hidden => CursorIcon::Default,
        CursorShape::Hand => CursorIcon::Pointer,
        CursorShape::Text => CursorIcon::Text,
        CursorShape::Grab => CursorIcon::Grab,
        CursorShape::Grabbing => CursorIcon::Grabbing,
        CursorShape::Crosshair => CursorIcon::Crosshair,
        CursorShape::Move => CursorIcon::Move,
        CursorShape::ResizeHorizontal => CursorIcon::EwResize,
        CursorShape::ResizeVertical => CursorIcon::NsResize,
        CursorShape::ResizeDiagonalDown => CursorIcon::NwseResize,
        CursorShape::ResizeDiagonalUp => CursorIcon::NeswResize,
    }
}

/// Offset from the outer frame's top-left to the client area's.
fn client_offset(window: &Window) -> (i32, i32) {
    match (window.inner_position(), window.outer_position()) {
        (Ok(inner), Ok(outer)) => (inner.x - outer.x, inner.y - outer.y),
        _ => (0, 0),
    }
}

/// Move and size `window` so its client area covers `rect`.
pub fn place_client(window: &Window, rect: Rect) {
    let (dx, dy) = client_offset(window);
    window.set_outer_position(PhysicalPosition::new(
        rect.x.round() as i32 - dx,
        rect.y.round() as i32 - dy,
    ));
    let _ = window.request_inner_size(PhysicalSize::new(
        rect.w.round().max(1.0) as u32,
        rect.h.round().max(1.0) as u32,
    ));
}

impl WindowControl for NativeWindow {
    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    fn set_fullscreen(&self, fullscreen: bool) {
        self.window
            .set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
    }

    fn set_cursor(&self, cursor: CursorShape) {
        self.window
            .set_cursor_visible(cursor != CursorShape::Hidden);
        self.window.set_cursor(cursor_icon(cursor));
    }

    fn set_ime_allowed(&self, allowed: bool) {
        self.window.set_ime_allowed(allowed);
    }

    fn set_client_rect(&self, rect: Rect) {
        if self.window.is_maximized() || self.window.fullscreen().is_some() {
            return;
        }
        place_client(&self.window, rect);
    }

    fn work_area(&self) -> Option<Rect> {
        let monitor = self
            .window
            .current_monitor()
            .or_else(|| self.window.primary_monitor())?;
        Some(monitor_area(&monitor).work)
    }
}

/// The bounds and work area of a monitor in physical pixels.
pub fn monitor_area(monitor: &MonitorHandle) -> MonitorArea {
    let pos = monitor.position();
    let size = monitor.size();
    let bounds = Rect::new(
        pos.x as f32,
        pos.y as f32,
        size.width as f32,
        size.height as f32,
    );
    MonitorArea {
        bounds,
        work: work_area(monitor, bounds),
    }
}

#[cfg(windows)]
fn work_area(monitor: &MonitorHandle, bounds: Rect) -> Rect {
    use winit::platform::windows::MonitorHandleExtWindows;
    win::work_area(monitor.hmonitor()).unwrap_or(bounds)
}

#[cfg(not(windows))]
fn work_area(monitor: &MonitorHandle, bounds: Rect) -> Rect {
    // winit does not report task bars and docks elsewhere: keep clear of
    // the usual menu bar and dock or panel sizes.
    let s = monitor.scale_factor() as f32;
    let (top, bottom) = if cfg!(target_os = "macos") {
        (25.0 * s, 70.0 * s)
    } else {
        (32.0 * s, 48.0 * s)
    };
    Rect::new(
        bounds.x,
        bounds.y + top,
        bounds.w,
        (bounds.h - top - bottom).max(bounds.h * 0.5),
    )
}

/// Paint the window's client area in `color` until the first frame
/// arrives (and under live resizing): the window class background brush on
/// Windows. Other platforms show nothing before the first frame, because
/// the window appears only with it.
pub fn paint_background(window: &Window, color: pandite_core::Color) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                win::paint_background(h.hwnd.get(), color);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (window, color);
}

/// Every monitor, the primary one first.
pub fn monitors(
    primary: Option<MonitorHandle>,
    all: impl Iterator<Item = MonitorHandle>,
) -> Vec<MonitorArea> {
    let mut out = Vec::new();
    if let Some(p) = &primary {
        out.push(monitor_area(p));
    }
    for m in all {
        if primary.as_ref() != Some(&m) {
            out.push(monitor_area(&m));
        }
    }
    out
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use pandite_core::Rect;

    #[repr(C)]
    #[derive(Default)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MonitorInfo {
        cb_size: u32,
        monitor: WinRect,
        work: WinRect,
        flags: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
        #[cfg(target_pointer_width = "64")]
        fn SetClassLongPtrW(hwnd: isize, index: i32, value: isize) -> usize;
        #[cfg(not(target_pointer_width = "64"))]
        fn SetClassLongW(hwnd: isize, index: i32, value: i32) -> u32;
        fn InvalidateRect(hwnd: isize, rect: *const std::ffi::c_void, erase: i32) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateSolidBrush(color: u32) -> isize;
        fn DeleteObject(object: isize) -> i32;
    }

    /// Index of the background brush in the window class.
    const GCLP_HBRBACKGROUND: i32 = -10;

    /// Make `color` the class background brush of `hwnd` and repaint.
    pub fn paint_background(hwnd: isize, color: pandite_core::Color) {
        let rgb = u32::from(color.r) | (u32::from(color.g) << 8) | (u32::from(color.b) << 16);
        // SAFETY: CreateSolidBrush takes a plain color value and returns a
        // new GDI handle or 0.
        let brush = unsafe { CreateSolidBrush(rgb) };
        if brush == 0 {
            return;
        }
        // SAFETY: `hwnd` is the live window of this process (it came from
        // its window handle); the class owns `brush` from now on.
        #[cfg(target_pointer_width = "64")]
        let old = unsafe { SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, brush) } as isize;
        // SAFETY: as above, for 32-bit Windows.
        #[cfg(not(target_pointer_width = "64"))]
        let old = unsafe { SetClassLongW(hwnd, GCLP_HBRBACKGROUND, brush as i32) } as isize;
        if old != 0 {
            // SAFETY: the previous brush was created here and is no longer
            // used by the class.
            unsafe { DeleteObject(old) };
        }
        // SAFETY: a null rectangle means the whole client area.
        unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
    }

    /// The work area of a monitor handle.
    pub fn work_area(hmonitor: isize) -> Option<Rect> {
        let mut info = MonitorInfo {
            cb_size: std::mem::size_of::<MonitorInfo>() as u32,
            ..MonitorInfo::default()
        };
        // SAFETY: `info` is a properly sized MONITORINFO with cbSize set,
        // valid for writes during the call; an invalid handle makes the
        // call fail without touching memory.
        let ok = unsafe { GetMonitorInfoW(hmonitor, &mut info) } != 0;
        let w = info.work;
        (ok && w.right > w.left && w.bottom > w.top).then(|| {
            Rect::new(
                w.left as f32,
                w.top as f32,
                (w.right - w.left) as f32,
                (w.bottom - w.top) as f32,
            )
        })
    }
}
