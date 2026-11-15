//! Gallery thumbnails: which are uploaded to the renderer, which the
//! library should make next, and what the strip shows for each index.

use pandite_core::TextureSink;
use pandite_library::{Fetch, Library};
use pandite_ui::{thumbnail_texture_id, ThumbState};
use std::collections::BTreeSet;
use std::ops::Range;

/// Thumbnail bookkeeping for the gallery strip.
#[derive(Debug, Default)]
pub(crate) struct Thumbs {
    states: Vec<ThumbState>,
    uploaded: BTreeSet<usize>,
    /// Uploaded indexes whose thumbnail was made again (at a new size):
    /// the next sync uploads the new one over the old one.
    refresh: BTreeSet<usize>,
    remove: Vec<usize>,
    visible: Range<usize>,
    reported: bool,
    dirty: bool,
}

impl Thumbs {
    /// Per-index states for [`pandite_ui::UiContext::thumbnails`].
    pub fn states(&self) -> &[ThumbState] {
        &self.states
    }

    /// The folder list was replaced: every index is stale.
    pub fn reset(&mut self, len: usize) {
        self.remove.extend(std::mem::take(&mut self.uploaded));
        self.refresh.clear();
        self.states = vec![ThumbState::Pending; len];
        self.reported = false;
        self.visible = 0..0;
        self.dirty = true;
    }

    /// Forget every upload (the GPU device was replaced).
    pub fn invalidate(&mut self) {
        self.uploaded.clear();
        self.refresh.clear();
        self.remove.clear();
        for s in &mut self.states {
            if matches!(s, ThumbState::Ready { .. }) {
                *s = ThumbState::Pending;
            }
        }
        self.dirty = true;
    }

    /// A thumbnail finished; upload it if it is near the visible range.
    /// One that replaces an uploaded thumbnail (made again at a new size)
    /// is uploaded over it.
    pub fn ready(&mut self, index: usize) {
        if let Some(state) = self.states.get(index) {
            if matches!(state, ThumbState::Ready { .. }) {
                self.refresh.insert(index);
            }
            self.dirty = true;
        }
    }

    /// Set the state of one index unless its thumbnail is already shown.
    pub fn mark(&mut self, index: usize, state: ThumbState) {
        if let Some(s) = self.states.get_mut(index) {
            if !matches!(s, ThumbState::Ready { .. }) {
                *s = state;
            }
        }
    }

    /// Tell the library which range the strip shows, when it changed.
    pub fn set_visible(&mut self, range: Range<usize>, library: &Library) {
        if self.reported && range == self.visible {
            return;
        }
        library.set_visible_thumbnails(range.clone());
        self.visible = range;
        self.reported = true;
        self.dirty = true;
    }

    /// Indexes whose textures are kept: the visible range plus a margin on
    /// both sides, so scrolling back does not re-upload.
    fn keep(&self) -> Range<usize> {
        let margin = self.visible.len().max(8) * 2;
        let end = (self.visible.end + margin).min(self.states.len());
        self.visible.start.saturating_sub(margin)..end
    }

    /// Upload thumbnails that became ready near the visible range and free
    /// textures far out of view. Returns true when something changed.
    pub fn sync(&mut self, library: &Library, sink: &mut dyn TextureSink) -> bool {
        let mut changed = false;
        for index in self.remove.drain(..) {
            sink.remove(thumbnail_texture_id(index));
            changed = true;
        }
        if !self.dirty {
            return changed;
        }
        self.dirty = false;
        let keep = self.keep();
        let far: Vec<usize> = self
            .uploaded
            .iter()
            .copied()
            .filter(|i| !keep.contains(i))
            .collect();
        for index in far {
            sink.remove(thumbnail_texture_id(index));
            self.uploaded.remove(&index);
            self.refresh.remove(&index);
            if let Some(s) = self.states.get_mut(index) {
                *s = ThumbState::Pending;
            }
            changed = true;
        }
        for index in keep {
            let shown = matches!(self.states[index], ThumbState::Ready { .. });
            if shown && !self.refresh.remove(&index) {
                continue;
            }
            match library.thumbnail(index) {
                Fetch::Ready(image) => {
                    // Under the same id, a new upload replaces the old
                    // texture, also at another size.
                    sink.upload(thumbnail_texture_id(index), &image, false);
                    self.states[index] = ThumbState::Ready {
                        width: image.width(),
                        height: image.height(),
                    };
                    self.uploaded.insert(index);
                    changed = true;
                }
                // The uploaded thumbnail stays.
                _ if shown => {}
                Fetch::NotLocal => self.states[index] = ThumbState::NotDownloaded,
                Fetch::Failed(_) => self.states[index] = ThumbState::Failed,
                Fetch::Pending | Fetch::Downloading | Fetch::Cancelled => {}
            }
        }
        changed
    }

    /// Number of uploaded thumbnail textures.
    pub fn uploaded(&self) -> usize {
        self.uploaded.len()
    }
}
