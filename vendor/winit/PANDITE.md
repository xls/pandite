# Pandite patch of winit 0.30.13

This is winit 0.30.13 from crates.io with one change. The Mac App Store rejects the private function `CGSSetWindowBackgroundBlurRadius`.

- `src/platform_impl/macos/ffi.rs`: the `CGSMainConnectionID` and `CGSSetWindowBackgroundBlurRadius` declarations are removed.
- `src/platform_impl/macos/window_delegate.rs`: `set_blur` does nothing.

Pandite does not use window blur. When a winit release removes the call, delete `[patch.crates-io]` in the root `Cargo.toml` and this folder.
