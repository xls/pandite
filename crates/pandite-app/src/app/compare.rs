//! The comparison of selected images: which files it shows, their decoded
//! pictures, and what the shell must hold on the GPU for it.
//!
//! One selected file is set against the image on screen with a slider;
//! two to eight selected files show in tiles. The comparison follows the
//! selection: a file that is unselected leaves its tile, and a selection
//! the comparison cannot show ends it.

use super::{file_name, jobs, Handled, State};
use pandite_core::{Content, Point, Rect, Size};
use pandite_ui::{CompareItem, CompareKind, ComparePane, CompareState, PeekLens, Ui};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The most images a comparison shows.
pub(super) const COMPARE_MAX: usize = 8;

/// What the shell must do with the pictures of the comparison.
#[derive(Clone, Debug)]
pub enum CompareUpdate {
    /// Hold `content` under `slot`, replacing what the slot held.
    Set {
        /// Names the picture in [`PaneOut::slot`].
        slot: u64,
        /// The decoded picture.
        content: Content,
        /// Draw the picture as if it had this size, so it covers the
        /// image on screen exactly.
        logical: Option<(u32, u32)>,
    },
    /// Drop the picture of this slot.
    Remove(u64),
    /// Drop every picture.
    Clear,
}

/// Where the shell draws one picture of a frame that shows several.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneOut {
    /// The slot of a [`CompareUpdate::Set`], or `None` for the image on
    /// screen.
    pub slot: Option<u64>,
    /// The area the view places the picture in.
    pub viewport: Rect,
    /// The part of the window the picture may draw to.
    pub clip: Rect,
    /// Where the whole picture lands, in place of what the view gives:
    /// set for the enlarged copies inside the magnifier.
    pub rect: Option<Rect>,
}

/// The slot of the plain background behind the pictures in the magnifier.
/// The shell fills it with one pixel of the window background.
pub const BACKDROP_SLOT: u64 = u64::MAX;

/// One compared file.
pub(super) struct Slot {
    id: u64,
    path: PathBuf,
    /// The decoded picture, once it arrived.
    content: Option<Content>,
    /// Its pixel size.
    size: Option<(u32, u32)>,
    /// The size the shell was told to draw it at.
    logical: Option<(u32, u32)>,
    /// `content` is a reduced first stage; `size` is the full image's.
    standin: bool,
}

/// The comparison on screen.
pub(super) struct Compare {
    pub kind: CompareKind,
    slots: Vec<Slot>,
    /// One per slot, for the interface.
    pub items: Vec<CompareItem>,
    /// Where the panes of the last frame were.
    panes: Vec<PaneOut>,
}

impl State {
    /// D: compare the selected files.
    pub(super) fn start_compare(&mut self, ui: &mut Ui, now: Duration) -> Handled {
        if self.compare.is_some() {
            return Handled::Ignored;
        }
        let files = self.selected_files();
        let kind = match files.len() {
            1 => {
                // The image the user looks at, or waits for.
                let other = self.target.as_ref().map(|t| t.path.as_path());
                if other.is_none() || other == Some(files[0].as_path()) {
                    return Handled::Ignored;
                }
                CompareKind::Slider
            }
            2..=COMPARE_MAX => CompareKind::Tiles,
            _ => return Handled::Ignored,
        };
        self.compare = Some(Compare {
            kind,
            slots: Vec::new(),
            items: Vec::new(),
            panes: Vec::new(),
        });
        for path in files {
            self.add_compare_slot(path);
        }
        if kind == CompareKind::Tiles {
            self.view = pandite_core::ViewState::default();
        }
        self.refresh_compare();
        if self.compare.is_none() {
            ui.toast("Could not start the comparison", now);
        }
        self.dirty = true;
        Handled::Done
    }

    /// True when the tiles are full and the file at `index` is not one of
    /// them: it cannot be selected until a tile leaves.
    pub(super) fn tiles_full(&self, index: usize) -> bool {
        let full = self
            .compare
            .as_ref()
            .is_some_and(|c| c.kind == CompareKind::Tiles && c.slots.len() >= COMPARE_MAX);
        full && self
            .library
            .path(index)
            .is_some_and(|path| !self.selection.contains(&path))
    }

    /// End the comparison and free its pictures.
    pub(super) fn end_compare(&mut self) -> Handled {
        if self.compare.take().is_none() {
            return Handled::Ignored;
        }
        self.compare_out.push(CompareUpdate::Clear);
        if self.shown.is_some() {
            // The tiles had their own zoom: the image shows fitted again.
            self.view = pandite_core::ViewState::default();
        }
        self.dirty = true;
        Handled::Done
    }

    /// Keep the picture of the one selected file while it is on screen,
    /// so a comparison with it starts without a read. Call after every
    /// change.
    pub(super) fn keep_selected_picture(&mut self) {
        let [path] = self.selection.as_slice() else {
            self.compare_kept = None;
            return;
        };
        if self.compare_kept.as_ref().is_some_and(|(p, _)| p == path) {
            return;
        }
        self.compare_kept = self
            .shown
            .as_ref()
            .filter(|s| s.path == *path && s.standin.is_none())
            .map(|s| (path.clone(), s.base.clone()));
    }

    /// Add `path` to the comparison. A picture that is kept or that the
    /// library holds shows at once: the full image, or its screen image
    /// while the file is read.
    fn add_compare_slot(&mut self, path: PathBuf) {
        let id = self.next_id();
        let index = self.index_of(&path);
        let kept = self
            .compare_kept
            .as_ref()
            .filter(|(p, _)| *p == path)
            .map(|(_, image)| Arc::new(Content::Still(image.clone())));
        let cached = kept.or_else(|| index.and_then(|i| self.library.get(i).ready()));
        let reduced = match &cached {
            Some(_) => None,
            None => index.and_then(|i| self.library.reduced(i)),
        };
        let started = cached.is_some() || {
            // The quick stage is for files the library has no picture of.
            let quick = reduced
                .is_none()
                .then(|| super::screen_size(self.window_size));
            jobs::compare_load(
                self.out.clone(),
                Arc::clone(&self.disk),
                Arc::clone(&self.registry),
                id,
                path.clone(),
                quick,
            )
            .is_ok()
        };
        let Some(compare) = self.compare.as_mut() else {
            return;
        };
        compare.items.push(CompareItem {
            name: file_name(&path),
            index,
            state: if started {
                CompareState::Loading
            } else {
                CompareState::Failed
            },
        });
        compare.slots.push(Slot {
            id,
            path,
            content: None,
            size: None,
            logical: None,
            standin: false,
        });
        if let Some(content) = cached {
            let still = Content::Still(content.first_image().clone());
            self.compare_loaded(id, Ok(still), None);
        } else if let Some(reduced) = reduced {
            let full = (reduced.width, reduced.height);
            self.compare_loaded(id, Ok(Content::Still(reduced.image)), Some(full));
        }
    }

    /// A picture of a compared file arrived, or the file could not be
    /// read. `full_size` marks a reduced first stage and carries the size
    /// of the full image.
    pub(super) fn compare_loaded(
        &mut self,
        slot: u64,
        result: pandite_core::Result<Content>,
        full_size: Option<(u32, u32)>,
    ) {
        let Some(compare) = self.compare.as_mut() else {
            return;
        };
        let Some(at) = compare.slots.iter().position(|s| s.id == slot) else {
            return;
        };
        let held = &mut compare.slots[at];
        match result {
            Ok(content) => {
                if full_size.is_some() && held.content.is_some() && !held.standin {
                    // The full image is here already.
                    return;
                }
                let image = content.first_image();
                held.size = Some(full_size.unwrap_or((image.width(), image.height())));
                held.standin = full_size.is_some();
                held.content = Some(content);
                compare.items[at].state = CompareState::Ready;
                self.push_compare_slot(at);
            }
            // A stand-in stays when the full image fails.
            Err(_) if held.content.is_some() => {}
            Err(_) => compare.items[at].state = CompareState::Failed,
        }
        self.dirty = true;
    }

    /// The size the slider draws its picture at: the size of the image on
    /// screen when both have the same shape, so a smaller copy of a
    /// picture covers the original exactly.
    fn slider_logical(&self, size: (u32, u32)) -> Option<(u32, u32)> {
        let shown = self.shown.as_ref()?.display_dims();
        if shown == size || shown.0 == 0 || shown.1 == 0 || size.0 == 0 || size.1 == 0 {
            return None;
        }
        let (a, b) = (
            shown.0 as f64 / shown.1 as f64,
            size.0 as f64 / size.1 as f64,
        );
        ((a - b).abs() <= 0.01 * a.max(b)).then_some(shown)
    }

    /// Hand the picture of slot `at` to the shell.
    fn push_compare_slot(&mut self, at: usize) {
        let Some(compare) = self.compare.as_ref() else {
            return;
        };
        let slot = &compare.slots[at];
        let (Some(content), Some(size)) = (slot.content.clone(), slot.size) else {
            return;
        };
        let logical = match compare.kind {
            CompareKind::Slider => self.slider_logical(size),
            CompareKind::Tiles => None,
        };
        // A reduced first stage draws at the size of the full image.
        let logical = logical.or(slot.standin.then_some(size));
        let id = slot.id;
        if let Some(compare) = self.compare.as_mut() {
            compare.slots[at].logical = logical;
        }
        self.compare_out.push(CompareUpdate::Set {
            slot: id,
            content,
            logical,
        });
        self.dirty = true;
    }

    /// Make the comparison follow the selection, the folder list and the
    /// image on screen. Call after every change.
    pub(super) fn refresh_compare(&mut self) {
        let Some(kind) = self.compare.as_ref().map(|c| c.kind) else {
            return;
        };
        let files = self.selected_files();
        match kind {
            CompareKind::Slider => {
                let same = self
                    .compare
                    .as_ref()
                    .is_some_and(|c| files.len() == 1 && c.slots[0].path == files[0]);
                if !same || self.target.is_none() {
                    self.end_compare();
                    return;
                }
                // The image on screen changed: the picture may need a new
                // size.
                let stale = self.compare.as_ref().is_some_and(|c| {
                    let slot = &c.slots[0];
                    slot.size.is_some_and(|size| {
                        let want = self.slider_logical(size);
                        want.or(slot.standin.then_some(size)) != slot.logical
                    })
                });
                if stale {
                    self.push_compare_slot(0);
                }
            }
            CompareKind::Tiles => {
                if !(2..=COMPARE_MAX).contains(&files.len()) {
                    self.end_compare();
                    return;
                }
                let gone: Vec<u64> = self
                    .compare
                    .iter()
                    .flat_map(|c| &c.slots)
                    .filter(|s| !files.contains(&s.path))
                    .map(|s| s.id)
                    .collect();
                if let Some(compare) = self.compare.as_mut() {
                    for id in &gone {
                        if let Some(at) = compare.slots.iter().position(|s| s.id == *id) {
                            compare.slots.remove(at);
                            compare.items.remove(at);
                        }
                    }
                }
                for id in gone {
                    self.compare_out.push(CompareUpdate::Remove(id));
                    self.dirty = true;
                }
                for path in &files {
                    let held = self
                        .compare
                        .as_ref()
                        .is_some_and(|c| c.slots.iter().any(|s| s.path == *path));
                    if !held {
                        self.add_compare_slot(path.clone());
                        self.dirty = true;
                    }
                }
                // Folder order, like the gallery strip.
                if let Some(compare) = self.compare.as_mut() {
                    let order = |path: &PathBuf| files.iter().position(|f| f == path);
                    let mut both: Vec<(Slot, CompareItem)> = compare
                        .slots
                        .drain(..)
                        .zip(compare.items.drain(..))
                        .collect();
                    both.sort_by_key(|(slot, _)| order(&slot.path));
                    for (slot, item) in both {
                        compare.slots.push(slot);
                        compare.items.push(item);
                    }
                }
            }
        }
        // The folder indices, for Space on a tile.
        let indices: Vec<Option<usize>> = self
            .compare
            .iter()
            .flat_map(|c| &c.slots)
            .map(|s| self.index_of(&s.path))
            .collect();
        if let Some(compare) = self.compare.as_mut() {
            for (item, index) in compare.items.iter_mut().zip(indices) {
                item.index = index;
            }
        }
    }

    /// The GPU device was replaced: hand every picture over again.
    pub(super) fn repush_compare(&mut self) {
        let n = self.compare.as_ref().map_or(0, |c| c.slots.len());
        for at in 0..n {
            self.push_compare_slot(at);
        }
    }

    /// Keep where the interface put the panes, and give them slot names.
    pub(super) fn place_compare(&mut self, panes: &[ComparePane]) -> Vec<PaneOut> {
        let Some(compare) = self.compare.as_mut() else {
            return Vec::new();
        };
        compare.panes = panes
            .iter()
            .filter_map(|p| {
                let slot = match p.item {
                    Some(i) => Some(compare.slots.get(i)?.id),
                    None => None,
                };
                Some(PaneOut {
                    slot,
                    viewport: p.viewport,
                    clip: p.clip,
                    rect: None,
                })
            })
            .collect();
        compare.panes.clone()
    }

    /// The size the picture of a pane is drawn with.
    fn pane_size(&self, slot: Option<u64>) -> Option<Size> {
        let (w, h) = match slot {
            None => return self.display_size(),
            Some(id) => {
                let slot = self.compare.as_ref()?.slots.iter().find(|s| s.id == id)?;
                slot.logical.or(slot.size)?
            }
        };
        Some(Size::new(w as f32, h as f32))
    }

    /// Add the magnifier to the panes of a frame: for every square a
    /// plain background, then its picture again, enlarged so that the
    /// source point lands on the center of the square. A picture shown
    /// below 50 % is enlarged to 100 %, any other to twice its size.
    /// Without a comparison `panes` is empty and the image on screen
    /// fills `area`.
    pub(super) fn peek_panes(
        &self,
        panes: Vec<PaneOut>,
        area: Rect,
        lenses: &[PeekLens],
    ) -> Vec<PaneOut> {
        if lenses.is_empty() {
            return panes;
        }
        let mut out = panes;
        if out.is_empty() {
            out.push(PaneOut {
                slot: None,
                viewport: area,
                clip: area,
                rect: None,
            });
        }
        let base = out.clone();
        let mut enlarged = Vec::new();
        for lens in lenses {
            let Some(clip) = lens.rect.intersect(lens.bounds) else {
                continue;
            };
            out.push(PaneOut {
                slot: Some(BACKDROP_SLOT),
                viewport: clip,
                clip,
                rect: Some(lens.rect),
            });
            let slot = match lens.item {
                None => None,
                Some(i) => match self.compare.as_ref().and_then(|c| c.slots.get(i)) {
                    Some(slot) => Some(slot.id),
                    None => continue,
                },
            };
            let Some(pane) = base.iter().find(|p| p.slot == slot) else {
                continue;
            };
            let Some(size) = self.pane_size(slot) else {
                continue;
            };
            let mut rect = self.view.image_rect(size, pane.viewport);
            rect.x = rect.x.round();
            rect.y = rect.y.round();
            if rect.w <= 0.0 || size.w <= 0.0 {
                continue;
            }
            let scale = rect.w / size.w;
            let by = (2.0 * scale).max(1.0) / scale;
            let center = lens.rect.center();
            enlarged.push(PaneOut {
                slot,
                viewport: clip,
                clip,
                rect: Some(Rect::new(
                    center.x + (rect.x - lens.source.x) * by,
                    center.y + (rect.y - lens.source.y) * by,
                    rect.w * by,
                    rect.h * by,
                )),
            });
        }
        // The backgrounds first, so the pictures of one frame share a
        // drawing layer.
        out.extend(enlarged);
        out
    }

    /// The picture size and the area that zoom and pan work with: in
    /// tiles, the tile under `anchor` (else the first with a picture);
    /// otherwise the image on screen in the image area.
    pub(super) fn view_frame(&self, anchor: Option<Point>) -> Option<(Size, Rect)> {
        let tiles = self
            .compare
            .as_ref()
            .filter(|c| c.kind == CompareKind::Tiles);
        let Some(compare) = tiles else {
            return Some((self.display_size()?, self.content_rect));
        };
        let sized = |pane: &PaneOut| {
            let slot = compare.slots.iter().find(|s| Some(s.id) == pane.slot)?;
            let (w, h) = slot.size?;
            Some((Size::new(w as f32, h as f32), pane.viewport))
        };
        anchor
            .and_then(|p| compare.panes.iter().find(|pane| pane.clip.contains(p)))
            .and_then(sized)
            .or_else(|| compare.panes.iter().find_map(sized))
    }
}
