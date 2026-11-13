//! The platform traits implemented with winit, the system clipboard
//! (arboard for text everywhere and for images outside Windows, the
//! Windows clipboard functions for images on Windows) and native file
//! dialogs (rfd).

use crate::platform::{
    Clipboard, DialogKind, DialogReply, Dialogs, Filter, ImageWriter, WindowControl,
};
use crate::settings::MonitorArea;
#[cfg(any(windows, test))]
use pandite_core::ImageBuf;
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

    fn image_writer(&mut self) -> Box<dyn ImageWriter> {
        #[cfg(windows)]
        {
            Box::new(win::ClipboardImage)
        }
        #[cfg(not(windows))]
        {
            // On Linux the clipboard serves what was copied while one
            // clipboard object of the process lives: this one stays open,
            // so the worker may drop its own.
            let _ = self.get();
            Box::new(ArboardImage)
        }
    }
}

/// Puts an image on the clipboard through arboard, which converts it to
/// the formats of the system (PNG on Linux, an NSImage on macOS).
#[cfg(not(windows))]
struct ArboardImage;

#[cfg(not(windows))]
impl ImageWriter for ArboardImage {
    fn set_image(
        self: Box<Self>,
        image: &pandite_core::ImageBuf,
        cancel: &pandite_core::CancelToken,
    ) -> pandite_core::Result<()> {
        cancel.check()?;
        let data = arboard::ImageData {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: std::borrow::Cow::Borrowed(image.data()),
        };
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_image(data))
            .map_err(|e| pandite_core::Error::Backend(e.to_string()))
    }
}

/// Bytes of a BITMAPV5HEADER.
#[cfg(any(windows, test))]
const DIBV5_HEADER: usize = 124;

/// The byte size of the CF_DIBV5 form of a `width` x `height` image, or
/// `None` when its pixels do not fit the 32-bit size field.
#[cfg(any(windows, test))]
fn dibv5_len(width: u32, height: u32) -> Option<usize> {
    let pixels = u64::from(width) * u64::from(height) * 4;
    u32::try_from(pixels).ok()?;
    usize::try_from(pixels).ok()?.checked_add(DIBV5_HEADER)
}

/// Write `image` (RGBA8, top row first) into `out` (exactly
/// [`dibv5_len`] bytes) in the CF_DIBV5 layout: a BITMAPV5HEADER for 32-bit
/// sRGB pixels with an alpha mask, then BGRA rows from the bottom row up
/// (Word and WordPad do not paste top-down bitmaps). Large images are
/// converted in bands on a few threads.
#[cfg(any(windows, test))]
fn fill_dibv5(image: &ImageBuf, out: &mut [u8]) {
    let (w, h) = (image.width(), image.height());
    let (head, pixels) = out.split_at_mut(DIBV5_HEADER);
    let mut header = Vec::with_capacity(DIBV5_HEADER);
    header.extend_from_slice(&(DIBV5_HEADER as u32).to_le_bytes());
    header.extend_from_slice(&(w as i32).to_le_bytes());
    // A positive height: the rows run bottom-up.
    header.extend_from_slice(&(h as i32).to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes()); // planes
    header.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    header.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    header.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    header.extend_from_slice(&[0u8; 16]); // resolution, palette
    for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
        header.extend_from_slice(&mask.to_le_bytes());
    }
    header.extend_from_slice(&0x7352_4742u32.to_le_bytes()); // LCS_sRGB
    header.extend_from_slice(&[0u8; 48]); // endpoints and gamma, unused
    header.extend_from_slice(&4u32.to_le_bytes()); // LCS_GM_IMAGES
    header.extend_from_slice(&[0u8; 12]); // profile, reserved
    head.copy_from_slice(&header);
    let stride = w as usize * 4;
    let rows = h as usize;
    let bands = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8)
        .min(rows.div_ceil(256))
        .max(1);
    let band_rows = rows.div_ceil(bands);
    let convert = |first: usize, band: &mut [u8]| {
        for (r, dst) in band.chunks_exact_mut(stride).enumerate() {
            let src = image.row((rows - 1 - (first + r)) as u32);
            for (d, p) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                d.copy_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
    };
    if bands == 1 {
        convert(0, pixels);
        return;
    }
    std::thread::scope(|s| {
        for (i, band) in pixels.chunks_mut(band_rows * stride).enumerate() {
            let convert = &convert;
            s.spawn(move || convert(i * band_rows, band));
        }
    });
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

/// Turn off the system's animation when the window opens and closes
/// (Windows; nothing elsewhere), so an image is on screen the moment the
/// window shows.
pub fn disable_transitions(window: &Window) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                win::disable_transitions(h.hwnd.get());
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
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
    use crate::platform::ImageWriter;
    use pandite_core::{
        CancelToken, EncodeOptions, EncoderProvider, Error, ImageBuf, Rect, Result as CoreResult,
    };
    use std::marker::PhantomData;
    use std::time::Duration;

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
        fn OpenClipboard(owner: isize) -> i32;
        fn EmptyClipboard() -> i32;
        fn CloseClipboard() -> i32;
        fn SetClipboardData(format: u32, data: isize) -> isize;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
        fn GlobalLock(mem: isize) -> *mut u8;
        fn GlobalUnlock(mem: isize) -> i32;
        fn GlobalFree(mem: isize) -> isize;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateSolidBrush(color: u32) -> isize;
        fn DeleteObject(object: isize) -> i32;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    /// The window attribute that turns its open and close animation off.
    const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;

    /// Turn off the open and close animation of `hwnd`.
    pub fn disable_transitions(hwnd: isize) {
        let on: i32 = 1;
        // SAFETY: `hwnd` is the live window of this process; `on` is a
        // BOOL valid for reads of the size passed. A failure changes
        // nothing.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                std::ptr::from_ref(&on).cast(),
                std::mem::size_of::<i32>() as u32,
            );
        }
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

    /// Movable global memory, the kind the clipboard takes.
    const GMEM_MOVEABLE: u32 = 0x0002;
    /// The clipboard format of a BITMAPV5HEADER bitmap.
    const CF_DIBV5: u32 = 17;
    /// Transparent images up to this many pixels also go on the clipboard
    /// as PNG, which programs that read transparency prefer. Larger ones
    /// would take seconds to compress; their bitmap keeps the alpha.
    const PNG_MAX_PIXELS: u64 = 16_000_000;

    fn failed(what: &str) -> Error {
        Error::Backend(what.to_string())
    }

    /// A movable global memory block, freed on drop unless the clipboard
    /// took it over.
    pub struct Global {
        handle: isize,
        len: usize,
    }

    impl Global {
        /// A new block of `len` bytes.
        pub fn new(len: usize) -> CoreResult<Global> {
            // SAFETY: GlobalAlloc takes plain values and returns a new
            // handle, or 0 when the memory is not there.
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, len.max(1)) };
            if handle == 0 {
                return Err(failed("not enough memory for the clipboard"));
            }
            Ok(Global { handle, len })
        }

        /// Lock the block and let `f` write its `len` bytes.
        pub fn fill<T>(&mut self, f: impl FnOnce(&mut [u8]) -> T) -> CoreResult<T> {
            // SAFETY: `handle` is a live movable block owned by `self`;
            // GlobalLock pins it and returns its address, or null.
            let ptr = unsafe { GlobalLock(self.handle) };
            if ptr.is_null() {
                return Err(failed("cannot lock the clipboard memory"));
            }
            let _unlock = Unlock(self.handle);
            // SAFETY: the block holds at least `len` bytes (it was
            // allocated with that size), stays pinned until `_unlock`
            // drops after `f` returns, and `&mut self` keeps every other
            // access away meanwhile.
            let bytes = unsafe { std::slice::from_raw_parts_mut(ptr, self.len) };
            Ok(f(bytes))
        }

        /// Give the block to the open clipboard as `format`. The
        /// clipboard owns it from then on; on failure it is freed.
        fn hand_over(self, _open: &Opened, format: u32) -> CoreResult<()> {
            // SAFETY: `_open` proves this thread has the clipboard open,
            // and the handle is an unlocked movable block that `self` owns.
            let taken = unsafe { SetClipboardData(format, self.handle) } != 0;
            if !taken {
                return Err(failed("the clipboard did not take the image"));
            }
            std::mem::forget(self);
            Ok(())
        }
    }

    impl Drop for Global {
        fn drop(&mut self) {
            // SAFETY: the block is owned here, unlocked, and nothing else
            // holds its handle.
            unsafe { GlobalFree(self.handle) };
        }
    }

    /// Unlocks a global memory block when dropped.
    struct Unlock(isize);

    impl Drop for Unlock {
        fn drop(&mut self) {
            // SAFETY: balances the GlobalLock of the same handle in
            // `Global::fill`.
            unsafe { GlobalUnlock(self.0) };
        }
    }

    /// The clipboard, open on this thread until dropped. Not `Send`: it
    /// must close on the thread that opened it.
    struct Opened(PhantomData<*const ()>);

    impl Opened {
        /// Open the clipboard, waiting a moment when another program has
        /// it open.
        fn open() -> CoreResult<Opened> {
            for attempt in 0..20u64 {
                // SAFETY: a null owner window is allowed; the call takes no
                // pointers.
                if unsafe { OpenClipboard(0) } != 0 {
                    return Ok(Opened(PhantomData));
                }
                std::thread::sleep(Duration::from_millis(10 + 2 * attempt));
            }
            Err(failed("another program is using the clipboard"))
        }
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            // SAFETY: an `Opened` exists only while this thread has the
            // clipboard open.
            unsafe { CloseClipboard() };
        }
    }

    /// Puts images on the Windows clipboard as CF_DIBV5 (32-bit with
    /// alpha; Windows offers CF_DIB and CF_BITMAP from it), plus PNG for a
    /// transparent image. The pixels are prepared before the clipboard
    /// opens, so other programs are kept out only for the hand-over.
    pub struct ClipboardImage;

    impl ImageWriter for ClipboardImage {
        fn set_image(self: Box<Self>, image: &ImageBuf, cancel: &CancelToken) -> CoreResult<()> {
            let (w, h) = (image.width(), image.height());
            let len = super::dibv5_len(w, h).ok_or(Error::TooLarge {
                width: w,
                height: h,
            })?;
            let mut dib = Global::new(len)?;
            dib.fill(|bytes| super::fill_dibv5(image, bytes))?;
            cancel.check()?;
            let small = u64::from(w) * u64::from(h) <= PNG_MAX_PIXELS;
            let png = if small && image.has_transparency() {
                let mut data = Vec::new();
                pandite_codecs::PngEncoder.encode(image, &EncodeOptions::default(), &mut data)?;
                let mut block = Global::new(data.len())?;
                block.fill(|bytes| bytes.copy_from_slice(&data))?;
                Some(block)
            } else {
                None
            };
            cancel.check()?;
            let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
            // SAFETY: `name` is a NUL-terminated UTF-16 string that lives
            // for the call.
            let png_format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
            let open = Opened::open()?;
            // SAFETY: the clipboard is open on this thread (`open`).
            if unsafe { EmptyClipboard() } == 0 {
                return Err(failed("cannot clear the clipboard"));
            }
            // Some programs take the first format offered: PNG goes first,
            // as other applications do. Without it the bitmap still works.
            if let (Some(block), true) = (png, png_format != 0) {
                let _ = block.hand_over(&open, png_format);
            }
            dib.hand_over(&open, CF_DIBV5)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Global memory alone, without the clipboard: a block takes the
        /// bytes written into it and is freed on drop.
        #[test]
        fn global_block_holds_its_bytes() {
            let mut block = Global::new(1000).unwrap();
            block
                .fill(|b| b.iter_mut().enumerate().for_each(|(i, v)| *v = i as u8))
                .unwrap();
            let sum = block
                .fill(|b| b.iter().map(|&v| u64::from(v)).sum::<u64>())
                .unwrap();
            assert_eq!(sum, (0..1000u64).map(|i| i % 256).sum::<u64>());
            let empty = Global::new(0).unwrap();
            drop(empty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::PixelFormat;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// An image whose every pixel tells its position.
    fn numbered(w: u32, h: u32) -> ImageBuf {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[x as u8, y as u8, (x + y) as u8, 255 - y as u8]);
            }
        }
        ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
    }

    #[test]
    fn dibv5_holds_bottom_up_bgra_with_alpha() {
        // Small and banded sizes take the same layout.
        for (w, h) in [(3, 2), (7, 1500)] {
            let image = numbered(w, h);
            let len = dibv5_len(w, h).unwrap();
            assert_eq!(len, DIBV5_HEADER + (w * h * 4) as usize);
            let mut out = vec![0xAA; len];
            fill_dibv5(&image, &mut out);
            assert_eq!(u32_at(&out, 0), 124, "BITMAPV5HEADER");
            assert_eq!(u32_at(&out, 4), w);
            assert_eq!(u32_at(&out, 8), h, "positive height: bottom-up rows");
            assert_eq!(u32_at(&out, 12), 1 | (32 << 16), "one plane, 32 bits");
            assert_eq!(u32_at(&out, 16), 3, "BI_BITFIELDS");
            assert_eq!(u32_at(&out, 20), w * h * 4, "image size");
            assert_eq!(
                [
                    u32_at(&out, 40),
                    u32_at(&out, 44),
                    u32_at(&out, 48),
                    u32_at(&out, 52)
                ],
                [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
                "channel masks"
            );
            assert_eq!(u32_at(&out, 56), 0x7352_4742, "sRGB");
            assert_eq!(u32_at(&out, 108), 4, "LCS_GM_IMAGES");
            let pixels = &out[DIBV5_HEADER..];
            for y in 0..h {
                let row = (h - 1 - y) as usize;
                for x in 0..w {
                    let at = (row * w as usize + x as usize) * 4;
                    let want = [(x + y) as u8, y as u8, x as u8, 255 - y as u8];
                    assert_eq!(&pixels[at..at + 4], &want, "{w}x{h} pixel {x},{y}");
                }
            }
        }
        // The size field is 32 bits: larger images do not fit.
        assert!(dibv5_len(65_535, 65_535).is_none());
        assert!(dibv5_len(30_000, 30_000).is_some());
    }
}
