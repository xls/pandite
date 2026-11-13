//! The GPU device shared by every renderer, upload and provider.

use crate::pipelines::Pipelines;
use pandite_core::{Error, Result};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

/// How to pick and open the GPU.
///
/// Adapters are tried in this order: discrete GPU, integrated GPU, virtual
/// GPU, unknown type, then software rasterizers, so software is used only
/// when nothing else works. Within one type, native backends come before
/// OpenGL. A window is only offered adapters that can present to it.
#[derive(Clone, Debug)]
pub struct GpuOptions {
    /// `HighPerformance` (default) or `None` keeps the order above.
    /// `LowPower` tries integrated GPUs before discrete ones.
    pub power_preference: wgpu::PowerPreference,
    /// Backends to try. Only the backends compiled into this build exist:
    /// DirectX 12 on Windows, Metal on Apple platforms, Vulkan and OpenGL ES
    /// elsewhere. The `WGPU_BACKEND` environment variable narrows it further.
    pub backends: wgpu::Backends,
    /// Accept only a fallback adapter: a software rasterizer such as WARP
    /// on Windows or lavapipe and llvmpipe on Linux. Off by default. Use it
    /// to test or diagnose rendering without a hardware GPU; opening fails
    /// when no such adapter exists.
    pub force_fallback_adapter: bool,
    /// Try adapters whose name contains this text (ignoring case) first,
    /// for a user setting such as "NVIDIA". When none matches, the order
    /// above applies. `None` (default) or an empty text matches nothing.
    pub adapter_name: Option<String>,
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            power_preference: wgpu::PowerPreference::HighPerformance,
            backends: wgpu::Backends::all(),
            force_fallback_adapter: false,
            adapter_name: None,
        }
    }
}

/// One GPU adapter of this machine, as [`list_adapters`] reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterSummary {
    /// Name reported by the driver, for example "NVIDIA GeForce RTX 5090".
    pub name: String,
    /// Discrete, integrated, virtual, software (`Cpu`) or unknown (`Other`).
    pub device_type: wgpu::DeviceType,
    /// Graphics API the adapter is reached through.
    pub backend: wgpu::Backend,
}

impl AdapterSummary {
    fn of(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();
        Self {
            name: info.name,
            device_type: info.device_type,
            backend: info.backend,
        }
    }
}

impl std::fmt::Display for AdapterSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.device_type {
            wgpu::DeviceType::DiscreteGpu => "discrete",
            wgpu::DeviceType::IntegratedGpu => "integrated",
            wgpu::DeviceType::VirtualGpu => "virtual",
            wgpu::DeviceType::Cpu => "software",
            wgpu::DeviceType::Other => "unknown type",
        };
        write!(f, "{} ({:?}, {kind})", self.name, self.backend)
    }
}

/// Every adapter the backends of `options` reach (narrowed by the
/// `WGPU_BACKEND` environment variable), best first in the order
/// [`GpuOptions`] describes, ignoring `force_fallback_adapter` and
/// `adapter_name`. Exact duplicates are listed once. For a settings screen
/// that lets the user pick a GPU by name.
pub fn list_adapters(options: &GpuOptions) -> Vec<AdapterSummary> {
    let instance = wgpu::Instance::new(instance_descriptor(options, None));
    let all: Vec<AdapterSummary> = block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .iter()
        .map(AdapterSummary::of)
        .collect();
    let plain = GpuOptions {
        force_fallback_adapter: false,
        adapter_name: None,
        ..options.clone()
    };
    // DXGI can report one GPU twice; a settings screen lists it once.
    let mut out: Vec<AdapterSummary> = Vec::new();
    for i in adapter_order(&all, &plain) {
        if !out.contains(&all[i]) {
            out.push(all[i].clone());
        }
    }
    out
}

/// Indices of `adapters` in the order to try them under `options`. With
/// `force_fallback_adapter` only software adapters are listed.
pub(crate) fn adapter_order(adapters: &[AdapterSummary], options: &GpuOptions) -> Vec<usize> {
    let wanted = options
        .adapter_name
        .as_deref()
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty());
    let low_power = options.power_preference == wgpu::PowerPreference::LowPower;
    let type_rank = |t: wgpu::DeviceType| match t {
        wgpu::DeviceType::DiscreteGpu => u8::from(low_power),
        wgpu::DeviceType::IntegratedGpu => u8::from(!low_power),
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    };
    let mut order: Vec<usize> = (0..adapters.len())
        .filter(|&i| {
            !options.force_fallback_adapter || adapters[i].device_type == wgpu::DeviceType::Cpu
        })
        .collect();
    // Stable: equal keys keep the enumeration order.
    order.sort_by_key(|&i| {
        let a = &adapters[i];
        let unnamed = wanted
            .as_ref()
            .is_some_and(|w| !a.name.to_lowercase().contains(w.as_str()));
        (
            unnamed,
            type_rank(a.device_type),
            a.backend == wgpu::Backend::Gl,
        )
    });
    order
}

/// The adapters to try for `options`, best first, keeping only those that
/// can present to `surface` when one is given.
pub(crate) fn candidates(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
    options: &GpuOptions,
) -> Vec<wgpu::Adapter> {
    let mut adapters: Vec<Option<wgpu::Adapter>> =
        block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
            .map(Some)
            .collect();
    let summaries: Vec<AdapterSummary> =
        adapters.iter().flatten().map(AdapterSummary::of).collect();
    adapter_order(&summaries, options)
        .into_iter()
        .filter_map(|i| adapters[i].take())
        .collect()
}

/// A wgpu instance that can present to `window`, and a surface for it.
pub(crate) fn window_instance<W>(
    window: W,
    options: &GpuOptions,
) -> Result<(wgpu::Instance, wgpu::Surface<'static>)>
where
    W: wgpu::rwh::HasWindowHandle
        + wgpu::rwh::HasDisplayHandle
        + Clone
        + std::fmt::Debug
        + Send
        + Sync
        + 'static,
{
    let instance =
        wgpu::Instance::new(instance_descriptor(options, Some(Box::new(window.clone()))));
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::from_window_without_display(window))
        .map_err(|e| Error::Backend(format!("cannot create the window surface: {e}")))?;
    Ok((instance, surface))
}

/// The error for an empty candidate list or a list that all failed.
pub(crate) fn no_adapter(options: &GpuOptions, failures: &[String]) -> Error {
    let what = if options.force_fallback_adapter {
        "no software GPU adapter"
    } else {
        "no GPU adapter"
    };
    if failures.is_empty() {
        Error::Backend(format!("{what} found"))
    } else {
        Error::Backend(format!("{what} usable: {}", failures.join("; ")))
    }
}

/// An open GPU: instance, adapter, device and queue, plus the pipelines
/// every renderer shares. Wrap it in an [`Arc`] and share it between
/// renderers, upload threads and providers; all methods take `&self`.
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    summary: AdapterSummary,
    device: wgpu::Device,
    queue: wgpu::Queue,
    first_error: Arc<Mutex<Option<String>>>,
    lost: Arc<AtomicBool>,
    max_texture: u32,
    pipelines: OnceLock<Pipelines>,
    measure: AtomicBool,
    staging: StagingPool,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("adapter", &self.summary.name)
            .field("device_type", &self.summary.device_type)
            .field("backend", &self.summary.backend)
            .field("max_texture", &self.max_texture)
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Open a GPU for offscreen work on the first adapter, in the order
    /// [`GpuOptions`] describes, that opens a device. Fails with
    /// [`Error::Backend`] when the machine has no usable adapter.
    pub fn new(options: &GpuOptions) -> Result<Arc<Gpu>> {
        let instance = wgpu::Instance::new(instance_descriptor(options, None));
        let mut failures = Vec::new();
        for adapter in candidates(&instance, None, options) {
            match Self::open(instance.clone(), adapter) {
                Ok(gpu) => return Ok(Arc::new(gpu)),
                Err(err) => failures.push(err.to_string()),
            }
        }
        Err(no_adapter(options, &failures))
    }

    /// Open a GPU that can present to `window`, and the surface for it: the
    /// first adapter, in the order [`GpuOptions`] describes, that can
    /// present to the window and opens a device. [`crate::Renderer::for_window`]
    /// also moves on to the next adapter when configuring the surface fails.
    ///
    /// The window must provide raw window and display handles (a winit
    /// `Arc<Window>` does). On macOS call this on the main thread.
    pub fn for_window<W>(
        window: W,
        options: &GpuOptions,
    ) -> Result<(Arc<Gpu>, wgpu::Surface<'static>)>
    where
        W: wgpu::rwh::HasWindowHandle
            + wgpu::rwh::HasDisplayHandle
            + Clone
            + std::fmt::Debug
            + Send
            + Sync
            + 'static,
    {
        let (instance, surface) = window_instance(window, options)?;
        let mut failures = Vec::new();
        for adapter in candidates(&instance, Some(&surface), options) {
            match Self::open(instance.clone(), adapter) {
                Ok(gpu) => return Ok((Arc::new(gpu), surface)),
                Err(err) => failures.push(err.to_string()),
            }
        }
        Err(no_adapter(options, &failures))
    }

    /// Open a device on `adapter`.
    pub(crate) fn open(instance: wgpu::Instance, adapter: wgpu::Adapter) -> Result<Gpu> {
        let summary = AdapterSummary::of(&adapter);
        // Ask for everything the adapter offers so large images fit in as
        // few tiles as possible (16384 texels per side on DirectX 12).
        let limits = adapter.limits();
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pandite"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| Error::Backend(format!("cannot open a device on {summary}: {e}")))?;

        // Uploads, renderer setup and frames catch their own errors with
        // error scopes. Anything else is recorded for `take_error` instead
        // of panicking (wgpu's default).
        let first_error = Arc::new(Mutex::new(None::<String>));
        let sink = Arc::clone(&first_error);
        device.on_uncaptured_error(Arc::new(move |err: wgpu::Error| {
            if let Ok(mut slot) = sink.lock() {
                slot.get_or_insert_with(|| err.to_string());
            }
        }));
        let lost = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lost);
        device.set_device_lost_callback(move |_reason, _message| {
            flag.store(true, Ordering::Relaxed);
        });

        Ok(Gpu {
            instance,
            adapter,
            summary,
            device,
            queue,
            first_error,
            lost,
            max_texture: limits.max_texture_dimension_2d,
            pipelines: OnceLock::new(),
            measure: AtomicBool::new(false),
            staging: StagingPool::default(),
        })
    }

    /// The wgpu instance (create more surfaces with it).
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// The adapter the device was opened on.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// The logical device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The submission queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Largest texture side the device accepts. Larger images are tiled.
    pub fn max_texture_size(&self) -> u32 {
        self.max_texture
    }

    /// Name, device type and backend of the adapter the device runs on.
    pub fn adapter_summary(&self) -> &AdapterSummary {
        &self.summary
    }

    /// Human-readable adapter name, backend and device type, for
    /// diagnostics: "NVIDIA GeForce RTX 5090 (Dx12, discrete)".
    pub fn describe(&self) -> String {
        self.summary.to_string()
    }

    /// The first GPU error raised outside the renderer's own calls since
    /// the last call, if any. Uploads, renderer setup and frames report
    /// their errors directly; this catches the rest (for example from a
    /// custom provider's work on another thread).
    pub fn take_error(&self) -> Option<String> {
        self.first_error
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// True once the driver reported the device as lost. Recreate the
    /// [`Gpu`] and every renderer after that.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    pub(crate) fn pipelines(&self) -> &Pipelines {
        self.pipelines
            .get_or_init(|| Pipelines::new(&self.device, &self.adapter))
    }

    /// The reusable staging buffers of large uploads.
    pub(crate) fn staging(&self) -> &StagingPool {
        &self.staging
    }

    /// Allocate the staging buffers large uploads use and touch their
    /// memory, so the first image upload does not pay for fresh pages.
    /// Call it on a worker thread at start-up.
    pub fn prepare_uploads(&self) {
        let leases: Vec<StagingLease> = (0..STAGING_KEEP)
            .filter_map(|_| self.staging.take(self, crate::image::STRIP_BYTES).ok())
            .collect();
        for lease in leases {
            if let Ok(mut view) = lease.buffer.slice(..).get_mapped_range_mut() {
                view.slice(..).fill(0);
            }
            self.staging.put_mapped(lease);
        }
    }

    /// Measure content uploads: when on, every upload waits for the GPU
    /// after the pixel conversion and after mip generation and records
    /// both durations (see [`crate::PreparedContent::upload_timings`]).
    /// Off by default; the waits run on the uploading thread only.
    pub fn set_measure_uploads(&self, on: bool) {
        self.measure.store(on, Ordering::Relaxed);
    }

    /// True when uploads are measured.
    pub(crate) fn measuring(&self) -> bool {
        self.measure.load(Ordering::Relaxed)
    }

    /// Block until every submitted command finished. Never call it on an
    /// interface thread outside of measurements.
    pub fn wait_idle(&self) -> Result<()> {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map(|_| ())
            .map_err(|e| Error::Backend(format!("GPU wait failed: {e}")))
    }

    /// Run `work` and turn out-of-memory and validation errors it raises
    /// on this thread into an [`Error`]. Used around uploads, where a
    /// failure must reach the caller instead of the global error slot.
    pub(crate) fn scoped<T>(&self, what: &str, work: impl FnOnce() -> Result<T>) -> Result<T> {
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = work();
        let validation = block_on(validation.pop());
        let oom = block_on(oom.pop());
        if let Some(err) = oom {
            return Err(Error::Backend(format!("{what}: out of GPU memory ({err})")));
        }
        if let Some(err) = validation {
            return Err(Error::Backend(format!("{what}: {err}")));
        }
        result
    }

    /// Copy one mip level of a texture back to the CPU, rows tightly packed.
    pub(crate) fn read_texture(&self, texture: &wgpu::Texture, level: u32) -> Result<Vec<u8>> {
        let format = texture.format();
        let bpp = format
            .block_copy_size(None)
            .ok_or_else(|| Error::Invalid(format!("cannot read back {format:?}")))?;
        let width = (texture.width() >> level).max(1);
        let height = (texture.height() >> level).max(1);
        let row = width * bpp;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |result| {
            // The receiver outlives the poll below; a send error cannot happen.
            let _ = tx.send(result);
        });
        self.wait_idle()?;
        rx.recv()
            .map_err(|_| Error::Backend("readback callback did not run".into()))?
            .map_err(|e| Error::Backend(format!("readback mapping failed: {e}")))?;
        let mapped = buffer
            .get_mapped_range(..)
            .map_err(|e| Error::Backend(format!("readback range: {e:?}")))?;
        let mut out = Vec::with_capacity((row * height) as usize);
        for y in 0..height as usize {
            let start = y * padded as usize;
            out.extend_from_slice(&mapped[start..start + row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(out)
    }
}

fn instance_descriptor(
    options: &GpuOptions,
    display: Option<Box<dyn wgpu::wgt::WgpuHasDisplayHandle>>,
) -> wgpu::InstanceDescriptor {
    let mut desc = match display {
        Some(display) => wgpu::InstanceDescriptor::new_with_display_handle(display),
        None => wgpu::InstanceDescriptor::new_without_display_handle(),
    };
    desc.backends = options.backends;
    desc.with_env()
}

/// Drive a future to completion on this thread. wgpu's native futures are
/// ready almost immediately; the thread parks only while one is pending.
/// Staging buffers kept for reuse.
const STAGING_KEEP: usize = 2;

/// A mapped staging buffer on loan to one upload strip.
pub(crate) struct StagingLease {
    buffer: wgpu::Buffer,
    /// Set by the map callback once the buffer is mapped again.
    mapped: Arc<AtomicBool>,
}

impl StagingLease {
    /// The buffer; it is mapped for writing while on loan.
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

/// One buffer waiting in the pool: mapped, or being mapped again after
/// the copy of `pending` finished.
struct Pooled {
    lease: StagingLease,
    pending: Option<wgpu::SubmissionIndex>,
}

/// Reusable upload buffers (`MAP_WRITE | COPY_SRC`). A strip takes a
/// mapped buffer, fills it with one copy of its rows, unmaps it and
/// records a buffer-to-texture copy; after the submit the buffer goes
/// back and is mapped again once that copy finished. Two buffers let the
/// CPU fill one while the GPU copies from the other, and reused memory
/// is already paged in.
#[derive(Default)]
pub(crate) struct StagingPool {
    free: Mutex<Vec<Pooled>>,
}

impl StagingPool {
    /// A mapped buffer of at least `size` bytes. Waits (on this upload
    /// thread) for a pooled buffer's previous copy when it is still
    /// running; creates a new buffer when none fits.
    pub(crate) fn take(&self, gpu: &Gpu, size: u64) -> Result<StagingLease> {
        let found = {
            let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
            let pos = free.iter().position(|p| p.lease.buffer.size() >= size);
            pos.map(|i| free.swap_remove(i))
        };
        if let Some(pooled) = found {
            if !pooled.lease.mapped.load(Ordering::Acquire) {
                gpu.device
                    .poll(wgpu::PollType::Wait {
                        submission_index: pooled.pending,
                        timeout: None,
                    })
                    .map_err(|e| Error::Backend(format!("GPU wait failed: {e}")))?;
            }
            if pooled.lease.mapped.load(Ordering::Acquire) {
                return Ok(pooled.lease);
            }
            // The map failed (device lost): drop it and make a new one.
        }
        let size = size.max(crate::image::STRIP_BYTES);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload staging"),
            size,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        Ok(StagingLease {
            buffer,
            mapped: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Return a buffer whose copy was submitted as `index`: it is mapped
    /// again in the background and reused once mapped. Beyond
    /// [`STAGING_KEEP`] buffers the extra one is dropped.
    pub(crate) fn give_back(&self, lease: StagingLease, index: wgpu::SubmissionIndex) {
        lease.mapped.store(false, Ordering::Release);
        let flag = Arc::clone(&lease.mapped);
        lease
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Write, move |result| {
                flag.store(result.is_ok(), Ordering::Release);
            });
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < STAGING_KEEP {
            free.push(Pooled {
                lease,
                pending: Some(index),
            });
        }
    }

    /// Return a buffer that is still mapped (never used by the GPU).
    fn put_mapped(&self, lease: StagingLease) {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < STAGING_KEEP {
            free.push(Pooled {
                lease,
                pending: None,
            });
        }
    }
}

pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use wgpu::{Backend, DeviceType};

    fn adapter(name: &str, device_type: DeviceType, backend: Backend) -> AdapterSummary {
        AdapterSummary {
            name: name.into(),
            device_type,
            backend,
        }
    }

    fn names(list: &[AdapterSummary], options: &GpuOptions) -> Vec<String> {
        adapter_order(list, options)
            .into_iter()
            .map(|i| list[i].name.clone())
            .collect()
    }

    #[test]
    fn adapter_selection_prefers_discrete() {
        // Enumeration order as Windows reports it on a laptop: the display
        // adapter (integrated) first, software last.
        let machine = [
            adapter(
                "AMD Radeon(TM) Graphics",
                DeviceType::IntegratedGpu,
                Backend::Dx12,
            ),
            adapter(
                "NVIDIA GeForce RTX 5090",
                DeviceType::DiscreteGpu,
                Backend::Dx12,
            ),
            adapter(
                "Microsoft Basic Render Driver",
                DeviceType::Cpu,
                Backend::Dx12,
            ),
        ];
        let default = GpuOptions::default();
        assert_eq!(
            default.power_preference,
            wgpu::PowerPreference::HighPerformance
        );
        assert_eq!(
            names(&machine, &default),
            [
                "NVIDIA GeForce RTX 5090",
                "AMD Radeon(TM) Graphics",
                "Microsoft Basic Render Driver"
            ]
        );
        let none = GpuOptions {
            power_preference: wgpu::PowerPreference::None,
            ..GpuOptions::default()
        };
        assert_eq!(names(&machine, &none)[0], "NVIDIA GeForce RTX 5090");

        // LowPower swaps discrete and integrated, software stays last.
        let low = GpuOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            ..GpuOptions::default()
        };
        assert_eq!(
            names(&machine, &low),
            [
                "AMD Radeon(TM) Graphics",
                "NVIDIA GeForce RTX 5090",
                "Microsoft Basic Render Driver"
            ]
        );

        // The name filter wins over the type, ignoring case; a name that
        // matches nothing leaves the default order.
        let named = |name: &str| GpuOptions {
            adapter_name: Some(name.into()),
            ..GpuOptions::default()
        };
        assert_eq!(
            names(&machine, &named("radeon"))[0],
            "AMD Radeon(TM) Graphics"
        );
        assert_eq!(
            names(&machine, &named("BASIC RENDER"))[0],
            "Microsoft Basic Render Driver"
        );
        assert_eq!(names(&machine, &named("Intel")), names(&machine, &default));
        assert_eq!(names(&machine, &named("  ")), names(&machine, &default));

        // Software only when forced, or when nothing else exists.
        let forced = GpuOptions {
            force_fallback_adapter: true,
            ..GpuOptions::default()
        };
        assert_eq!(names(&machine, &forced), ["Microsoft Basic Render Driver"]);
        let forced_named = GpuOptions {
            adapter_name: Some("nvidia".into()),
            ..forced.clone()
        };
        assert_eq!(
            names(&machine, &forced_named),
            ["Microsoft Basic Render Driver"]
        );
        let runner = [adapter(
            "Microsoft Basic Render Driver",
            DeviceType::Cpu,
            Backend::Dx12,
        )];
        assert_eq!(names(&runner, &default), ["Microsoft Basic Render Driver"]);
        let gpu_only = [adapter(
            "Apple M4",
            DeviceType::IntegratedGpu,
            Backend::Metal,
        )];
        assert!(names(&gpu_only, &forced).is_empty());

        // Linux: virtual and unknown types rank between GPUs and software,
        // and the same GPU through Vulkan beats it through OpenGL.
        let linux = [
            adapter(
                "llvmpipe (LLVM 20.1.2, 256 bits)",
                DeviceType::Cpu,
                Backend::Vulkan,
            ),
            adapter(
                "Mesa Intel(R) Graphics",
                DeviceType::IntegratedGpu,
                Backend::Gl,
            ),
            adapter("virgl", DeviceType::VirtualGpu, Backend::Vulkan),
            adapter(
                "Mesa Intel(R) Graphics",
                DeviceType::IntegratedGpu,
                Backend::Vulkan,
            ),
            adapter("unknown GL renderer", DeviceType::Other, Backend::Gl),
        ];
        let order = adapter_order(&linux, &default);
        assert_eq!(order, [3, 1, 2, 4, 0]);
    }

    #[test]
    fn adapter_summary_reads_well() {
        let a = adapter(
            "NVIDIA GeForce RTX 5090",
            DeviceType::DiscreteGpu,
            Backend::Dx12,
        );
        assert_eq!(a.to_string(), "NVIDIA GeForce RTX 5090 (Dx12, discrete)");
        let w = adapter(
            "Microsoft Basic Render Driver",
            DeviceType::Cpu,
            Backend::Dx12,
        );
        assert_eq!(
            w.to_string(),
            "Microsoft Basic Render Driver (Dx12, software)"
        );
    }
}
