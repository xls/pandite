//! Text recognition bookkeeping: the providers, the one running job and
//! the per-image results of this session.
//!
//! Recognition reads the image on screen only, on its own thread, once its
//! full-resolution image has been presented for [`TEXT_DELAY`]: flipping
//! through images starts no recognition, and nothing competes with the
//! first pixel or the full-resolution frame. Navigation cancels the job.
//! Results (also "no text" and failures) stay cached per image for the
//! session, at most [`CACHE_ENTRIES`] of them.

use super::jobs::{self, Outbox};
use pandite_core::{CancelToken, ImageBuf, TextLayout, TextProvider};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How long the full image must have been on screen before its text is
/// read.
pub(crate) const TEXT_DELAY: Duration = Duration::from_millis(250);

/// Images whose recognized text the session keeps.
pub(crate) const CACHE_ENTRIES: usize = 32;

/// Names the pixels a layout was read from: the file and the size of the
/// full-resolution image (an edited image saved in place has new pixels and
/// is forgotten by path).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TextKey {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

impl TextKey {
    pub(crate) fn new(path: &Path, image: &ImageBuf) -> Self {
        Self {
            path: path.to_path_buf(),
            width: image.width(),
            height: image.height(),
        }
    }
}

/// The running recognition.
pub(crate) struct TextJob {
    /// Names this job; its answer carries it.
    pub ticket: u64,
    /// The shown image it reads.
    pub image: u64,
    cancel: CancelToken,
}

/// Everything the core knows about text recognition.
pub(crate) struct TextState {
    providers: Vec<Arc<dyn TextProvider>>,
    /// The user's setting allows recognition.
    enabled: bool,
    /// A job found no working recognizer: no more jobs this session.
    unavailable: bool,
    pub job: Option<TextJob>,
    cache: VecDeque<(TextKey, Arc<TextLayout>)>,
    next_id: u64,
    /// Answers of jobs with a smaller ticket read pixels that changed
    /// since (a save replaced them): they are dropped.
    valid_from: u64,
}

impl TextState {
    pub(crate) fn new(providers: Vec<Arc<dyn TextProvider>>, enabled: bool) -> Self {
        Self {
            providers,
            enabled,
            unavailable: false,
            job: None,
            cache: VecDeque::new(),
            next_id: 1,
            valid_from: 0,
        }
    }

    /// True when the answer of job `ticket` still describes the pixels
    /// it read.
    pub(crate) fn current(&self, ticket: u64) -> bool {
        ticket >= self.valid_from
    }

    /// True while recognition may run: a provider is registered, the
    /// setting allows it and none was found missing.
    pub(crate) fn active(&self) -> bool {
        self.enabled && !self.unavailable && !self.providers.is_empty()
    }

    /// Stop for the session: no recognizer works on this system.
    pub(crate) fn set_unavailable(&mut self) {
        self.unavailable = true;
        self.cancel_job();
    }

    /// A new id for a layout attached to an image.
    pub(crate) fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The cached result for `key`, now the most recently used one.
    pub(crate) fn cached(&mut self, key: &TextKey) -> Option<Arc<TextLayout>> {
        let pos = self.cache.iter().position(|(k, _)| k == key)?;
        let entry = self.cache.remove(pos)?;
        let layout = Arc::clone(&entry.1);
        self.cache.push_back(entry);
        Some(layout)
    }

    /// Keep `layout` for `key`, dropping the least recently used entry
    /// beyond the bound.
    pub(crate) fn store(&mut self, key: TextKey, layout: Arc<TextLayout>) {
        self.cache.retain(|(k, _)| *k != key);
        self.cache.push_back((key, layout));
        while self.cache.len() > CACHE_ENTRIES {
            self.cache.pop_front();
        }
    }

    /// Forget every result read from `path` (its pixels changed), also the
    /// answers of jobs still running.
    pub(crate) fn forget(&mut self, path: &Path) {
        self.cache.retain(|(k, _)| k.path != path);
        self.valid_from = self.next_id;
    }

    /// Number of cached results.
    #[cfg(test)]
    pub(crate) fn cached_len(&self) -> usize {
        self.cache.len()
    }

    /// Stop the running job; its result, when it still comes, is ignored
    /// for display.
    pub(crate) fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.cancel();
        }
    }

    /// Start reading `image` (shown image `image_id`) on a new thread.
    /// Returns false when no thread could start.
    pub(crate) fn start(
        &mut self,
        out: Outbox,
        image_id: u64,
        key: TextKey,
        image: ImageBuf,
    ) -> bool {
        self.cancel_job();
        let ticket = self.next_id();
        let cancel = CancelToken::new();
        let job = jobs::TextJobSpec {
            ticket,
            image_id,
            key,
            image,
            providers: self.providers.clone(),
            cancel: cancel.clone(),
        };
        if jobs::recognize(out, job).is_err() {
            return false;
        }
        self.job = Some(TextJob {
            ticket,
            image: image_id,
            cancel,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> TextKey {
        TextKey {
            path: PathBuf::from(name),
            width: 10,
            height: 10,
        }
    }

    #[test]
    fn cache_is_bounded_and_least_recently_used_goes_first() {
        let mut state = TextState::new(Vec::new(), true);
        assert!(!state.active(), "no provider, nothing runs");
        for i in 0..CACHE_ENTRIES + 5 {
            state.store(key(&format!("{i}.png")), Arc::new(TextLayout::new(10, 10)));
        }
        assert_eq!(state.cached_len(), CACHE_ENTRIES);
        assert!(state.cached(&key("0.png")).is_none(), "the oldest left");
        // Using an entry keeps it.
        assert!(state.cached(&key("5.png")).is_some());
        state.store(key("new.png"), Arc::new(TextLayout::new(10, 10)));
        assert!(state.cached(&key("5.png")).is_some());
        assert!(state.cached(&key("6.png")).is_none());
        // Forgetting a path also voids the answers of earlier jobs.
        let early = state.next_id();
        assert!(state.current(early));
        state.forget(Path::new("5.png"));
        assert!(state.cached(&key("5.png")).is_none());
        assert!(!state.current(early));
        let later = state.next_id();
        assert!(state.current(later));
    }
}
