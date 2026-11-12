//! The GPU device shared by every renderer, upload and provider.

use crate::pipelines::Pipelines;
use pandite_core::{Error, Result};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

/// How to pick and open the GPU.
#[derive(Clone, Debug)]
pub struct GpuOptions {
    /// Prefer the power saving adapter (default, right for a viewer) or the
    /// fastest one.
    pub power_preference: wgpu::PowerPreference,
    /// Backends to try. Only the backends compiled into this build exist:
    /// DirectX 12 on Windows, Metal on Apple platforms, Vulkan and OpenGL ES
    /// elsewhere. The `WGPU_BACKEND` environment variable narrows it further.
    pub backends: wgpu::Backends,
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            power_preference: wgpu::PowerPreference::LowPower,
            backends: wgpu::Backends::all(),
        }
    }
}

/// An open GPU: instance, adapter, device and queue, plus the pipelines
/// every renderer shares. Wrap it in an [`Arc`] and share it between
/// renderers, upload threads and providers; all methods take `&self`.
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    first_error: Arc<Mutex<Option<String>>>,
    lost: Arc<AtomicBool>,
    max_texture: u32,
    pipelines: OnceLock<Pipelines>,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let info = self.adapter.get_info();
        f.debug_struct("Gpu")
            .field("adapter", &info.name)
            .field("backend", &info.backend)
            .field("max_texture", &self.max_texture)
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Open a GPU for offscreen work. Fails with [`Error::Backend`] when the
    /// machine has no usable adapter.
    pub fn new(options: &GpuOptions) -> Result<Arc<Gpu>> {
        let instance = wgpu::Instance::new(instance_descriptor(options, None));
        Self::open(instance, None, options).map(Arc::new)
    }

    /// Open a GPU that can present to `window`, and the surface for it.
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
        let instance =
            wgpu::Instance::new(instance_descriptor(options, Some(Box::new(window.clone()))));
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::from_window_without_display(window))
            .map_err(|e| Error::Backend(format!("cannot create the window surface: {e}")))?;
        let gpu = Self::open(instance, Some(&surface), options)?;
        Ok((Arc::new(gpu), surface))
    }

    fn open(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        options: &GpuOptions,
    ) -> Result<Gpu> {
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: options.power_preference,
            force_fallback_adapter: false,
            compatible_surface: surface,
            apply_limit_buckets: false,
        }))
        .map_err(|e| Error::Backend(format!("no GPU adapter: {e}")))?;
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
        .map_err(|e| Error::Backend(format!("cannot open the GPU device: {e}")))?;

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
            device,
            queue,
            first_error,
            lost,
            max_texture: limits.max_texture_dimension_2d,
            pipelines: OnceLock::new(),
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

    /// Human-readable adapter name and backend, for diagnostics.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
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

    /// Block until every submitted command finished.
    pub(crate) fn wait_idle(&self) -> Result<()> {
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
