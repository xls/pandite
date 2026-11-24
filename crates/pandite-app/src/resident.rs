//! The resident program: one pandite process that stays in the background
//! with its GPU device open, so the next image opens without the start-up
//! cost.
//!
//! The first pandite process of a user session listens on a named pipe.
//! When its window closes it does not exit: it frees every image, hides
//! the window and waits (see `shell`). A later `pandite <image>` connects
//! to the pipe, sends the path and exits; the resident shows the image.
//! A resident that shows a window shows the image in that window, so a
//! user session has one pandite window. `busy` answers only text that is
//! no request. A process started with `--new-instance` does not ask and
//! does not listen: it opens its own window.
//!
//! The request is three UTF-8 lines: `pandite <version>`, a verb (`open`,
//! `ping` or `quit`) and the verb's argument (`<launch time> <path>` for
//! `open`). The answer is one line: `ok`, `busy` or `stale`. A resident of
//! another version or another build answers `stale` and exits, so an
//! updated program never talks to an old one; it starts as usual and
//! becomes the resident.
//!
//! While it waits the resident holds no image and no texture. It keeps the
//! thumbnails of the last folder for 30 minutes, inside the thumbnail
//! cache limit, and the GPU device open. With `stay_ready = false` in the
//! settings file the first process still listens while its window is
//! open, but exits when the window closes. `pandite --quit` ends a
//! resident.
//!
//! Windows only. On other platforms nothing listens and every process
//! opens its own window.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What a later process asks the resident for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Show the window with this image (or empty).
    Open {
        /// The file to open, absolute.
        path: Option<PathBuf>,
        /// When the asking process started.
        launched: Option<SystemTime>,
    },
    /// Is a resident there? Changes nothing.
    Ping,
    /// Exit.
    Quit,
}

/// The resident's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Done.
    Ok,
    /// The text was no request: the asking process opens its own window.
    Busy,
    /// The resident is another version and exits.
    Stale,
}

// The pipe is the only user of the reply text.
#[cfg(any(windows, test))]
impl Reply {
    fn text(self) -> &'static str {
        match self {
            Reply::Ok => "ok",
            Reply::Busy => "busy",
            Reply::Stale => "stale",
        }
    }

    fn parse(text: &str) -> Option<Reply> {
        match text.trim() {
            "ok" => Some(Reply::Ok),
            "busy" => Some(Reply::Busy),
            "stale" => Some(Reply::Stale),
            _ => None,
        }
    }
}

/// The first line of every request: the version and, where the system
/// tells, the size and date of the program file, so a rebuilt or updated
/// program of the same version also replaces the one that waits.
fn greeting() -> String {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD
        .get_or_init(|| {
            let file = std::env::current_exe()
                .and_then(std::fs::metadata)
                .map(|m| {
                    let date = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs());
                    format!("{}-{date}", m.len())
                })
                .unwrap_or_default();
            format!("pandite {} {file}", env!("CARGO_PKG_VERSION"))
        })
        .clone()
}

fn millis(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

/// The request as text.
pub fn encode(request: &Request) -> Option<String> {
    let (verb, arg) = match request {
        Request::Open { path, launched } => {
            let path = match path {
                // A name that is not Unicode cannot travel as text.
                Some(p) => p.to_str()?.to_string(),
                None => String::new(),
            };
            if path.contains(['\n', '\r']) {
                return None;
            }
            let at = launched.map_or(0, millis);
            ("open", format!("{at} {path}"))
        }
        Request::Ping => ("ping", String::new()),
        Request::Quit => ("quit", String::new()),
    };
    Some(format!("{}\n{verb}\n{arg}\n", greeting()))
}

/// Read a request. `Err(Reply::Stale)` for another version, `Err(Reply::Busy)`
/// for text that is no request.
pub fn decode(text: &str) -> Result<Request, Reply> {
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let first = lines.next().unwrap_or("");
    if first != greeting() {
        return Err(if first.starts_with("pandite ") {
            Reply::Stale
        } else {
            Reply::Busy
        });
    }
    let verb = lines.next().unwrap_or("");
    let arg = lines.next().unwrap_or("");
    match verb {
        "open" => {
            let (at, path) = arg.split_once(' ').unwrap_or((arg, ""));
            let launched = at
                .parse::<u64>()
                .ok()
                .filter(|&ms| ms > 0)
                .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
            let path = (!path.is_empty()).then(|| PathBuf::from(path));
            Ok(Request::Open { path, launched })
        }
        "ping" => Ok(Request::Ping),
        "quit" => Ok(Request::Quit),
        _ => Err(Reply::Busy),
    }
}

/// Ask the resident, if one listens. `None` when there is none or it did
/// not answer.
pub fn ask(request: &Request) -> Option<Reply> {
    let text = encode(request)?;
    #[cfg(windows)]
    {
        win::ask(&text)
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        None
    }
}

/// Hand the image of this process to the resident. True when the resident
/// took it: this process has nothing left to do.
pub fn hand_over(path: Option<&Path>) -> bool {
    let request = Request::Open {
        path: path.map(Path::to_path_buf),
        launched: Some(launch_time()),
    };
    ask(&request) == Some(Reply::Ok)
}

/// When this process was created (now, where the system does not tell).
pub fn launch_time() -> SystemTime {
    #[cfg(windows)]
    if let Some(time) = win::process_created() {
        return time;
    }
    SystemTime::now()
}

/// Become the resident: answer requests with `handler` on a background
/// thread for the rest of the process. False when another process is the
/// resident already, or on a platform without one.
pub fn listen(handler: impl Fn(Request) -> Reply + Send + 'static) -> bool {
    #[cfg(windows)]
    {
        win::listen(Box::new(handler))
    }
    #[cfg(not(windows))]
    {
        let _ = handler;
        false
    }
}

/// Give back the memory the process no longer uses to the system.
pub fn trim_memory() {
    #[cfg(windows)]
    win::trim_memory();
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use super::{decode, Reply, Request};
    use std::ffi::c_void;
    use std::io::{Read, Write};
    use std::os::windows::io::AsRawHandle;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    type Handle = *mut c_void;

    const INVALID_HANDLE_VALUE: isize = -1;
    const PIPE_ACCESS_DUPLEX: u32 = 3;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    /// Longest request: a path of 32767 UTF-16 units is at most three
    /// times that in UTF-8.
    const MAX_REQUEST: usize = 100_000;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer: u32,
            in_buffer: u32,
            default_timeout: u32,
            security: *mut c_void,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
        fn DisconnectNamedPipe(pipe: Handle) -> i32;
        fn FlushFileBuffers(file: Handle) -> i32;
        fn ReadFile(
            file: Handle,
            buffer: *mut u8,
            len: u32,
            read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn WriteFile(
            file: Handle,
            buffer: *const u8,
            len: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn GetLastError() -> u32;
        fn GetNamedPipeServerProcessId(pipe: Handle, process_id: *mut u32) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentProcessId() -> u32;
        fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> i32;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn SetProcessWorkingSetSize(process: Handle, min: usize, max: usize) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }

    /// The pipe of this user in this logon session.
    fn pipe_name() -> String {
        let mut session = 0u32;
        // SAFETY: `session` is valid for one write; the call has no other
        // memory preconditions.
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
        let user: String = std::env::var("USERNAME")
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        format!(r"\\.\pipe\pandite-resident-{session}-{user}")
    }

    pub fn ask(text: &str) -> Option<Reply> {
        let mut pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe_name())
            .ok()?;
        let mut server = 0u32;
        // SAFETY: the handle is an open pipe owned by `pipe`; `server` is
        // valid for one write.
        if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server) } != 0 {
            // This process was started by the user and may take the
            // foreground; pass that right on to the resident's window.
            // SAFETY: no memory preconditions.
            unsafe { AllowSetForegroundWindow(server) };
        }
        pipe.write_all(text.as_bytes()).ok()?;
        let mut answer = Vec::new();
        let mut byte = [0u8; 1];
        while answer.len() < 16 {
            match pipe.read(&mut byte) {
                Ok(1) if byte[0] != b'\n' => answer.push(byte[0]),
                _ => break,
            }
        }
        Reply::parse(std::str::from_utf8(&answer).ok()?)
    }

    /// Read the three request lines from a connected pipe.
    fn read_request(pipe: Handle) -> Option<String> {
        let mut data = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let mut got = 0u32;
            // SAFETY: `chunk` is valid for writes of its length and `got`
            // for one write; `pipe` is the pipe handle this thread owns.
            let ok = unsafe {
                ReadFile(
                    pipe,
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut got,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 || got == 0 {
                return None;
            }
            data.extend_from_slice(&chunk[..got as usize]);
            if data.iter().filter(|&&b| b == b'\n').count() >= 3 {
                return String::from_utf8(data).ok();
            }
            if data.len() > MAX_REQUEST {
                return None;
            }
        }
    }

    fn write_reply(pipe: Handle, reply: Reply) {
        let text = format!("{}\n", reply.text());
        let mut written = 0u32;
        // SAFETY: `text` is valid for reads of its length and `written`
        // for one write; `pipe` is the pipe handle this thread owns.
        unsafe {
            WriteFile(
                pipe,
                text.as_ptr(),
                text.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
            FlushFileBuffers(pipe);
        }
    }

    /// A handle that may move to the listener thread.
    struct Pipe(Handle);
    // SAFETY: a pipe handle is a process-wide kernel object reference; the
    // listener thread is its only user.
    unsafe impl Send for Pipe {}

    pub fn listen(handler: Box<dyn Fn(Request) -> Reply + Send>) -> bool {
        let name: Vec<u16> = pipe_name().encode_utf16().chain(Some(0)).collect();
        // One instance, and only if no other process made the pipe first:
        // the owner of the pipe is the resident. Byte mode, blocking, local
        // clients only. The default security lets this user alone write.
        // SAFETY: `name` is NUL terminated and outlives the call; a null
        // security descriptor asks for the default one.
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                std::ptr::null_mut(),
            )
        };
        if pipe.is_null() || pipe as isize == INVALID_HANDLE_VALUE {
            return false;
        }
        let pipe = Pipe(pipe);
        std::thread::Builder::new()
            .name("pandite-resident".into())
            .spawn(move || {
                let pipe = pipe;
                loop {
                    // SAFETY: `pipe.0` is the pipe created above; it stays
                    // open for the life of the process.
                    let connected = unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) } != 0
                        // SAFETY: reads the calling thread's last error.
                        || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                    if !connected {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    let request = read_request(pipe.0).map(|text| decode(&text));
                    let (reply, stale) = match request {
                        Some(Ok(request)) => (handler(request), false),
                        Some(Err(Reply::Stale)) => (Reply::Stale, true),
                        Some(Err(reply)) => (reply, false),
                        None => (Reply::Busy, false),
                    };
                    write_reply(pipe.0, reply);
                    // SAFETY: as above.
                    unsafe { DisconnectNamedPipe(pipe.0) };
                    if stale {
                        // A newer program started: make room for it.
                        handler(Request::Quit);
                    }
                }
            })
            .is_ok()
    }

    pub fn process_created() -> Option<SystemTime> {
        let mut times = [FileTime::default(); 4];
        let [created, exit, kernel, user] = &mut times;
        // SAFETY: the four pointers are valid for one write each; the
        // pseudo handle of the current process is always valid.
        let ok = unsafe { GetProcessTimes(GetCurrentProcess(), created, exit, kernel, user) } != 0;
        if !ok {
            return None;
        }
        // 100 ns units since 1601; the Unix epoch is 11644473600 s later.
        let ticks = (u64::from(times[0].high) << 32) | u64::from(times[0].low);
        let unix = ticks.checked_sub(116_444_736_000_000_000)?;
        Some(UNIX_EPOCH + Duration::from_nanos(unix.checked_mul(100)?))
    }

    pub fn trim_memory() {
        // SAFETY: the pseudo handle is always valid; both sizes at the
        // maximum value ask the system to trim the working set.
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_survive_the_trip() {
        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        let path = if cfg!(windows) {
            r"C:\Users\me\Pictures\caf\u{e9} 1 2.png"
        } else {
            "/home/me/Pictures/caf\u{e9} 1 2.png"
        };
        for request in [
            Request::Open {
                path: Some(PathBuf::from(path)),
                launched: Some(at),
            },
            Request::Open {
                path: None,
                launched: None,
            },
            Request::Ping,
            Request::Quit,
        ] {
            let text = encode(&request).expect("encodes");
            assert_eq!(decode(&text), Ok(request));
        }
    }

    #[test]
    fn other_versions_and_noise_are_refused() {
        assert_eq!(
            decode("pandite 0.0.0-other\nopen\n0 a.png\n"),
            Err(Reply::Stale)
        );
        assert_eq!(decode("hello\nopen\n0 a.png\n"), Err(Reply::Busy));
        assert_eq!(decode(""), Err(Reply::Busy));
        let unknown = format!("{}\ndance\n\n", greeting());
        assert_eq!(decode(&unknown), Err(Reply::Busy));
        // A path with a line break cannot be sent.
        let odd = Request::Open {
            path: Some(PathBuf::from("a\nb.png")),
            launched: None,
        };
        assert_eq!(encode(&odd), None);
    }

    #[test]
    fn replies_parse() {
        for reply in [Reply::Ok, Reply::Busy, Reply::Stale] {
            assert_eq!(Reply::parse(reply.text()), Some(reply));
        }
        assert_eq!(Reply::parse("what"), None);
    }
}
