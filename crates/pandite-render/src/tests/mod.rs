//! GPU tests. They render offscreen through the real device and compare
//! read-back pixels. One device is shared by every test in the binary.
//!
//! Without a GPU adapter each test prints a skip notice and passes, so CI
//! runners without a GPU stay green. With `PANDITE_REQUIRE_GPU=1` a missing
//! adapter fails the test instead.
//!
//! With `PANDITE_FORCE_FALLBACK_ADAPTER=1` the tests run on the software
//! adapter (WARP on Windows, lavapipe or llvmpipe on Linux) instead of the
//! hardware GPU. `WGPU_BACKEND` (for example `gl` on Linux) picks among
//! several software adapters. `PANDITE_ADAPTER_NAME=radeon` runs them on
//! the adapter whose name contains that text (ignoring case) instead of
//! the default choice, the discrete GPU.

mod animation;
mod interface;
mod providers;
mod still;
mod zoom;

use crate::{FontChoice, Gpu, GpuOptions, RenderSettings, Renderer};
use pandite_core::{Color, DrawList, Rect, ViewState};
use std::io::Write;
use std::sync::{Arc, Once, OnceLock};

/// True when the environment variable is set to anything but empty or "0".
fn switch(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

fn require_gpu() -> bool {
    switch("PANDITE_REQUIRE_GPU")
}

fn force_fallback() -> bool {
    switch("PANDITE_FORCE_FALLBACK_ADAPTER")
}

/// `PANDITE_ADAPTER_NAME`: run on the adapter whose name contains it.
fn adapter_name() -> Option<String> {
    std::env::var("PANDITE_ADAPTER_NAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// The shared test device, or `None` (after a printed notice) when the
/// machine has no adapter and `PANDITE_REQUIRE_GPU` is not set.
pub(crate) fn gpu() -> Option<Arc<Gpu>> {
    static GPU: OnceLock<std::result::Result<Arc<Gpu>, String>> = OnceLock::new();
    let open = || {
        let options = GpuOptions {
            force_fallback_adapter: force_fallback(),
            adapter_name: adapter_name(),
            ..GpuOptions::default()
        };
        Gpu::new(&options).map_err(|e| e.to_string())
    };
    match GPU.get_or_init(open) {
        Ok(gpu) => Some(Arc::clone(gpu)),
        Err(err) => {
            assert!(
                !require_gpu(),
                "PANDITE_REQUIRE_GPU is set but no GPU adapter is usable: {err}"
            );
            static NOTICE: Once = Once::new();
            NOTICE.call_once(|| {
                // Written past the test harness capture so CI logs show it.
                let _ = writeln!(
                    std::io::stderr(),
                    "SKIP: pandite-render GPU tests skipped, no GPU adapter ({err}). \
                     Set PANDITE_REQUIRE_GPU=1 to fail instead."
                );
            });
            None
        }
    }
}

/// `let gpu = gpu_or_skip!();` returns from the test when there is no GPU.
macro_rules! gpu_or_skip {
    () => {
        match crate::tests::gpu() {
            Some(gpu) => gpu,
            None => return,
        }
    };
}
pub(crate) use gpu_or_skip;

/// Settings with a plain background and the embedded font, so tests do not
/// depend on the machine's fonts.
pub(crate) fn plain(background: Color) -> RenderSettings {
    RenderSettings {
        background,
        checkerboard: None,
        font: FontChoice::Embedded,
        ..RenderSettings::default()
    }
}

/// Read-back pixels with coordinate access.
pub(crate) struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Pixels {
    pub(crate) fn at(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "({x}, {y}) outside");
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    pub(crate) fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let p = self.at(x, y);
        [p[0], p[1], p[2]]
    }
}

pub(crate) fn offscreen(gpu: &Arc<Gpu>, w: u32, h: u32, settings: RenderSettings) -> Renderer {
    Renderer::offscreen(Arc::clone(gpu), w, h, settings).expect("offscreen renderer")
}

/// Render one frame over the whole target and read it back.
pub(crate) fn frame(renderer: &mut Renderer, view: &ViewState, ui: &DrawList) -> Pixels {
    let viewport = renderer.full_viewport();
    frame_in(renderer, view, viewport, ui)
}

pub(crate) fn frame_in(
    renderer: &mut Renderer,
    view: &ViewState,
    viewport: Rect,
    ui: &DrawList,
) -> Pixels {
    renderer.render(view, viewport, ui).expect("render");
    let image = renderer.read_pixels().expect("read back");
    Pixels {
        width: image.width(),
        height: image.height(),
        data: image.data().to_vec(),
    }
}

/// True when every channel differs by at most `tol`.
pub(crate) fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

/// Assert with a readable message.
pub(crate) fn assert_rgb(px: &Pixels, x: u32, y: u32, want: [u8; 3], tol: u8) {
    let got = px.rgb(x, y);
    assert!(
        near(got, want, tol),
        "pixel ({x}, {y}) is {got:?}, want {want:?} +-{tol}"
    );
}

/// The sRGB code of a linear value, as the GPU stores it.
pub(crate) fn srgb_code(linear: f32) -> u8 {
    (crate::color::linear_to_srgb(linear) * 255.0).round() as u8
}

/// An RGBA8 image from a per-pixel function.
pub(crate) fn rgba8(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> pandite_core::ImageBuf {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&f(x, y));
        }
    }
    pandite_core::ImageBuf::new(w, h, pandite_core::PixelFormat::Rgba8, data).expect("image")
}

#[test]
fn fallback_adapter_is_software() {
    if !force_fallback() {
        let _ = writeln!(
            std::io::stderr(),
            "NOTICE: fallback_adapter_is_software checks nothing: \
             PANDITE_FORCE_FALLBACK_ADAPTER is not set."
        );
        return;
    }
    let gpu = gpu_or_skip!();
    let info = gpu.adapter().get_info();
    // Written past the test harness capture so gate logs name the adapter.
    let _ = writeln!(
        std::io::stderr(),
        "ADAPTER: {}, driver {} {}, max texture {}",
        gpu.describe(),
        info.driver,
        info.driver_info,
        gpu.max_texture_size()
    );
    assert_eq!(
        info.device_type,
        wgpu::DeviceType::Cpu,
        "PANDITE_FORCE_FALLBACK_ADAPTER is set but the adapter is not a software one: {info:?}"
    );
}

#[test]
fn default_adapter_is_discrete_when_present() {
    if force_fallback() || adapter_name().is_some() {
        let _ = writeln!(
            std::io::stderr(),
            "NOTICE: default_adapter_is_discrete_when_present checks nothing: \
             PANDITE_FORCE_FALLBACK_ADAPTER or PANDITE_ADAPTER_NAME is set."
        );
        return;
    }
    let adapters = crate::list_adapters(&GpuOptions::default());
    let listed: Vec<String> = adapters.iter().map(ToString::to_string).collect();
    let _ = writeln!(std::io::stderr(), "ADAPTERS: {}", listed.join("; "));
    let gpu = gpu_or_skip!();
    let _ = writeln!(std::io::stderr(), "ADAPTER IN USE: {}", gpu.describe());
    if !adapters
        .iter()
        .any(|a| a.device_type == wgpu::DeviceType::DiscreteGpu)
    {
        let _ = writeln!(
            std::io::stderr(),
            "NOTICE: default_adapter_is_discrete_when_present: no discrete adapter here."
        );
        return;
    }
    assert_eq!(
        gpu.adapter_summary().device_type,
        wgpu::DeviceType::DiscreteGpu,
        "the default options chose {} over a discrete GPU ({})",
        gpu.describe(),
        listed.join("; ")
    );
    // The best adapter of the list is the one in use.
    assert_eq!(&adapters[0], gpu.adapter_summary());
}

#[test]
fn public_types_cross_threads() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<Renderer>();
    send::<crate::PreparedContent>();
    send::<crate::ContentFactory>();
    sync::<crate::ContentFactory>();
    send::<crate::GpuImage>();
    sync::<crate::GpuImage>();
    send::<Arc<Gpu>>();
    sync::<Gpu>();
    send::<crate::TextMeasurer>();
    sync::<crate::TextMeasurer>();
    send::<crate::ProviderRegistry>();
    sync::<crate::ProviderRegistry>();
}
