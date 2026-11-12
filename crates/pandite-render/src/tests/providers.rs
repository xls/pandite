//! G4: renderer providers are pluggable.

use super::*;
use crate::provider::{
    ContentRenderer, DrawContext, FrameContext, ProviderContext, ProviderRegistry, RenderInput,
    RenderProvider,
};
use crate::{AnimationProvider, StillProvider, DEPTH_FORMAT};
use pandite_core::{Animation, Content, ContentKind, Error, Frame, LoopCount, Size};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Wraps a provider and counts how often it was asked.
struct Spy {
    inner: Arc<dyn RenderProvider>,
    calls: Arc<AtomicUsize>,
}

impl RenderProvider for Spy {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> pandite_core::Result<Box<dyn ContentRenderer>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.create(ctx, input)
    }
}

fn still(color: [u8; 3]) -> Content {
    Content::Still(rgba8(6, 6, |_, _| [color[0], color[1], color[2], 255]))
}

fn anim(color: [u8; 3]) -> Content {
    let frame = Frame {
        image: rgba8(6, 6, |_, _| [color[0], color[1], color[2], 255]),
        delay: Duration::from_millis(40),
    };
    Content::Animation(
        Animation::new(vec![frame.clone(), frame], LoopCount::Infinite).expect("anim"),
    )
}

#[test]
fn provider_registry_dispatches_by_kind() {
    let registry = ProviderRegistry::with_builtin();
    assert_eq!(
        registry.get(ContentKind::Still).map(|p| p.name()),
        Some("still")
    );
    assert_eq!(
        registry.get(ContentKind::Animation).map(|p| p.name()),
        Some("animation")
    );
    assert!(!registry.supports(ContentKind::Scene3d), "3D slot is free");

    let gpu = gpu_or_skip!();
    let mut r = offscreen(&gpu, 24, 24, plain(Color::BLACK));
    let still_calls = Arc::new(AtomicUsize::new(0));
    let anim_calls = Arc::new(AtomicUsize::new(0));
    r.providers_mut().register(
        ContentKind::Still,
        Arc::new(Spy {
            inner: Arc::new(StillProvider),
            calls: Arc::clone(&still_calls),
        }),
    );
    r.providers_mut().register(
        ContentKind::Animation,
        Arc::new(Spy {
            inner: Arc::new(AnimationProvider::default()),
            calls: Arc::clone(&anim_calls),
        }),
    );
    let view = ViewState::default();

    r.set_content(still([250, 10, 10])).expect("still");
    assert_eq!(
        (
            still_calls.load(Ordering::SeqCst),
            anim_calls.load(Ordering::SeqCst)
        ),
        (1, 0)
    );
    assert_eq!(r.content_kind(), Some(ContentKind::Still));
    assert_rgb(
        &frame(&mut r, &view, &DrawList::new()),
        12,
        12,
        [250, 10, 10],
        1,
    );

    r.set_content(anim([10, 250, 10])).expect("animation");
    assert_eq!(
        (
            still_calls.load(Ordering::SeqCst),
            anim_calls.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
    assert_eq!(r.content_kind(), Some(ContentKind::Animation));
    assert_eq!(r.update(Duration::ZERO), Some(Duration::from_millis(40)));
    assert_rgb(
        &frame(&mut r, &view, &DrawList::new()),
        12,
        12,
        [10, 250, 10],
        1,
    );

    // The still provider handles animation content too (first frame), so
    // re-registering it for Animation switches playback off.
    r.providers_mut()
        .register(ContentKind::Animation, Arc::new(StillProvider));
    r.set_content(anim([10, 10, 250]))
        .expect("still for animation");
    assert_eq!(r.update(Duration::from_millis(5)), None);
    assert_rgb(
        &frame(&mut r, &view, &DrawList::new()),
        12,
        12,
        [10, 10, 250],
        1,
    );
    assert_eq!(anim_calls.load(Ordering::SeqCst), 1);

    // A factory made off the render thread dispatches the same way.
    let factory = r.content_factory();
    let prepared =
        std::thread::spawn(move || factory.create(&RenderInput::Content(still([1, 2, 3]))))
            .join()
            .expect("worker")
            .expect("prepared on a worker thread");
    assert_eq!(prepared.kind(), ContentKind::Still);
    assert_eq!(still_calls.load(Ordering::SeqCst), 2);
    r.set_prepared(prepared);
    assert_rgb(
        &frame(&mut r, &view, &DrawList::new()),
        12,
        12,
        [1, 2, 3],
        1,
    );
}

/// What the custom provider observed, shared with the test.
#[derive(Default)]
struct Observed {
    updates: Mutex<Vec<Duration>>,
    depth_format: Mutex<Option<wgpu::TextureFormat>>,
    drawn: AtomicBool,
}

/// A stand-in for a future 3D provider: its own WGSL pipeline with depth
/// test and depth writes, drawing two quads at different depths.
struct SceneProvider {
    observed: Arc<Observed>,
}

struct SceneRenderer {
    observed: Arc<Observed>,
    size: Size,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
}

const SCENE_WGSL: &str = r"
struct Scene { rect: vec4<f32>, screen: vec4<f32> };
@group(0) @binding(0) var<uniform> scene: Scene;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> Out {
    let corner = vec2<f32>(f32(v & 1u), f32((v >> 1u) & 1u));
    // Instance 0: near quad over the left half. Instance 1: far quad over
    // all of it, drawn later; the depth test must keep the near one.
    var width = scene.rect.z * 0.5;
    var depth = 0.25;
    var color = vec4<f32>(1.0, 0.5, 0.0, 1.0);
    if (i == 1u) {
        width = scene.rect.z;
        depth = 0.75;
        color = vec4<f32>(0.0, 0.25, 1.0, 1.0);
    }
    let p = scene.rect.xy + corner * vec2<f32>(width, scene.rect.w);
    let ndc = p / scene.screen.xy * 2.0 - vec2<f32>(1.0);
    var out: Out;
    out.pos = vec4<f32>(ndc.x, -ndc.y, depth, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return in.color;
}
";

impl RenderProvider for SceneProvider {
    fn name(&self) -> &'static str {
        "test scene"
    }

    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> pandite_core::Result<Box<dyn ContentRenderer>> {
        let RenderInput::Custom { kind, size, data } = input else {
            return Err(Error::Unsupported("scene data expected".into()));
        };
        assert_eq!(*kind, ContentKind::Scene3d);
        assert!(
            data.downcast_ref::<&str>().is_some(),
            "provider data arrives intact"
        );
        let device = ctx.gpu.device();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test scene"),
            source: wgpu::ShaderSource::Wgsl(SCENE_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test scene"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: ctx.target.depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.target.color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Ok(Box::new(SceneRenderer {
            observed: Arc::clone(&self.observed),
            size: *size,
            pipeline,
            uniform,
            group,
        }))
    }
}

impl ContentRenderer for SceneRenderer {
    fn size(&self) -> Size {
        self.size
    }

    fn update(&mut self, dt: Duration) -> Option<Duration> {
        self.observed.updates.lock().expect("lock").push(dt);
        // A turntable animation redraws every display frame.
        Some(Duration::from_millis(16))
    }

    fn uses_depth(&self) -> bool {
        true
    }

    fn prepare(&mut self, ctx: &mut FrameContext<'_>) -> pandite_core::Result<()> {
        let r = ctx.placement.image_rect;
        let words: [f32; 8] = [
            r.x,
            r.y,
            r.w,
            r.h,
            ctx.target.width as f32,
            ctx.target.height as f32,
            0.0,
            0.0,
        ];
        ctx.gpu
            .queue()
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&words));
        Ok(())
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        *self.observed.depth_format.lock().expect("lock") = ctx.depth_format;
        self.observed.drawn.store(true, Ordering::SeqCst);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..4, 0..2);
    }
}

#[test]
fn custom_provider_is_drawn() {
    let gpu = gpu_or_skip!();
    let mut r = offscreen(&gpu, 64, 48, plain(Color::BLACK));
    let observed = Arc::new(Observed::default());
    let replaced = r.providers_mut().register(
        ContentKind::Scene3d,
        Arc::new(SceneProvider {
            observed: Arc::clone(&observed),
        }),
    );
    assert!(replaced.is_none());
    r.set_input(&RenderInput::Custom {
        kind: ContentKind::Scene3d,
        size: Size::new(40.0, 20.0),
        data: Arc::new("teapot"),
    })
    .expect("custom provider chosen");
    assert_eq!(r.content_kind(), Some(ContentKind::Scene3d));
    assert_eq!(r.content_size(), Some(Size::new(40.0, 20.0)));
    assert_eq!(
        r.update(Duration::from_millis(7)),
        Some(Duration::from_millis(16))
    );
    assert_eq!(
        *observed.updates.lock().expect("lock"),
        vec![Duration::from_millis(7)]
    );

    // A label on top proves the interface pass still runs after a depth pass.
    let mut ui = DrawList::new();
    ui.rect(Rect::new(0.0, 0.0, 4.0, 4.0), Color::WHITE, 0.0);
    let view = ViewState::default();
    let px = frame(&mut r, &view, &ui);
    assert!(observed.drawn.load(Ordering::SeqCst));
    assert_eq!(
        *observed.depth_format.lock().expect("lock"),
        Some(DEPTH_FORMAT)
    );
    // 40 x 20 centered in 64 x 48: x 12..52, y 14..34.
    let orange = [255, srgb_code(0.5), 0];
    let blue = [0, srgb_code(0.25), 255];
    assert_rgb(&px, 13, 15, orange, 1);
    assert_rgb(&px, 31, 30, orange, 1);
    // The far quad was drawn after the near one but lost the depth test.
    assert_rgb(&px, 33, 20, blue, 1);
    assert_rgb(&px, 51, 33, blue, 1);
    assert_rgb(&px, 11, 20, [0, 0, 0], 0);
    assert_rgb(&px, 2, 2, [255, 255, 255], 0);

    // The built-in providers were not touched.
    assert_eq!(
        r.providers().get(ContentKind::Still).map(|p| p.name()),
        Some("still")
    );
    assert_eq!(
        r.providers().get(ContentKind::Animation).map(|p| p.name()),
        Some("animation")
    );
    r.set_content(still([9, 99, 199])).expect("still");
    let px = frame(&mut r, &view, &DrawList::new());
    assert_rgb(&px, 32, 24, [9, 99, 199], 1);
}

#[test]
fn unknown_kind_is_unsupported_error() {
    let scene = RenderInput::Custom {
        kind: ContentKind::Scene3d,
        size: Size::new(10.0, 10.0),
        data: Arc::new(()),
    };
    let gpu = gpu_or_skip!();
    let mut r = offscreen(&gpu, 16, 16, plain(Color::BLACK));
    r.set_content(still([200, 0, 0])).expect("still");
    match r.set_input(&scene) {
        Err(Error::Unsupported(msg)) => assert!(msg.contains("Scene3d"), "{msg}"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
    // The failed call kept the previous content.
    assert_eq!(r.content_kind(), Some(ContentKind::Still));
    assert_rgb(
        &frame(&mut r, &ViewState::default(), &DrawList::new()),
        8,
        8,
        [200, 0, 0],
        1,
    );

    // An empty registry supports nothing, not even stills.
    *r.providers_mut() = ProviderRegistry::new();
    assert!(matches!(
        r.set_content(still([1, 1, 1])),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        r.content_factory()
            .create(&RenderInput::Content(still([1, 1, 1]))),
        Err(Error::Unsupported(_))
    ));
    // A provider handed the wrong data says so instead of drawing garbage.
    r.providers_mut()
        .register(ContentKind::Scene3d, Arc::new(StillProvider));
    assert!(matches!(r.set_input(&scene), Err(Error::Unsupported(_))));
}
