//! Render a few interface screens to PNG files with a tiny software
//! rasterizer, for layout review without a GPU. Text is drawn as one block
//! per character (no font), icons with their real masks, thumbnails as
//! gradients.
//!
//! Usage: `cargo run -p pandite-ui --example preview -- <out-dir>`

use pandite_core::{
    Color, DrawCmd, DrawList, Point, Rect, Size, TextMeasure, TextureId, ViewState,
};
use pandite_ui::{
    thumbnail_index, EncoderInfo, Icon, ImageInfo, InputEvent, Key, Loading, Modifiers,
    PointerButton, ThemeKind, ThumbState, Ui, UiContext,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Measure;

impl TextMeasure for Measure {
    fn measure(&self, text: &str, size: f32) -> Size {
        Size::new(text.chars().count() as f32 * size * 0.55, size * 1.3)
    }
}

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
    clip: Option<Rect>,
}

fn sd_box(px: f32, py: f32, r: Rect, radius: f32) -> f32 {
    let radius = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let qx = (px - (r.x + r.w * 0.5)).abs() - (r.w * 0.5 - radius);
    let qy = (py - (r.y + r.h * 0.5)).abs() - (r.h * 0.5 - radius);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius
}

impl Canvas {
    fn new(w: usize, h: usize, bg: Color) -> Self {
        let c = [f32::from(bg.r), f32::from(bg.g), f32::from(bg.b)];
        Self {
            w,
            h,
            px: vec![c; w * h],
            clip: None,
        }
    }

    fn bounds(&self, r: Rect) -> Option<(usize, usize, usize, usize)> {
        let mut area = Rect::new(0.0, 0.0, self.w as f32, self.h as f32);
        if let Some(c) = self.clip {
            area = area.intersect(c)?;
        }
        let r = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0).intersect(area)?;
        Some((
            r.x.floor() as usize,
            r.y.floor() as usize,
            (r.right().ceil() as usize).min(self.w),
            (r.bottom().ceil() as usize).min(self.h),
        ))
    }

    fn blend(&mut self, x: usize, y: usize, c: Color, a: f32) {
        let a = a * f32::from(c.a) / 255.0;
        let p = &mut self.px[y * self.w + x];
        p[0] += (f32::from(c.r) - p[0]) * a;
        p[1] += (f32::from(c.g) - p[1]) * a;
        p[2] += (f32::from(c.b) - p[2]) * a;
    }

    fn shape(&mut self, r: Rect, c: Color, cover: impl Fn(f32, f32) -> f32) {
        let Some((x0, y0, x1, y1)) = self.bounds(r) else {
            return;
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let a = cover(x as f32 + 0.5, y as f32 + 0.5);
                if a > 0.0 {
                    self.blend(x, y, c, a.min(1.0));
                }
            }
        }
    }

    fn draw(&mut self, list: &DrawList) {
        for cmd in &list.cmds {
            match cmd {
                DrawCmd::Rect {
                    rect,
                    color,
                    radius,
                } => {
                    let (r, rad) = (*rect, *radius);
                    self.shape(r, *color, |x, y| {
                        (0.5 - sd_box(x, y, r, rad)).clamp(0.0, 1.0)
                    });
                }
                DrawCmd::Border {
                    rect,
                    color,
                    width,
                    radius,
                } => {
                    let (r, rad, w) = (*rect, *radius, *width);
                    self.shape(r, *color, |x, y| {
                        let d = sd_box(x, y, r, rad);
                        let outer = (0.5 - d).clamp(0.0, 1.0);
                        let inner = (0.5 - (d + w)).clamp(0.0, 1.0);
                        outer - inner
                    });
                }
                DrawCmd::Image {
                    rect,
                    texture,
                    tint,
                    ..
                } => self.image(*rect, *texture, *tint),
                DrawCmd::Text {
                    pos,
                    text,
                    size,
                    color,
                } => {
                    let adv = size * 0.55;
                    for (i, ch) in text.chars().enumerate() {
                        if ch == ' ' {
                            continue;
                        }
                        let tall = ch.is_ascii_uppercase() || ch.is_ascii_digit();
                        let top = if tall { 0.25 } else { 0.42 };
                        let r = Rect::new(
                            pos.x + i as f32 * adv + adv * 0.12,
                            pos.y + size * top,
                            adv * 0.72,
                            size * (0.95 - top),
                        );
                        self.shape(r, *color, |x, y| {
                            (0.5 - sd_box(x, y, r, 1.0)).clamp(0.0, 1.0) * 0.85
                        });
                    }
                }
                DrawCmd::Clip(c) => self.clip = *c,
                _ => {}
            }
        }
    }

    fn image(&mut self, r: Rect, id: TextureId, tint: Color) {
        if let Some(icon) = Icon::from_texture_id(id) {
            let size = r.w.round().max(1.0) as u32;
            let mask = icon.coverage(size);
            self.shape(r, tint, |x, y| {
                let ix = (x - r.x).floor() as i64;
                let iy = (y - r.y).floor() as i64;
                if ix < 0 || iy < 0 || ix >= i64::from(size) || iy >= i64::from(size) {
                    0.0
                } else {
                    f32::from(mask[(iy as u32 * size + ix as u32) as usize]) / 255.0
                }
            });
        } else if let Some(i) = thumbnail_index(id) {
            let hue = (i * 47 % 255) as u8;
            self.shape(r, Color::rgb(hue, 120, 255 - hue), |x, y| {
                if r.contains(Point::new(x, y)) {
                    0.4 + 0.6 * ((y - r.y) / r.h)
                } else {
                    0.0
                }
            });
        } else {
            // The application's image: a soft landscape gradient.
            let rr = r;
            self.shape(r, Color::rgb(90, 140, 190), |x, y| {
                f32::from(rr.contains(Point::new(x, y)))
            });
            let ground = Rect::new(r.x, r.y + r.h * 0.6, r.w, r.h * 0.4);
            self.shape(ground, Color::rgb(70, 110, 60), |x, y| {
                f32::from(ground.contains(Point::new(x, y)))
            });
        }
    }

    fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity((self.w * 3 + 1) * self.h);
        for y in 0..self.h {
            raw.push(0);
            for x in 0..self.w {
                for c in self.px[y * self.w + x] {
                    raw.push(c.round().clamp(0.0, 255.0) as u8);
                }
            }
        }
        encode_png(self.w as u32, self.h as u32, &raw)
    }
}

fn encode_png(w: u32, h: u32, raw: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(block) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &v in raw {
        a = (a + u32::from(v)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in &out[start..] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

const ENCODERS: &[EncoderInfo<'static>] = &[
    EncoderInfo {
        id: "png",
        name: "PNG",
        has_quality: false,
    },
    EncoderInfo {
        id: "jpeg",
        name: "JPEG",
        has_quality: true,
    },
    EncoderInfo {
        id: "webp",
        name: "WebP",
        has_quality: true,
    },
    EncoderInfo {
        id: "bmp",
        name: "BMP",
        has_quality: false,
    },
];

fn context(
    theme: ThemeKind,
    w: f32,
    h: f32,
    scale: f32,
    thumbs: &'static [ThumbState],
) -> UiContext<'static> {
    let mut ctx = UiContext {
        window_size: Size::new(w, h),
        scale_factor: scale,
        theme,
        count: thumbs.len(),
        index: 6,
        thumbnails: thumbs,
        encoders: ENCODERS,
        folder: Some(Path::new("C:/Users/me/Pictures/Holiday")),
        can_undo: true,
        ..UiContext::default()
    };
    let content = Ui::new().content_rect(&ctx);
    let view = ViewState::default();
    let size = Size::new(4032.0, 3024.0);
    let s = view.scale(size, content.size());
    ctx.image = Some(ImageInfo {
        file_name: "IMG_20260712_181502_beach_sunset.jpg",
        width: 4032,
        height: 3024,
        view,
        zoom_percent: s * 100.0,
        screen_rect: view.image_rect(size, content),
        edited: true,
    });
    ctx
}

fn render(path: PathBuf, ui: &mut Ui, ctx: &UiContext<'_>) {
    let theme = pandite_ui::Theme::for_kind(ctx.theme);
    let mut canvas = Canvas::new(
        ctx.window_size.w as usize,
        ctx.window_size.h as usize,
        theme.canvas,
    );
    if let Some(info) = ctx.image {
        canvas.image(info.screen_rect, TextureId(1), Color::WHITE);
    }
    let mut list = DrawList::new();
    ui.build(ctx, &Measure, &mut list);
    canvas.draw(&list);
    std::fs::write(&path, canvas.png()).expect("write preview");
    println!("wrote {}", path.display());
}

fn ev(ui: &mut Ui, ctx: &UiContext<'_>, e: InputEvent) {
    ui.handle_event(e, ctx);
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let mut thumbs = vec![
        ThumbState::Ready {
            width: 160,
            height: 120
        };
        40
    ];
    thumbs[9] = ThumbState::Pending;
    thumbs[10] = ThumbState::NotDownloaded;
    thumbs[11] = ThumbState::Failed;
    let thumbs: &'static [ThumbState] = Box::leak(thumbs.into_boxed_slice());

    // Main view, hovering a toolbar button long enough for its tooltip.
    let ctx = context(ThemeKind::Dark, 1280.0, 800.0, 1.0, thumbs);
    let mut ui = Ui::new();
    ev(
        &mut ui,
        &ctx,
        InputEvent::PointerMove {
            pos: Point::new(275.0, 20.0),
        },
    );
    let mut later = ctx;
    later.now = Duration::from_millis(900);
    render(dir.join("main-dark.png"), &mut ui, &later);

    let mut light = context(ThemeKind::Light, 1280.0, 800.0, 1.0, thumbs);
    light.now = Duration::from_millis(900);
    let mut ui = Ui::new();
    ev(
        &mut ui,
        &light,
        InputEvent::PointerMove {
            pos: Point::new(275.0, 20.0),
        },
    );
    ui.toast("Path copied", Duration::ZERO);
    render(dir.join("main-light.png"), &mut ui, &light);

    // Context menu.
    let mut ui = Ui::new();
    let p = Point::new(700.0, 90.0);
    ev(&mut ui, &ctx, InputEvent::PointerMove { pos: p });
    ev(
        &mut ui,
        &ctx,
        InputEvent::PointerDown {
            pos: p,
            button: PointerButton::Right,
        },
    );
    ev(
        &mut ui,
        &ctx,
        InputEvent::PointerUp {
            pos: p,
            button: PointerButton::Right,
        },
    );
    ev(
        &mut ui,
        &ctx,
        InputEvent::PointerMove {
            pos: Point::new(760.0, 270.0),
        },
    );
    render(dir.join("menu.png"), &mut ui, &ctx);

    // Resize dialog in exact mode.
    let mut ui = Ui::new();
    let key = |ui: &mut Ui, k: Key, mods: Modifiers| {
        ui.handle_event(
            InputEvent::KeyDown {
                key: k,
                mods,
                repeat: false,
            },
            &ctx,
        );
    };
    key(&mut ui, Key::R, Modifiers::CTRL);
    key(&mut ui, Key::Tab, Modifiers::SHIFT);
    key(&mut ui, Key::Right, Modifiers::NONE);
    key(&mut ui, Key::Right, Modifiers::NONE);
    key(&mut ui, Key::Tab, Modifiers::NONE);
    ev(&mut ui, &ctx, InputEvent::Text("2000".into()));
    render(dir.join("resize.png"), &mut ui, &ctx);

    // Bulk dialog with inputs.
    let mut ui = Ui::new();
    key(&mut ui, Key::B, Modifiers::CTRL);
    ui.set_bulk_folder(PathBuf::from("C:/Users/me/Pictures/Holiday"));
    render(dir.join("bulk.png"), &mut ui, &ctx);

    // Crop mode with a selection.
    let mut ui = Ui::new();
    key(&mut ui, Key::C, Modifiers::NONE);
    let a = Point::new(400.0, 200.0);
    let b = Point::new(900.0, 520.0);
    ev(&mut ui, &ctx, InputEvent::PointerMove { pos: a });
    ev(
        &mut ui,
        &ctx,
        InputEvent::PointerDown {
            pos: a,
            button: PointerButton::Left,
        },
    );
    ev(&mut ui, &ctx, InputEvent::PointerMove { pos: b });
    ev(
        &mut ui,
        &ctx,
        InputEvent::PointerUp {
            pos: b,
            button: PointerButton::Left,
        },
    );
    render(dir.join("crop.png"), &mut ui, &ctx);

    // First-screen splash for a cloud file, and the small overlay.
    let mut splash = ctx;
    splash.image = None;
    splash.loading = Some(Loading {
        file_name: "IMG_20260712_181502_beach_sunset.jpg",
        elapsed: Duration::from_secs(14),
        progress: Some(0.37),
        cloud: true,
        over_image: false,
    });
    render(dir.join("splash.png"), &mut Ui::new(), &splash);
    let mut over = ctx;
    over.loading = Some(Loading {
        over_image: true,
        progress: None,
        ..splash.loading.unwrap()
    });
    render(dir.join("loading.png"), &mut Ui::new(), &over);

    // Empty first run, and a narrow window at 150% scale.
    let mut empty = context(ThemeKind::Dark, 1000.0, 640.0, 1.0, &[]);
    empty.image = None;
    empty.folder = None;
    render(dir.join("empty.png"), &mut Ui::new(), &empty);
    let narrow = context(ThemeKind::Dark, 640.0, 520.0, 1.5, thumbs);
    render(dir.join("narrow-150.png"), &mut Ui::new(), &narrow);
}
