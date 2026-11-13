//! Text recognition bookkeeping: the providers, the one running job and
//! the per-image results of this session.
//!
//! Recognition reads the image on screen only, on its own thread, once its
//! full-resolution picture has been presented for [`TEXT_DELAY`]: flipping
//! through images starts no recognition, and nothing competes with the
//! first pixel or the full-resolution frame. Navigation cancels the job.
//!
//! Recognizers read sideways and upside-down text poorly, so the text is
//! read the way the picture on screen is turned ([`Turn`]): a quarter turn
//! or a mirror that changes the turn reads the image again, turned the new
//! way, and the new words replace the old ones. A crop or a resize keeps
//! the turn and reads nothing. The job turns the image on its own thread
//! and turns the word outlines back, so every layout is in pixels of the
//! decoded image and follows the edits on screen like the first one.
//! Results (also "no text" and failures) stay cached per image and turn
//! for the session, at most [`CACHE_ENTRIES`] of them: turning back shows
//! the earlier words at once.

use super::jobs::{self, Outbox};
use crate::edit::Edit;
use pandite_core::{Affine, CancelToken, ImageBuf, TextLayout, TextProvider};
use pandite_ops::Rotation;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How long the full picture must have been on screen before its text is
/// read.
pub(crate) const TEXT_DELAY: Duration = Duration::from_millis(250);

/// Readings (an image in one turn) the session keeps.
pub(crate) const CACHE_ENTRIES: usize = 32;

/// How the picture on screen is turned against the decoded image: one of
/// the eight results of quarter turns and mirrors. A mirror left to right
/// (when `mirrored`) comes first, then `quarters` quarter turns clockwise.
/// Crops and resizes do not change it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Turn {
    mirrored: bool,
    /// 0 to 3.
    quarters: u8,
}

impl Turn {
    /// The decoded image as it is.
    pub(crate) const UPRIGHT: Turn = Turn {
        mirrored: false,
        quarters: 0,
    };

    /// The turn of an image after `edits`, applied in order.
    pub(crate) fn of<'a>(edits: impl IntoIterator<Item = &'a Edit>) -> Turn {
        edits.into_iter().fold(Turn::UPRIGHT, |t, e| t.then(e))
    }

    /// This turn followed by `edit`.
    pub(crate) fn then(self, edit: &Edit) -> Turn {
        let q = self.quarters;
        let (mirrored, quarters) = match edit {
            Edit::RotateRight => (self.mirrored, q + 1),
            Edit::RotateLeft => (self.mirrored, q + 3),
            // A mirror after `q` turns equals `-q` turns after a mirror.
            Edit::FlipHorizontal => (!self.mirrored, 4 - q),
            // A flip top to bottom is a mirror followed by a half turn.
            Edit::FlipVertical => (!self.mirrored, 6 - q),
            Edit::Crop(_) | Edit::Resize(_) => (self.mirrored, q),
        };
        Turn {
            mirrored,
            quarters: quarters % 4,
        }
    }

    /// `image` turned this way. Copies every pixel unless the turn is
    /// [`Turn::UPRIGHT`]: call it on a worker thread.
    pub(crate) fn apply(self, image: &ImageBuf) -> ImageBuf {
        let rotation = match self.quarters {
            1 => Some(Rotation::Cw90),
            2 => Some(Rotation::Cw180),
            3 => Some(Rotation::Cw270),
            _ => None,
        };
        match (self.mirrored, rotation) {
            (false, None) => image.clone(),
            (false, Some(r)) => pandite_ops::rotate(image, r),
            (true, None) => pandite_ops::flip_horizontal(image),
            // A mirror and a half turn: one flip top to bottom.
            (true, Some(Rotation::Cw180)) => pandite_ops::flip_vertical(image),
            (true, Some(r)) => pandite_ops::rotate(&pandite_ops::flip_horizontal(image), r),
        }
    }

    /// Where this turn moves a point of a `size.0` x `size.1` image: from
    /// its pixel coordinates to those of the turned image.
    pub(crate) fn transform(self, size: (u32, u32)) -> Affine {
        let (mut w, mut h) = (size.0 as f32, size.1 as f32);
        let mut t = if self.mirrored {
            // (x, y) -> (w - x, y)
            Affine::new(-1.0, 0.0, 0.0, 1.0, w, 0.0)
        } else {
            Affine::IDENTITY
        };
        for _ in 0..self.quarters {
            // (x, y) -> (h - y, x)
            t = t.then(&Affine::new(0.0, 1.0, -1.0, 0.0, h, 0.0));
            (w, h) = (h, w);
        }
        t
    }

    /// `layout`, read from a `size.0` x `size.1` image turned this way,
    /// in pixels of that image as it is.
    pub(crate) fn unturn(self, layout: TextLayout, size: (u32, u32)) -> TextLayout {
        if self == Turn::UPRIGHT {
            return layout;
        }
        // Quarter turns and mirrors always invert.
        let back = self.transform(size).inverse().unwrap_or(Affine::IDENTITY);
        layout.mapped(size.0, size.1, |p| back.apply(p))
    }
}

/// Names the reading a layout came from: the file, the size of the
/// full-resolution image (an edited image saved in place has new pixels
/// and is forgotten by path) and the turn the text was read in. The
/// layout is in pixels of that image, whatever the turn.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TextKey {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub turn: Turn,
}

impl TextKey {
    pub(crate) fn new(path: &Path, image: &ImageBuf, turn: Turn) -> Self {
        Self {
            path: path.to_path_buf(),
            width: image.width(),
            height: image.height(),
            turn,
        }
    }
}

/// The running recognition.
pub(crate) struct TextJob {
    /// Names this job; its answer carries it.
    pub ticket: u64,
    /// The shown image it reads.
    pub image: u64,
    /// The turn it reads the image in.
    pub turn: Turn,
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

    /// Start reading `image` (shown image `image_id`) turned as
    /// `key.turn` says, on a new thread. Returns false when no thread
    /// could start.
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
        let turn = key.turn;
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
            turn,
            cancel,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::edit::EditStack;
    use pandite_core::{PixelFormat, Point, Quad, Rect, TextLine, TextWord};
    use pandite_ops::PixelRect;

    fn key(name: &str) -> TextKey {
        TextKey {
            path: PathBuf::from(name),
            width: 10,
            height: 10,
            turn: Turn::UPRIGHT,
        }
    }

    /// A 3 x 2 image where every pixel has its own red value.
    fn sample() -> ImageBuf {
        let mut data = Vec::new();
        for i in 0..6u8 {
            data.extend_from_slice(&[10 * (i + 1), 0, 0, 255]);
        }
        ImageBuf::new(3, 2, PixelFormat::Rgba8, data).unwrap()
    }

    /// Every sequence of up to four quarter turns and mirrors.
    fn sequences() -> Vec<Vec<Edit>> {
        let steps = [
            Edit::RotateRight,
            Edit::RotateLeft,
            Edit::FlipHorizontal,
            Edit::FlipVertical,
        ];
        let mut all = vec![Vec::new()];
        let mut last = vec![Vec::new()];
        for _ in 0..4 {
            let mut next = Vec::new();
            for seq in &last {
                for step in steps {
                    let mut longer: Vec<Edit> = seq.clone();
                    longer.push(step);
                    next.push(longer);
                }
            }
            all.extend(next.iter().cloned());
            last = next;
        }
        all
    }

    #[test]
    fn turn_matches_the_edits_it_sums_up() {
        let base = sample();
        let mut seen = std::collections::HashSet::new();
        for seq in sequences() {
            let mut stack = EditStack::new();
            for edit in &seq {
                stack.push(*edit);
            }
            stack.run_all(&base).unwrap();
            let turn = Turn::of(&seq);
            assert_eq!(turn, Turn::of(stack.edits()), "{seq:?}");
            seen.insert(turn);
            // The turned image is the edited picture, pixel for pixel, and
            // the turn moves points as the edits do.
            let turned = turn.apply(&base);
            let edited = stack.current(&base);
            assert_eq!(
                (turned.width(), turned.height()),
                (edited.width(), edited.height()),
                "{seq:?}"
            );
            assert_eq!(turned.data(), edited.data(), "{seq:?}");
            assert_eq!(turn.transform((3, 2)), stack.transform((3, 2)), "{seq:?}");
        }
        assert_eq!(seen.len(), 8, "every turn is reached");
        // Crops and resizes keep the turn.
        let crop = Edit::Crop(PixelRect::new(0, 0, 1, 1));
        assert_eq!(
            Turn::of(&[Edit::RotateRight, crop]),
            Turn::of(&[Edit::RotateRight])
        );
        assert_eq!(Turn::of(&[crop]), Turn::UPRIGHT);
        // Turning back or mirroring twice is upright again.
        assert_eq!(
            Turn::of(&[Edit::RotateRight, Edit::RotateLeft]),
            Turn::UPRIGHT
        );
        assert_eq!(
            Turn::of(&[Edit::FlipVertical, Edit::FlipVertical]),
            Turn::UPRIGHT
        );
        assert_eq!(
            Turn::of(&[Edit::FlipHorizontal, Edit::FlipVertical]),
            Turn::of(&[Edit::RotateRight, Edit::RotateRight])
        );
    }

    #[test]
    fn words_read_turned_land_on_the_decoded_image() {
        let size = (30, 20);
        let word = |x: f32| TextWord {
            text: "w".into(),
            quad: Quad::from_rect(Rect::new(x, 2.0, 4.0, 3.0)),
        };
        let image = ImageBuf::solid_rgba8(size.0, size.1, [0, 0, 0, 255]).unwrap();
        for seq in sequences() {
            let turn = Turn::of(&seq);
            let to_turned = turn.transform(size);
            let turned = turn.apply(&image);
            let turned_size = (turned.width(), turned.height());
            // The image's corners land on the turned image's corners.
            for corner in [(0.0, 0.0), (30.0, 0.0), (30.0, 20.0), (0.0, 20.0)] {
                let p = to_turned.apply(Point::new(corner.0, corner.1));
                let on_x = p.x.abs() < 1e-4 || (p.x - turned_size.0 as f32).abs() < 1e-4;
                let on_y = p.y.abs() < 1e-4 || (p.y - turned_size.1 as f32).abs() < 1e-4;
                assert!(on_x && on_y, "{seq:?}: {corner:?} went to {p:?}");
            }
            let read = TextLayout {
                width: turned_size.0,
                height: turned_size.1,
                lines: vec![TextLine {
                    words: vec![word(1.0), word(8.0)],
                }],
            };
            let back = turn.unturn(read.clone(), size);
            assert_eq!((back.width, back.height), size, "{seq:?}");
            assert_eq!(back.text(), read.text());
            // Turned forward again, every corner is where it was read.
            for (b, r) in back.words().zip(read.words()) {
                for (p, q) in b.1.quad.points.iter().zip(r.1.quad.points) {
                    let p = to_turned.apply(*p);
                    assert!(
                        (p.x - q.x).abs() < 1e-4 && (p.y - q.y).abs() < 1e-4,
                        "{seq:?}: {p:?} vs {q:?}"
                    );
                }
            }
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
        // Each turn of an image is a reading of its own.
        let right = TextKey {
            turn: Turn::of(&[Edit::RotateRight]),
            ..key("turned.png")
        };
        state.store(right.clone(), Arc::new(TextLayout::new(10, 10)));
        assert!(state.cached(&key("turned.png")).is_none());
        assert!(state.cached(&right).is_some());
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
