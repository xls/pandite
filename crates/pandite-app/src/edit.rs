//! View edits: rotate, flip, crop and resize applied to the decoded image.
//!
//! Edits never touch the file. An [`EditStack`] holds the operations and
//! the image after each one, so undo shows the previous result at once.
//! New operations queue while one runs on a worker; the stack only plans
//! the work ([`EditStack::next_job`]) and records results
//! ([`EditStack::finish`]), so it is plain data and easy to test.

use pandite_core::{CancelToken, Error, ImageBuf, Result};
use pandite_ops::{Filter, PixelRect, ResizeOptions, ResizeSpec, Rotation};
use std::collections::VecDeque;

/// One view edit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    /// Rotate 90 degrees counterclockwise.
    RotateLeft,
    /// Rotate 90 degrees clockwise.
    RotateRight,
    /// Mirror left to right.
    FlipHorizontal,
    /// Mirror top to bottom.
    FlipVertical,
    /// Keep this rectangle of the image before the edit.
    Crop(PixelRect),
    /// Resample with Lanczos3 to the size this rule gives.
    Resize(ResizeSpec),
}

impl Edit {
    /// Apply the edit to `image` and return the result. Resizing stops
    /// early with [`Error::Cancelled`] when `cancel` is set.
    pub fn apply(&self, image: &ImageBuf, cancel: &CancelToken) -> Result<ImageBuf> {
        cancel.check()?;
        match *self {
            Edit::RotateLeft => Ok(pandite_ops::rotate(image, Rotation::Cw270)),
            Edit::RotateRight => Ok(pandite_ops::rotate(image, Rotation::Cw90)),
            Edit::FlipHorizontal => Ok(pandite_ops::flip_horizontal(image)),
            Edit::FlipVertical => Ok(pandite_ops::flip_vertical(image)),
            Edit::Crop(rect) => pandite_ops::crop(image, rect),
            Edit::Resize(spec) => {
                let (w, h) = spec.resolve(image.width(), image.height())?;
                let options = ResizeOptions {
                    filter: Filter::Lanczos3,
                    threads: 0,
                    cancel: Some(cancel.clone()),
                };
                pandite_ops::resize_with(image, w, h, &options)
            }
        }
    }

    /// A short name for messages.
    pub fn label(&self) -> &'static str {
        match self {
            Edit::RotateLeft | Edit::RotateRight => "Rotate",
            Edit::FlipHorizontal | Edit::FlipVertical => "Flip",
            Edit::Crop(_) => "Crop",
            Edit::Resize(_) => "Resize",
        }
    }
}

/// Convert the interface's resize rule to the operation crate's.
pub fn resize_spec(spec: pandite_ui::ResizeSpec) -> ResizeSpec {
    match spec {
        pandite_ui::ResizeSpec::Percent(p) => ResizeSpec::Percent(f64::from(p)),
        pandite_ui::ResizeSpec::Fit {
            width,
            height,
            enlarge,
        } => ResizeSpec::Fit {
            width,
            height,
            enlarge,
        },
        pandite_ui::ResizeSpec::Exact { width, height } => ResizeSpec::Exact { width, height },
    }
}

/// Convert the interface's crop rectangle to the operation crate's.
pub fn pixel_rect(rect: pandite_ui::PixelRect) -> PixelRect {
    PixelRect::new(rect.x, rect.y, rect.width, rect.height)
}

/// What [`EditStack::undo`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Nothing to undo.
    Nothing,
    /// A queued edit that had not started was dropped; the picture is
    /// unchanged.
    Dequeued,
    /// The running edit will be thrown away when it finishes; the picture
    /// is unchanged.
    DroppedRunning,
    /// The last applied edit was removed; show [`EditStack::current`].
    Reverted,
}

/// The edits of the image on screen.
#[derive(Debug, Default)]
pub struct EditStack {
    applied: Vec<(Edit, ImageBuf)>,
    queued: VecDeque<Edit>,
    running: Option<Edit>,
    discard_running: bool,
}

impl EditStack {
    /// An empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an edit. It runs after the ones before it.
    pub fn push(&mut self, edit: Edit) {
        self.queued.push_back(edit);
    }

    /// The image to show: the result of the last applied edit, or `base`.
    pub fn current<'a>(&'a self, base: &'a ImageBuf) -> &'a ImageBuf {
        self.applied.last().map_or(base, |(_, image)| image)
    }

    /// True when some edit is applied, running or queued: the picture
    /// differs (or will differ) from the file.
    pub fn is_edited(&self) -> bool {
        !self.applied.is_empty() || self.running.is_some() || !self.queued.is_empty()
    }

    /// True when an edit runs or waits.
    pub fn is_busy(&self) -> bool {
        self.running.is_some() || !self.queued.is_empty()
    }

    /// True when nothing runs or waits.
    pub fn is_idle(&self) -> bool {
        !self.is_busy()
    }

    /// Number of applied edits.
    pub fn len(&self) -> usize {
        self.applied.len()
    }

    /// True when no edit is applied.
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }

    /// The applied edits in order.
    pub fn edits(&self) -> impl Iterator<Item = &Edit> {
        self.applied.iter().map(|(e, _)| e)
    }

    /// When idle and an edit waits, mark it running and return it with the
    /// image it applies to. Run it (normally on a worker) and report with
    /// [`EditStack::finish`].
    pub fn next_job(&mut self, base: &ImageBuf) -> Option<(Edit, ImageBuf)> {
        if self.running.is_some() {
            return None;
        }
        let edit = self.queued.pop_front()?;
        self.running = Some(edit);
        self.discard_running = false;
        Some((edit, self.current(base).clone()))
    }

    /// Record the result of the running edit. Returns `Ok(true)` when the
    /// picture changed, `Ok(false)` when the result was thrown away (undone
    /// while running, or nothing was running) and the error when the edit
    /// failed (it is dropped).
    pub fn finish(&mut self, result: Result<ImageBuf>) -> Result<bool> {
        let Some(edit) = self.running.take() else {
            return Ok(false);
        };
        if std::mem::take(&mut self.discard_running) {
            return Ok(false);
        }
        let image = result?;
        self.applied.push((edit, image));
        Ok(true)
    }

    /// Undo the newest edit: a queued one first, then the running one, then
    /// the last applied one.
    pub fn undo(&mut self) -> Undo {
        if self.queued.pop_back().is_some() {
            return Undo::Dequeued;
        }
        if self.running.is_some() && !self.discard_running {
            self.discard_running = true;
            return Undo::DroppedRunning;
        }
        if self.applied.pop().is_some() {
            return Undo::Reverted;
        }
        Undo::Nothing
    }

    /// Forget every edit; the picture is `base` again. A running job's
    /// result will be ignored.
    pub fn clear(&mut self) {
        self.applied.clear();
        self.queued.clear();
        if self.running.is_some() {
            self.discard_running = true;
        }
    }

    /// Keep the current result as the new base: the edits are saved.
    /// Returns the image to use as base from now on.
    pub fn commit(&mut self, base: &ImageBuf) -> ImageBuf {
        let image = self.current(base).clone();
        self.applied.clear();
        image
    }

    /// Apply every queued edit on this thread (tests and tools).
    pub fn run_all(&mut self, base: &ImageBuf) -> Result<()> {
        let cancel = CancelToken::new();
        while let Some((edit, input)) = self.next_job(base) {
            self.finish(edit.apply(&input, &cancel))?;
        }
        Ok(())
    }
}

/// Turn a failed edit into a short message.
pub fn describe_error(edit: &Edit, err: &Error) -> String {
    match err {
        Error::TooLarge { width, height } => {
            format!(
                "{} would make the image too large ({width} x {height})",
                edit.label()
            )
        }
        Error::Invalid(_) => format!("{} does not fit this image", edit.label()),
        other => format!("{} failed: {other}", edit.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::PixelFormat;

    /// A 3 x 2 image where every pixel has its own red value.
    fn sample() -> ImageBuf {
        let mut data = Vec::new();
        for i in 0..6u8 {
            data.extend_from_slice(&[10 * (i + 1), 0, 0, 255]);
        }
        ImageBuf::new(3, 2, PixelFormat::Rgba8, data).unwrap()
    }

    fn reds(img: &ImageBuf) -> Vec<u8> {
        img.data().chunks_exact(4).map(|p| p[0]).collect()
    }

    #[test]
    fn edit_stack_rotate_flip_crop_resize() {
        let base = sample();
        // Source rows: [10 20 30] / [40 50 60].
        let mut stack = EditStack::new();
        stack.push(Edit::RotateRight);
        stack.run_all(&base).unwrap();
        let r = stack.current(&base);
        assert_eq!((r.width(), r.height()), (2, 3));
        assert_eq!(reds(r), [40, 10, 50, 20, 60, 30]);

        stack.push(Edit::FlipHorizontal);
        stack.run_all(&base).unwrap();
        assert_eq!(reds(stack.current(&base)), [10, 40, 20, 50, 30, 60]);

        stack.push(Edit::FlipVertical);
        stack.run_all(&base).unwrap();
        assert_eq!(reds(stack.current(&base)), [30, 60, 20, 50, 10, 40]);

        stack.push(Edit::RotateLeft);
        stack.run_all(&base).unwrap();
        let r = stack.current(&base);
        assert_eq!((r.width(), r.height()), (3, 2));
        assert_eq!(reds(r), [60, 50, 40, 30, 20, 10]);

        stack.push(Edit::Crop(PixelRect::new(1, 0, 2, 2)));
        stack.run_all(&base).unwrap();
        let r = stack.current(&base);
        assert_eq!((r.width(), r.height()), (2, 2));
        assert_eq!(reds(r), [50, 40, 20, 10]);

        stack.push(Edit::Resize(ResizeSpec::Exact {
            width: 4,
            height: 6,
        }));
        stack.run_all(&base).unwrap();
        assert_eq!(
            (stack.current(&base).width(), stack.current(&base).height()),
            (4, 6)
        );
        assert_eq!(stack.len(), 6);
        assert!(stack.is_edited());

        // Undo walks back one result at a time without recomputing.
        assert_eq!(stack.undo(), Undo::Reverted);
        assert_eq!(reds(stack.current(&base)), [50, 40, 20, 10]);
        assert_eq!(stack.undo(), Undo::Reverted);
        assert_eq!(reds(stack.current(&base)), [60, 50, 40, 30, 20, 10]);
        while stack.undo() == Undo::Reverted {}
        assert_eq!(reds(stack.current(&base)), reds(&base));
        assert!(!stack.is_edited());
        assert_eq!(stack.undo(), Undo::Nothing);

        // The base image never changes.
        assert_eq!(reds(&base), [10, 20, 30, 40, 50, 60]);

        // Percent and fit rules convert from the interface's types.
        let half = resize_spec(pandite_ui::ResizeSpec::Percent(50.0));
        assert_eq!(half.resolve(400, 200).unwrap(), (200, 100));
        let fit = resize_spec(pandite_ui::ResizeSpec::Fit {
            width: 100,
            height: 100,
            enlarge: false,
        });
        assert_eq!(fit.resolve(400, 200).unwrap(), (100, 50));
    }

    #[test]
    fn queue_undo_and_failure() {
        let base = sample();
        let mut stack = EditStack::new();
        stack.push(Edit::RotateRight);
        stack.push(Edit::FlipVertical);
        // Undo drops the newest queued edit first.
        assert_eq!(stack.undo(), Undo::Dequeued);
        let (edit, input) = stack.next_job(&base).unwrap();
        assert_eq!(edit, Edit::RotateRight);
        assert!(stack.next_job(&base).is_none(), "one job at a time");
        // Undo while running throws the result away.
        assert_eq!(stack.undo(), Undo::DroppedRunning);
        let result = edit.apply(&input, &CancelToken::new());
        assert!(!stack.finish(result).unwrap());
        assert!(!stack.is_edited());
        // A crop outside the image fails and leaves the stack unchanged.
        stack.push(Edit::Crop(PixelRect::new(2, 0, 5, 5)));
        let (edit, input) = stack.next_job(&base).unwrap();
        let err = stack
            .finish(edit.apply(&input, &CancelToken::new()))
            .unwrap_err();
        assert!(describe_error(&edit, &err).contains("Crop"));
        assert!(stack.is_empty() && stack.is_idle());
    }
}
