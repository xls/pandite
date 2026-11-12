//! Console output for a program built for the Windows GUI subsystem.
//!
//! The release binary opens no console window. When a parent process
//! passes pipes or files as standard handles (a test oracle, a script with
//! redirection) printing works as usual. When it was started from an
//! interactive console without redirection, [`attach`] connects to that
//! console so `--help`, `--version` and smoke results are visible. Other
//! platforms need nothing.

use std::io::Write;

/// Make standard output and error reach the parent console when they are
/// not connected to anything. Safe to call more than once.
pub fn attach() {
    #[cfg(windows)]
    win::attach();
}

/// Print a line to standard output and flush it.
pub fn out(line: &str) {
    attach();
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

/// Print a line to standard error and flush it.
pub fn err(line: &str) {
    attach();
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{line}");
    let _ = stderr.flush();
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::ffi::c_void;
    use std::sync::Once;

    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(std_handle: u32) -> *mut c_void;
        fn SetStdHandle(std_handle: u32, handle: *mut c_void) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
    }

    fn missing(handle: *mut c_void) -> bool {
        handle.is_null() || handle as isize == INVALID_HANDLE_VALUE
    }

    pub fn attach() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            // SAFETY: GetStdHandle only reads the process's handle table.
            let (out, err) = unsafe {
                (
                    GetStdHandle(STD_OUTPUT_HANDLE),
                    GetStdHandle(STD_ERROR_HANDLE),
                )
            };
            if !missing(out) && !missing(err) {
                // A parent gave us pipes or files: keep them.
                return;
            }
            // SAFETY: AttachConsole has no memory preconditions; it fails
            // harmlessly when the parent has no console.
            if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
                return;
            }
            let name: Vec<u16> = "CONOUT$".encode_utf16().chain(Some(0)).collect();
            for (id, current) in [(STD_OUTPUT_HANDLE, out), (STD_ERROR_HANDLE, err)] {
                if !missing(current) {
                    continue;
                }
                // SAFETY: `name` is a NUL-terminated UTF-16 string that
                // outlives the call; the other pointers may be null.
                let handle = unsafe {
                    CreateFileW(
                        name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        std::ptr::null_mut(),
                        OPEN_EXISTING,
                        0,
                        std::ptr::null_mut(),
                    )
                };
                if !missing(handle) {
                    // SAFETY: `handle` is a valid console handle we own for
                    // the rest of the process.
                    unsafe { SetStdHandle(id, handle) };
                }
            }
        });
    }
}
