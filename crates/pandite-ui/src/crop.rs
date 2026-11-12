//! Crop mode: a selection rectangle in image pixel coordinates.
//!
//! The selection is stored in image pixels (floating point while dragging)
//! so zooming and panning in crop mode never disturb it. Screen positions
//! map through the image's on-screen rectangle from the context. Every
//! change is clamped to the image.

use crate::action::PixelRect;
use crate::context::ImageInfo;
use crate::input::CursorShape;
use pandite_core::{Point, Rect};

/// One of the eight resize handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::NW,
        Handle::N,
        Handle::NE,
        Handle::E,
        Handle::SE,
        Handle::S,
        Handle::SW,
        Handle::W,
    ];

    /// Which edges the handle moves: (left, top, right, bottom).
    fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Handle::N => (false, true, false, false),
            Handle::S => (false, false, false, true),
            Handle::E => (false, false, true, false),
            Handle::W => (true, false, false, false),
            Handle::NE => (false, true, true, false),
            Handle::NW => (true, true, false, false),
            Handle::SE => (false, false, true, true),
            Handle::SW => (true, false, false, true),
        }
    }

    /// The handle's center on a screen rectangle.
    pub fn center(self, r: Rect) -> Point {
        let (l, t, rt, b) = self.edges();
        let x = if l {
            r.x
        } else if rt {
            r.right()
        } else {
            r.x + r.w * 0.5
        };
        let y = if t {
            r.y
        } else if b {
            r.bottom()
        } else {
            r.y + r.h * 0.5
        };
        Point::new(x, y)
    }

    pub fn is_corner(self) -> bool {
        matches!(self, Handle::NE | Handle::NW | Handle::SE | Handle::SW)
    }

    pub fn cursor(self) -> CursorShape {
        match self {
            Handle::N | Handle::S => CursorShape::ResizeVertical,
            Handle::E | Handle::W => CursorShape::ResizeHorizontal,
            Handle::NW | Handle::SE => CursorShape::ResizeDiagonalDown,
            Handle::NE | Handle::SW => CursorShape::ResizeDiagonalUp,
        }
    }
}

/// What a press in crop mode grabbed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CropHit {
    Handle(Handle),
    Inside,
    Outside,
}

/// An active crop drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CropDrag {
    /// Drawing a new selection from this image point.
    New { anchor: Point },
    /// Moving the selection; `grab` is the image point first pressed.
    Move { start: Rect, grab: Point },
    /// Resizing with a handle.
    Resize { handle: Handle, start: Rect },
}

/// Crop mode state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CropState {
    /// Selection in image pixels.
    pub selection: Option<Rect>,
    pub drag: Option<CropDrag>,
    /// The image size the selection belongs to.
    pub image: (u32, u32),
}

/// Screen to image mapping for one image placement.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mapping {
    rect: Rect,
    sx: f32,
    sy: f32,
    w: f32,
    h: f32,
}

impl Mapping {
    pub fn new(info: &ImageInfo<'_>) -> Option<Self> {
        let r = info.screen_rect;
        if info.width == 0 || info.height == 0 || r.is_empty() {
            return None;
        }
        Some(Self {
            rect: r,
            sx: info.width as f32 / r.w,
            sy: info.height as f32 / r.h,
            w: info.width as f32,
            h: info.height as f32,
        })
    }

    /// A window point as image pixels, clamped to the image.
    pub fn to_image(self, p: Point) -> Point {
        Point::new(
            ((p.x - self.rect.x) * self.sx).clamp(0.0, self.w),
            ((p.y - self.rect.y) * self.sy).clamp(0.0, self.h),
        )
    }

    /// An image rectangle in window pixels.
    pub fn to_screen(self, r: Rect) -> Rect {
        Rect::new(
            self.rect.x + r.x / self.sx,
            self.rect.y + r.y / self.sy,
            r.w / self.sx,
            r.h / self.sy,
        )
    }

    pub fn image_size(&self) -> (f32, f32) {
        (self.w, self.h)
    }
}

impl CropState {
    pub fn new(image: (u32, u32)) -> Self {
        Self {
            selection: None,
            drag: None,
            image,
        }
    }

    /// What lies under window point `p`. Corners win over sides, sides over
    /// the inside, so tiny selections stay resizable.
    pub fn hit(&self, p: Point, map: &Mapping, handle_hit: f32) -> CropHit {
        let Some(sel) = self.selection else {
            return CropHit::Outside;
        };
        let screen = map.to_screen(sel);
        let half = handle_hit * 0.5;
        let near = |c: Point| (p.x - c.x).abs() <= half && (p.y - c.y).abs() <= half;
        for h in Handle::ALL.iter().filter(|h| h.is_corner()) {
            if near(h.center(screen)) {
                return CropHit::Handle(*h);
            }
        }
        // Side handles cover the whole edge, not only their square.
        let inside_x = p.x >= screen.x && p.x <= screen.right();
        let inside_y = p.y >= screen.y && p.y <= screen.bottom();
        if inside_x && (p.y - screen.y).abs() <= half {
            return CropHit::Handle(Handle::N);
        }
        if inside_x && (p.y - screen.bottom()).abs() <= half {
            return CropHit::Handle(Handle::S);
        }
        if inside_y && (p.x - screen.x).abs() <= half {
            return CropHit::Handle(Handle::W);
        }
        if inside_y && (p.x - screen.right()).abs() <= half {
            return CropHit::Handle(Handle::E);
        }
        if screen.contains(p) {
            CropHit::Inside
        } else {
            CropHit::Outside
        }
    }

    /// Begin a drag at window point `p`.
    pub fn begin(&mut self, p: Point, map: &Mapping, handle_hit: f32) {
        let ip = map.to_image(p);
        self.drag = Some(match (self.hit(p, map, handle_hit), self.selection) {
            (CropHit::Handle(handle), Some(start)) => CropDrag::Resize { handle, start },
            (CropHit::Inside, Some(start)) => CropDrag::Move { start, grab: ip },
            _ => CropDrag::New { anchor: ip },
        });
    }

    /// Continue the drag with the pointer at `p`. `square` keeps new
    /// selections and corner drags square.
    pub fn drag_to(&mut self, p: Point, map: &Mapping, square: bool) {
        let Some(drag) = self.drag else { return };
        let (w, h) = map.image_size();
        let ip = map.to_image(p);
        let sel = match drag {
            CropDrag::New { anchor } => {
                let mut end = ip;
                if square {
                    end = square_end(anchor, ip, w, h);
                }
                Rect::from_corners(anchor, end)
            }
            CropDrag::Move { start, grab } => {
                let dx = ip.x - grab.x;
                let dy = ip.y - grab.y;
                Rect::new(
                    (start.x + dx).clamp(0.0, (w - start.w).max(0.0)),
                    (start.y + dy).clamp(0.0, (h - start.h).max(0.0)),
                    start.w,
                    start.h,
                )
            }
            CropDrag::Resize { handle, start } => {
                let (l, t, r, b) = handle.edges();
                let mut x0 = start.x;
                let mut y0 = start.y;
                let mut x1 = start.right();
                let mut y1 = start.bottom();
                if l {
                    x0 = ip.x;
                }
                if r {
                    x1 = ip.x;
                }
                if t {
                    y0 = ip.y;
                }
                if b {
                    y1 = ip.y;
                }
                if square && handle.is_corner() {
                    let anchor = Point::new(if l { x1 } else { x0 }, if t { y1 } else { y0 });
                    let end = square_end(anchor, ip, w, h);
                    Rect::from_corners(anchor, end)
                } else {
                    Rect::from_corners(Point::new(x0, y0), Point::new(x1, y1))
                }
            }
        };
        self.selection = Some(clamp_rect(sel, w, h));
    }

    /// End the drag. A selection smaller than one image pixel is dropped.
    pub fn end(&mut self) {
        self.drag = None;
        if let Some(sel) = self.selection {
            if sel.w < 1.0 || sel.h < 1.0 {
                self.selection = None;
            }
        }
    }

    /// Select the whole image.
    pub fn select_all(&mut self) {
        self.selection = Some(Rect::new(
            0.0,
            0.0,
            self.image.0 as f32,
            self.image.1 as f32,
        ));
    }

    /// Move the selection by whole image pixels, staying inside the image.
    pub fn nudge(&mut self, dx: f32, dy: f32) {
        let (w, h) = (self.image.0 as f32, self.image.1 as f32);
        if let Some(sel) = self.selection.as_mut() {
            sel.x = (sel.x + dx).clamp(0.0, (w - sel.w).max(0.0));
            sel.y = (sel.y + dy).clamp(0.0, (h - sel.h).max(0.0));
        }
    }

    /// The selection rounded to whole pixels, when it is at least 1 x 1.
    pub fn pixel_rect(&self) -> Option<PixelRect> {
        let sel = self.selection?;
        let (w, h) = self.image;
        if w == 0 || h == 0 {
            return None;
        }
        let x0 = (sel.x.round().max(0.0) as u32).min(w - 1);
        let y0 = (sel.y.round().max(0.0) as u32).min(h - 1);
        let x1 = (sel.right().round().max(0.0) as u32).min(w);
        let y1 = (sel.bottom().round().max(0.0) as u32).min(h);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }
}

fn square_end(anchor: Point, p: Point, w: f32, h: f32) -> Point {
    let dx = p.x - anchor.x;
    let dy = p.y - anchor.y;
    let mut side = dx.abs().max(dy.abs());
    // Limit the side so the square stays inside the image.
    let room_x = if dx >= 0.0 { w - anchor.x } else { anchor.x };
    let room_y = if dy >= 0.0 { h - anchor.y } else { anchor.y };
    side = side.min(room_x).min(room_y);
    Point::new(anchor.x + side * dx.signum(), anchor.y + side * dy.signum())
}

fn clamp_rect(r: Rect, w: f32, h: f32) -> Rect {
    let x0 = r.x.clamp(0.0, w);
    let y0 = r.y.clamp(0.0, h);
    let x1 = r.right().clamp(0.0, w);
    let y1 = r.bottom().clamp(0.0, h);
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::ViewState;

    fn info() -> ImageInfo<'static> {
        ImageInfo {
            file_name: "a.png",
            width: 1000,
            height: 500,
            view: ViewState::default(),
            zoom_percent: 50.0,
            screen_rect: Rect::new(100.0, 100.0, 500.0, 250.0),
            edited: false,
        }
    }

    #[test]
    fn mapping_round_trips_and_clamps() {
        let map = Mapping::new(&info()).unwrap();
        assert_eq!(map.to_image(Point::new(100.0, 100.0)), Point::new(0.0, 0.0));
        assert_eq!(
            map.to_image(Point::new(350.0, 225.0)),
            Point::new(500.0, 250.0)
        );
        assert_eq!(
            map.to_image(Point::new(-50.0, 900.0)),
            Point::new(0.0, 500.0)
        );
        let r = map.to_screen(Rect::new(0.0, 0.0, 1000.0, 500.0));
        assert_eq!(r, Rect::new(100.0, 100.0, 500.0, 250.0));
    }

    #[test]
    fn square_drag_and_tiny_selection() {
        let map = Mapping::new(&info()).unwrap();
        let mut crop = CropState::new((1000, 500));
        crop.begin(Point::new(150.0, 150.0), &map, 18.0);
        crop.drag_to(Point::new(300.0, 175.0), &map, true);
        let sel = crop.selection.unwrap();
        assert_eq!(sel.w, sel.h);
        crop.end();
        crop.begin(Point::new(500.0, 300.0), &map, 18.0);
        crop.drag_to(Point::new(500.2, 300.2), &map, false);
        crop.end();
        assert!(crop.selection.is_none(), "sub-pixel selection is dropped");
    }

    #[test]
    fn nudge_and_select_all_stay_inside() {
        let mut crop = CropState::new((100, 50));
        crop.select_all();
        crop.nudge(10.0, 10.0);
        assert_eq!(crop.selection, Some(Rect::new(0.0, 0.0, 100.0, 50.0)));
        assert_eq!(
            crop.pixel_rect(),
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 100,
                height: 50
            })
        );
    }
}
