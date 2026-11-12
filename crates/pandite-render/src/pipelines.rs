//! Shader modules, bind group layouts, samplers and pipelines. Created once
//! per [`crate::Gpu`]; the display pipelines once per color target format.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Formats the upload path stores content in.
pub(crate) const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Storage for 16-bit and floating point content: linear half floats.
/// Devices that cannot render to it (OpenGL ES without
/// `EXT_color_buffer_float`) store such content in [`COLOR_FORMAT`].
pub(crate) const WIDE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Storage for alpha masks (icons) and the glyph atlas.
pub(crate) const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// Depth attachment format offered to providers that ask for depth.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Size of the per-draw uniform block in `image.wgsl` (`DrawParams`).
pub(crate) const DRAW_PARAMS_SIZE: u64 = 64;
/// Size of the per-frame uniform block in `image.wgsl` (`FrameParams`).
pub(crate) const FRAME_PARAMS_SIZE: u64 = 64;
/// Size of one interface instance in bytes.
pub(crate) const UI_INSTANCE_SIZE: u64 = 64;

/// Pipelines that draw into the color target of one renderer.
pub(crate) struct DisplayPipelines {
    pub image: wgpu::RenderPipeline,
    pub image_with_depth: wgpu::RenderPipeline,
    pub ui: wgpu::RenderPipeline,
}

pub(crate) struct Pipelines {
    /// Storage format for 16-bit and floating point content.
    pub wide_format: wgpu::TextureFormat,
    pub convert_float_layout: wgpu::BindGroupLayout,
    pub convert_uint_layout: wgpu::BindGroupLayout,
    pub mip_layout: wgpu::BindGroupLayout,
    pub convert_srgb8: wgpu::RenderPipeline,
    pub convert_rgba16: wgpu::RenderPipeline,
    pub convert_float: wgpu::RenderPipeline,
    mips: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    resample: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    /// Group 0 of the image pass: frame uniforms and the sampler.
    pub frame_layout: wgpu::BindGroupLayout,
    /// Group 1 of the image and interface passes: texture and draw uniforms.
    pub texture_layout: wgpu::BindGroupLayout,
    /// Trilinear, clamped to the edge, without anisotropic filtering. Used
    /// for content, resampling and the interface pass.
    pub sampler: wgpu::Sampler,
    image_module: wgpu::ShaderModule,
    ui_module: wgpu::ShaderModule,
    image_layout: wgpu::PipelineLayout,
    display: Mutex<HashMap<wgpu::TextureFormat, Arc<DisplayPipelines>>>,
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    groups: &[&wgpu::BindGroupLayout],
) -> wgpu::PipelineLayout {
    let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().copied().map(Some).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &groups,
        immediate_size: 0,
    })
}

struct PipelineSpec<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vertex: &'a str,
    fragment: &'a str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    topology: wgpu::PrimitiveTopology,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    depth: bool,
}

fn pipeline(device: &wgpu::Device, spec: &PipelineSpec<'_>) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.module,
            entry_point: Some(spec.vertex),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: spec.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: spec.topology,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: spec.depth.then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: spec.module,
            entry_point: Some(spec.fragment),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: spec.format,
                blend: spec.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

impl Pipelines {
    pub(crate) fn new(device: &wgpu::Device, adapter: &wgpu::Adapter) -> Self {
        let wide_usages = adapter
            .get_texture_format_features(WIDE_FORMAT)
            .allowed_usages;
        let wide_format = if wide_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
        {
            WIDE_FORMAT
        } else {
            COLOR_FORMAT
        };
        let unfilterable = wgpu::TextureSampleType::Float { filterable: false };
        let filterable = wgpu::TextureSampleType::Float { filterable: true };
        let upload_module = module(device, "upload", include_str!("shaders/upload.wgsl"));
        let image_module = module(device, "image", include_str!("shaders/image.wgsl"));
        let ui_module = module(device, "ui", include_str!("shaders/ui.wgsl"));

        let convert_float_layout = layout(
            device,
            "convert float",
            &[texture_entry(0, unfilterable), uniform_entry(2)],
        );
        let convert_uint_layout = layout(
            device,
            "convert uint",
            &[
                texture_entry(1, wgpu::TextureSampleType::Uint),
                uniform_entry(2),
            ],
        );
        let mip_layout = layout(device, "mip", &[texture_entry(0, unfilterable)]);
        let frame_layout = layout(device, "frame", &[uniform_entry(0), sampler_entry(1)]);
        let texture_layout = layout(
            device,
            "texture",
            &[texture_entry(0, filterable), uniform_entry(1)],
        );

        let convert_float_pl = pipeline_layout(device, "convert float", &[&convert_float_layout]);
        let convert_uint_pl = pipeline_layout(device, "convert uint", &[&convert_uint_layout]);
        let mip_pl = pipeline_layout(device, "mip", &[&mip_layout]);
        let image_layout = pipeline_layout(device, "image", &[&frame_layout, &texture_layout]);

        let fullscreen = |label: &str, pl: &wgpu::PipelineLayout, fs: &str, format| {
            pipeline(
                device,
                &PipelineSpec {
                    label,
                    layout: pl,
                    module: &upload_module,
                    vertex: "vs_fullscreen",
                    fragment: fs,
                    format,
                    blend: None,
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    buffers: &[],
                    depth: false,
                },
            )
        };
        let convert_srgb8 = fullscreen(
            "convert srgb8",
            &convert_float_pl,
            "fs_convert_srgb8",
            COLOR_FORMAT,
        );
        let convert_rgba16 = fullscreen(
            "convert rgba16",
            &convert_uint_pl,
            "fs_convert_rgba16",
            wide_format,
        );
        let convert_float = fullscreen(
            "convert float",
            &convert_float_pl,
            "fs_convert_float",
            wide_format,
        );
        // Only formats the device renders to: a pipeline that targets
        // `WIDE_FORMAT` on a device that cannot render to it is invalid.
        let storage_formats: &[wgpu::TextureFormat] = if wide_format == WIDE_FORMAT {
            &[COLOR_FORMAT, WIDE_FORMAT, MASK_FORMAT]
        } else {
            &[COLOR_FORMAT, MASK_FORMAT]
        };
        let mut mips = HashMap::new();
        for &format in storage_formats {
            mips.insert(format, fullscreen("mip", &mip_pl, "fs_mip", format));
        }
        let mut resample = HashMap::new();
        for &format in storage_formats {
            resample.insert(
                format,
                pipeline(
                    device,
                    &PipelineSpec {
                        label: "resample",
                        layout: &image_layout,
                        module: &image_module,
                        vertex: "vs_quad",
                        fragment: "fs_resample",
                        format,
                        blend: None,
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        buffers: &[],
                        depth: false,
                    },
                ),
            );
        }

        // No anisotropic filtering (`anisotropy_clamp` stays 1). Content is
        // drawn with one uniform scale, so the sampling footprint is square
        // and anisotropic filtering cannot improve on trilinear. Its result
        // is implementation-defined: Mesa lavapipe and llvmpipe pick an
        // axis per 2x2 quad from rounding noise and blur 1:1 content by 8 %
        // of each neighbor along it, and NVIDIA changes the mip blend by up
        // to 31 codes. Without it these drivers, AMD and WARP agree within
        // 3 codes.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("trilinear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });

        Self {
            wide_format,
            convert_float_layout,
            convert_uint_layout,
            mip_layout,
            convert_srgb8,
            convert_rgba16,
            convert_float,
            mips,
            resample,
            frame_layout,
            texture_layout,
            sampler,
            image_module,
            ui_module,
            image_layout,
            display: Mutex::new(HashMap::new()),
        }
    }

    /// The mip pipeline writing `format`.
    pub(crate) fn mip(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.mips[&format]
    }

    /// The resample pipeline writing `format`.
    pub(crate) fn resample(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.resample[&format]
    }

    /// Display pipelines for a color target, built on first use.
    pub(crate) fn display(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Arc<DisplayPipelines> {
        let mut cache = self
            .display
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(found) = cache.get(&format) {
            return Arc::clone(found);
        }
        let image_spec = |depth: bool| PipelineSpec {
            label: "image",
            layout: &self.image_layout,
            module: &self.image_module,
            vertex: "vs_quad",
            fragment: "fs_image",
            format,
            blend: None,
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            buffers: &[],
            depth,
        };
        let image = pipeline(device, &image_spec(false));
        let image_with_depth = pipeline(device, &image_spec(true));
        let ui_layout = pipeline_layout(device, "ui", &[&self.frame_layout, &self.texture_layout]);
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: UI_INSTANCE_SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x4,
                1 => Float32x4,
                2 => Float32x4,
                3 => Float32x4
            ],
        };
        let ui = pipeline(
            device,
            &PipelineSpec {
                label: "ui",
                layout: &ui_layout,
                module: &self.ui_module,
                vertex: "vs_ui",
                fragment: "fs_ui",
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                buffers: &[Some(instance_layout)],
                depth: false,
            },
        );
        let pipelines = Arc::new(DisplayPipelines {
            image,
            image_with_depth,
            ui,
        });
        cache.insert(format, Arc::clone(&pipelines));
        pipelines
    }
}
