//! Pop-up menus: the context menu, the toolbar overflow menu and drop-down
//! lists in dialogs.
//!
//! A menu has a fixed width and fixed item heights, so its layout is a pure
//! function of its items, its origin and the window. A menu taller than the
//! window is clamped to the window and scrolls with the wheel or the
//! keyboard.

use crate::command::Command;
use crate::metrics::Metrics;
use pandite_core::{Point, Rect};

/// What choosing an item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuPick {
    /// Run a command.
    Command(Command),
    /// Pick entry `n` of a dialog drop-down.
    Choice(usize),
}

/// One menu row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MenuItem {
    /// A selectable entry.
    Entry {
        label: String,
        shortcut: Option<String>,
        enabled: bool,
        checked: bool,
        pick: MenuPick,
    },
    /// A thin divider.
    Separator,
}

impl MenuItem {
    pub fn selectable(&self) -> bool {
        matches!(self, MenuItem::Entry { enabled: true, .. })
    }
}

/// Who opened the menu, which decides what a pick means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MenuOwner {
    /// Right click or the menu key. `anchor` is where it opened, used to
    /// zoom at that point.
    Context { anchor: Option<Point> },
    /// The toolbar overflow button.
    Overflow,
    /// A drop-down in the bulk dialog.
    FormatList,
}

/// An open menu.
#[derive(Clone, Debug)]
pub(crate) struct MenuState {
    pub owner: MenuOwner,
    pub origin: Point,
    pub items: Vec<MenuItem>,
    pub hover: Option<usize>,
    pub scroll: f32,
    /// Minimum width, for drop-downs that match their button.
    pub min_w: f32,
}

impl MenuState {
    pub fn new(owner: MenuOwner, origin: Point, items: Vec<MenuItem>) -> Self {
        Self {
            owner,
            origin,
            items,
            hover: None,
            scroll: 0.0,
            min_w: 0.0,
        }
    }

    fn item_h(item: &MenuItem, m: &Metrics) -> f32 {
        match item {
            MenuItem::Entry { .. } => m.menu_item_h(),
            MenuItem::Separator => m.menu_sep_h(),
        }
    }

    fn content_h(&self, m: &Metrics) -> f32 {
        self.items.iter().map(|i| Self::item_h(i, m)).sum::<f32>() + 2.0 * m.px(4.0)
    }

    /// The menu panel inside `window`.
    pub fn rect(&self, window: Rect, m: &Metrics) -> Rect {
        let w = m.menu_w().max(self.min_w).min(window.w.max(1.0));
        let h = self.content_h(m).min(window.h.max(1.0));
        let mut x = self.origin.x;
        let mut y = self.origin.y;
        if x + w > window.right() {
            // Open to the left of the pointer when there is no room.
            x = (self.origin.x - w).max(window.x);
            if x + w > window.right() {
                x = (window.right() - w).max(window.x);
            }
        }
        if y + h > window.bottom() {
            y = (self.origin.y - h).max(window.y);
            if y + h > window.bottom() {
                y = (window.bottom() - h).max(window.y);
            }
        }
        Rect::new(x.round(), y.round(), w, h)
    }

    /// Largest scroll offset.
    pub fn max_scroll(&self, window: Rect, m: &Metrics) -> f32 {
        (self.content_h(m) - self.rect(window, m).h).max(0.0)
    }

    /// Item rectangles in screen space (some may lie outside the panel
    /// when it scrolls). Calls `f` with index and rectangle.
    pub fn for_each_item(&self, window: Rect, m: &Metrics, mut f: impl FnMut(usize, Rect)) {
        let panel = self.rect(window, m);
        let mut y = panel.y + m.px(4.0) - self.scroll;
        for (i, item) in self.items.iter().enumerate() {
            let h = Self::item_h(item, m);
            f(i, Rect::new(panel.x, y, panel.w, h));
            y += h;
        }
    }

    /// The item under `p`. `None` outside the panel or over a separator.
    pub fn hit(&self, p: Point, window: Rect, m: &Metrics) -> Option<usize> {
        let panel = self.rect(window, m);
        if !panel.contains(p) {
            return None;
        }
        let mut found = None;
        self.for_each_item(window, m, |i, r| {
            if r.contains(p) && matches!(self.items[i], MenuItem::Entry { .. }) {
                found = Some(i);
            }
        });
        found
    }

    /// Move the hover to the next selectable item in direction `dir`
    /// (+1 or -1), wrapping around.
    pub fn step(&mut self, dir: i32) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let mut i = match self.hover {
            Some(h) => h as i32,
            None if dir > 0 => -1,
            None => n as i32,
        };
        for _ in 0..n {
            i = (i + dir).rem_euclid(n as i32);
            if self.items[i as usize].selectable() {
                self.hover = Some(i as usize);
                return;
            }
        }
    }

    /// Move the hover to the first (`dir` > 0) or last selectable item.
    pub fn jump(&mut self, first: bool) {
        self.hover = None;
        self.step(if first { 1 } else { -1 });
    }

    /// Scroll so the hovered item is inside the panel.
    pub fn scroll_to_hover(&mut self, window: Rect, m: &Metrics) {
        let Some(h) = self.hover else { return };
        let panel = self.rect(window, m);
        let mut top = m.px(4.0);
        for item in &self.items[..h] {
            top += Self::item_h(item, m);
        }
        let bottom = top + Self::item_h(&self.items[h], m);
        if top - self.scroll < 0.0 {
            self.scroll = top - m.px(4.0);
        } else if bottom - self.scroll > panel.h {
            self.scroll = bottom + m.px(4.0) - panel.h;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(window, m));
    }

    /// The pick of item `i`, when it is enabled.
    pub fn pick(&self, i: usize) -> Option<MenuPick> {
        match self.items.get(i)? {
            MenuItem::Entry {
                enabled: true,
                pick,
                ..
            } => Some(*pick),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize, enabled: bool) -> MenuItem {
        MenuItem::Entry {
            label: format!("item {n}"),
            shortcut: None,
            enabled,
            checked: false,
            pick: MenuPick::Choice(n),
        }
    }

    #[test]
    fn menu_stays_inside_window_and_scrolls_when_tall() {
        let m = Metrics::new(1.0);
        let window = Rect::new(0.0, 0.0, 400.0, 200.0);
        let items: Vec<MenuItem> = (0..20).map(|i| entry(i, true)).collect();
        let mut menu = MenuState::new(MenuOwner::Overflow, Point::new(390.0, 190.0), items);
        let r = menu.rect(window, &m);
        assert!(r.x >= 0.0 && r.right() <= 400.0);
        assert!(r.y >= 0.0 && r.bottom() <= 200.0);
        assert!(menu.max_scroll(window, &m) > 0.0);
        menu.jump(false);
        menu.scroll_to_hover(window, &m);
        let mut last = Rect::default();
        menu.for_each_item(window, &m, |i, rect| {
            if i == 19 {
                last = rect;
            }
        });
        assert!(last.bottom() <= r.bottom() + 0.5);
    }

    #[test]
    fn step_skips_disabled_and_separators() {
        let items = vec![
            entry(0, false),
            MenuItem::Separator,
            entry(2, true),
            entry(3, false),
            entry(4, true),
        ];
        let mut menu = MenuState::new(MenuOwner::Overflow, Point::new(0.0, 0.0), items);
        menu.step(1);
        assert_eq!(menu.hover, Some(2));
        menu.step(1);
        assert_eq!(menu.hover, Some(4));
        menu.step(1);
        assert_eq!(menu.hover, Some(2));
        menu.step(-1);
        assert_eq!(menu.hover, Some(4));
        assert_eq!(menu.pick(0), None);
        assert_eq!(menu.pick(4), Some(MenuPick::Choice(4)));
    }
}
