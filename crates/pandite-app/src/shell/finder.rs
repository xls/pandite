//! Files that macOS hands to the app: a double click in Finder, "Open
//! With", or a file dropped on the Dock icon.
//!
//! macOS does not pass such a file on the command line. It sends the app
//! an "open documents" event, and AppKit delivers it to the application
//! delegate as `application:openURLs:`. The delegate belongs to winit and
//! has no such method, so AppKit answers "pandite cannot open files in
//! this format". This module adds the method to the delegate's class and
//! forwards every file to the event loop.

use objc2::ffi;
use objc2::runtime::{AnyObject, Imp, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::{NSArray, NSURL};
use std::path::PathBuf;
use std::sync::OnceLock;

type Sink = Box<dyn Fn(PathBuf) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// `-[NSApplicationDelegate application:openURLs:]`.
extern "C-unwind" fn open_urls(
    _this: &AnyObject,
    _cmd: Sel,
    _app: &AnyObject,
    urls: &NSArray<NSURL>,
) {
    let Some(sink) = SINK.get() else { return };
    for url in urls {
        if let Some(path) = url.path() {
            sink(PathBuf::from(path.to_string()));
        }
    }
}

/// Send every file that macOS asks the app to open to `sink`. Call once,
/// on the main thread, after the event loop exists and before it runs.
#[allow(unsafe_code)]
pub fn install(sink: impl Fn(PathBuf) + Send + Sync + 'static) {
    if SINK.set(Box::new(sink)).is_err() {
        return;
    }
    // SAFETY: the calls run on the main thread. `delegate` is the
    // application delegate that the event loop created and keeps alive.
    // The method has the signature that its type encoding declares:
    // void, self, selector, two objects.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let delegate: *mut AnyObject = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let class = ffi::object_getClass(delegate.cast_const());
        let imp: Imp = std::mem::transmute::<
            extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSURL>),
            Imp,
        >(open_urls);
        ffi::class_addMethod(
            class.cast_mut(),
            sel!(application:openURLs:),
            imp,
            c"v@:@@".as_ptr(),
        );
        // AppKit notes what a delegate answers to when it is set: set it
        // again, so the new method counts.
        let none: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![app, setDelegate: none];
        let _: () = msg_send![app, setDelegate: delegate];
    }
}
