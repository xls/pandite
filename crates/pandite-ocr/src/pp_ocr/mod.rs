//! The built-in recognizer of macOS and Linux: the PP-OCRv6 tiny models
//! of PaddleOCR, embedded in the program and run by tract, a pure Rust
//! inference engine. Nothing is loaded from disk or the network.
//!
//! The pipeline reproduces the reference implementation, RapidOCR 3.10
//! with its default settings and the orientation classifier off:
//!
//! 1. The image is read at most 2000 pixels on the long side and at
//!    least 30 on the short side (sides rounded to multiples of 32 when
//!    scaled). Unlike the reference, shrinking averages the pixels.
//!    Transparent pixels lie over white, as for Windows OCR. The models
//!    read color, so the gray copy and the 6000 pixel cap of
//!    [`crate::prepare`] do not apply here.
//! 2. Very wide or very low images get black bands above and below.
//! 3. The detection model reads the image scaled so that its short side
//!    is at least 736 pixels, sides rounded to multiples of 32 (and, our
//!    addition, the long side at most 4000 for very tall images). [`db`]
//!    turns its probability map into boxes in reading order.
//! 4. Each box is cut out straight, turned when it is much taller than
//!    wide, scaled to 48 pixels high and read by the recognition model,
//!    one line at a time. [`ctc`] turns the output into text and word
//!    boxes from the time steps of the characters. Lines with a mean
//!    character probability below 0.5 are dropped.
//!
//! Both models are loaded and optimized once per process, on first use
//! (about 0.1 second). The recognition model takes any width. The
//! detection model needs a fixed input size, so an optimized plan is kept
//! for each of the last [`PLANS`] sizes; a new size costs about 0.1
//! second. Recognition runs on the calling thread only. The cancel token
//! is checked between the stages, between lines and between the model's
//! operations.

mod ctc;
mod db;
mod raster;
#[cfg(test)]
mod tests;

use pandite_core::{
    CancelToken, Error, ImageBuf, Point, Quad, Result, TextLayout, TextLine, TextProvider, TextWord,
};
use raster::{Bgr, Homography};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, OnceLock};
use tract_onnx::prelude::*;

/// PP-OCRv6 tiny text detection, ONNX. See `models/README.md`.
static DET_MODEL: &[u8] = include_bytes!("../../models/PP-OCRv6_det_tiny.onnx");
/// PP-OCRv6 tiny text recognition, ONNX, with its character list in the
/// metadata under `character`.
static REC_MODEL: &[u8] = include_bytes!("../../models/PP-OCRv6_rec_tiny.onnx");

/// The reference's `Global.max_side_len`.
const MAX_SIDE: f64 = 2000.0;
/// The reference's `Global.min_side_len`.
const MIN_SIDE: f64 = 30.0;
/// Wider than this many times its height, an image gets bands
/// (`Global.width_height_ratio`).
const BAND_RATIO: f64 = 8.0;
/// At most this high, an image gets bands (`Global.min_height`).
const BAND_HEIGHT: f64 = 30.0;
/// The short side the detection model reads at least
/// (`Det.limit_side_len` with `limit_type: min`).
const DET_SHORT_SIDE: f64 = 736.0;
/// Our cap of the detection input's long side, for very tall images.
const DET_LONG_SIDE: usize = 4000;
/// The recognition model's input height (`Rec.rec_img_shape`).
const REC_HEIGHT: usize = 48;
/// The narrowest recognition input; shorter lines are padded.
const REC_WIDTH: usize = 320;
/// Lines with a lower mean character probability are dropped
/// (`Global.text_score`).
const TEXT_SCORE: f32 = 0.5;
/// Optimized detection plans kept, one per input size.
const PLANS: usize = 4;

type Plan = Arc<TypedRunnableModel>;

/// Text recognition with the built-in PP-OCRv6 models. Reads Latin
/// scripts (with accents), digits, punctuation and Chinese. Fully
/// offline.
#[derive(Debug, Default)]
pub struct PpOcr;

/// One line as [`PpOcr::read`] finds it.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrLine {
    /// The detected box of the line in image pixels, corners in reading
    /// order (top-left, top-right, bottom-right, bottom-left as the text
    /// reads).
    pub quad: Quad,
    /// The text as the model read it.
    pub text: String,
    /// The mean probability of the characters, 0.5 to 1.
    pub score: f32,
    /// The words, split at spaces, each with its box inside `quad`.
    pub words: Vec<TextWord>,
}

impl PpOcr {
    /// The recognizer. The models load on first use.
    pub fn new() -> Self {
        Self
    }

    /// Every line of text in `image`, in reading order, with its box,
    /// score and words. Returns [`Error::Cancelled`] soon after `cancel`
    /// is set.
    pub fn read(&self, image: &ImageBuf, cancel: &CancelToken) -> Result<Vec<OcrLine>> {
        cancel.check()?;
        let engine = engine()?;
        let (iw, ih) = (image.width() as usize, image.height() as usize);
        let (ww, wh) = working_size(iw, ih);
        let work = raster::from_image(image, ww, wh, cancel)?;
        let band = band(ww, wh);
        let work = if band > 0 { work.pad_rows(band) } else { work };
        cancel.check()?;

        let (dw, dh) = detection_size(work.width, work.height);
        let input = raster::resize(&work, dw, dh);
        let input = tensor(&[1, 3, dh, dw], raster::planes(&input, dw))?;
        let plan = engine.detection_plan(dw, dh)?;
        cancel.check()?;
        let out = run(&plan, input, cancel)?;
        let map = out[0].to_plain_array_view::<f32>().map_err(backend)?;
        if map.shape() != [1, 1, dh, dw] {
            return Err(Error::Backend(format!(
                "text detection gave {:?} for {dw} x {dh}",
                map.shape()
            )));
        }
        let map: Vec<f32> = map.iter().copied().collect();
        drop(out);
        cancel.check()?;
        let boxes = db::boxes(&map, dw, dh, (work.width, work.height));
        drop(map);

        // From the banded working copy back to image pixels.
        let (sx, sy) = (iw as f64 / ww as f64, ih as f64 / wh as f64);
        let to_image = |p: [f64; 2]| {
            Point::new(
                (p[0] * sx).clamp(0.0, iw as f64) as f32,
                ((p[1] - band as f64) * sy).clamp(0.0, ih as f64) as f32,
            )
        };
        let mut lines = Vec::new();
        for quad in &boxes {
            cancel.check()?;
            if let Some(line) = engine.read_line(&work, quad, cancel, &to_image)? {
                lines.push(line);
            }
        }
        Ok(lines)
    }
}

impl TextProvider for PpOcr {
    fn id(&self) -> &'static str {
        "pp-ocr"
    }

    fn name(&self) -> &'static str {
        "PP-OCRv6"
    }

    fn available(&self) -> bool {
        catch_unwind(engine).is_ok_and(|e| e.is_ok())
    }

    fn recognize(&self, image: &ImageBuf, cancel: &CancelToken) -> Result<TextLayout> {
        cancel.check()?;
        let lines = match catch_unwind(AssertUnwindSafe(|| self.read(image, cancel))) {
            Ok(lines) => lines?,
            Err(_) => {
                return Err(Error::Backend(
                    "text recognition stopped unexpectedly".into(),
                ))
            }
        };
        let mut out = TextLayout::new(image.width(), image.height());
        out.lines = lines
            .into_iter()
            .filter(|l| !l.words.is_empty())
            .map(|l| TextLine { words: l.words })
            .collect();
        Ok(out)
    }
}

/// The loaded models.
struct Engine {
    /// The detection model as parsed, before its input size is fixed.
    detection: Mutex<InferenceModel>,
    /// Optimized detection plans by (width, height), most recent last.
    plans: Mutex<Vec<((usize, usize), Plan)>>,
    /// The recognition model, optimized for any input width.
    recognition: Plan,
    /// The text of every output class: the blank, the model's characters
    /// and the space.
    charset: Vec<String>,
}

/// The models, loaded on first use. The error says why they did not load.
fn engine() -> Result<&'static Engine> {
    static ENGINE: OnceLock<std::result::Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| Engine::load().map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|why| {
            Error::Unavailable(format!("the text recognition models did not load: {why}"))
        })
}

impl Engine {
    fn load() -> TractResult<Engine> {
        let onnx = tract_onnx::onnx()
            .with_ignore_output_shapes(true)
            .with_ignore_value_info(true);
        let detection = onnx.model_for_read(&mut &DET_MODEL[..])?;
        let proto = onnx.proto_model_for_read(&mut &REC_MODEL[..])?;
        let characters = proto
            .metadata_props
            .iter()
            .find(|p| p.key == "character")
            .ok_or_else(|| TractError::msg("the recognition model has no character list"))?;
        // As the reference builds it: the blank first, the space last.
        let mut charset = vec!["blank".to_string()];
        charset.extend(characters.value.lines().map(str::to_string));
        charset.push(" ".to_string());
        let model = onnx.model_for_proto_model(&proto)?;
        let width = model.sym("W");
        let fact = InferenceFact::dt_shape(
            f32::datum_type(),
            tvec![1.to_dim(), 3.to_dim(), REC_HEIGHT.to_dim(), width.to_dim()],
        );
        let recognition = model
            .with_input_fact(0, fact)?
            .into_optimized()?
            .into_runnable()?;
        Ok(Engine {
            detection: Mutex::new(detection),
            plans: Mutex::new(Vec::new()),
            recognition,
            charset,
        })
    }

    /// The detection plan for a `width` x `height` input.
    fn detection_plan(&self, width: usize, height: usize) -> Result<Plan> {
        let key = (width, height);
        {
            let mut plans = lock(&self.plans);
            if let Some(i) = plans.iter().position(|(k, _)| *k == key) {
                let hit = plans.remove(i);
                let plan = Arc::clone(&hit.1);
                plans.push(hit);
                return Ok(plan);
            }
        }
        let model = lock(&self.detection).clone();
        let fact = InferenceFact::dt_shape(f32::datum_type(), [1, 3, height, width]);
        let plan = model
            .with_input_fact(0, fact)
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(backend)?;
        let mut plans = lock(&self.plans);
        if !plans.iter().any(|(k, _)| *k == key) {
            if plans.len() >= PLANS {
                plans.remove(0);
            }
            plans.push((key, Arc::clone(&plan)));
        }
        Ok(plan)
    }

    /// Read the line in `quad` (corners in pixels of `work`). None when
    /// the box is degenerate or the line reads as nothing.
    fn read_line(
        &self,
        work: &Bgr,
        quad: &[[f64; 2]; 4],
        cancel: &CancelToken,
        to_image: &impl Fn([f64; 2]) -> Point,
    ) -> Result<Option<OcrLine>> {
        // The straight crop (`get_rotate_crop_image`).
        let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let cw = d(quad[0], quad[1]).max(d(quad[2], quad[3])) as usize;
        let ch = d(quad[0], quad[3]).max(d(quad[1], quad[2])) as usize;
        if cw == 0 || ch == 0 {
            return Ok(None);
        }
        let Some(map) = Homography::from_rect(cw as f64, ch as f64, quad) else {
            return Ok(None);
        };
        let mut crop = raster::warp(work, &map, cw, ch);
        // Tall boxes are turned a quarter counter-clockwise; a point of
        // the turned crop maps back through the turn, then the crop.
        let turned = ch as f64 / cw as f64 >= 1.5;
        if turned {
            crop = crop.rot90();
        }
        let to_work = |x: f64, y: f64| {
            if turned {
                map.apply(cw as f64 - y, x)
            } else {
                map.apply(x, y)
            }
        };

        // The model input: 48 high, at least 320 wide, zeros on the right
        // (`resize_norm_img` with one line per batch).
        let (w, h) = (crop.width, crop.height);
        let ratio = w as f64 / h as f64;
        let max_ratio = (REC_WIDTH as f64 / REC_HEIGHT as f64).max(ratio);
        let input_width = (REC_HEIGHT as f64 * max_ratio) as usize;
        let scaled = ((REC_HEIGHT as f64 * ratio).ceil() as usize).clamp(1, input_width);
        let resized = raster::resize(&crop, scaled, REC_HEIGHT);
        let input = tensor(
            &[1, 3, REC_HEIGHT, input_width],
            raster::planes(&resized, input_width),
        )?;
        let out = run(&self.recognition, input, cancel)?;
        let probs = out[0].to_plain_array_view::<f32>().map_err(backend)?;
        let &[1, steps, classes] = probs.shape() else {
            return Err(Error::Backend(format!(
                "text recognition gave {:?}",
                probs.shape()
            )));
        };
        let probs: Vec<f32> = probs.iter().copied().collect();
        let decoded = ctc::decode(&probs, steps, classes, &self.charset);
        let text = decoded.text();
        if text.trim().is_empty() || decoded.score < TEXT_SCORE {
            return Ok(None);
        }

        // The steps the crop itself covers (`line_txt_len`). Word edges
        // come from the steps, then grow over the ink of letters they cut,
        // by at most two steps.
        let columns = steps as f64 * ratio / max_ratio;
        let mut found = ctc::words(&decoded, w as f64, columns);
        ctc::grow_to_ink(
            &mut found,
            &raster::ink_columns(&crop),
            2.0 * w as f64 / columns,
        );
        let corners = |x0: f64, x1: f64| {
            Quad::new([
                to_image(to_work(x0, 0.0)),
                to_image(to_work(x1, 0.0)),
                to_image(to_work(x1, h as f64)),
                to_image(to_work(x0, h as f64)),
            ])
        };
        let words = found
            .into_iter()
            .map(|(text, x0, x1)| TextWord {
                text,
                quad: corners(x0, x1),
            })
            .collect();
        let order = if turned { [1, 2, 3, 0] } else { [0, 1, 2, 3] };
        Ok(Some(OcrLine {
            quad: Quad::new(order.map(|i| to_image(quad[i]))),
            text,
            score: decoded.score,
            words,
        }))
    }
}

/// The size the reference reads a `width` x `height` image at
/// (`resize_image_within_bounds`): the long side at most [`MAX_SIDE`],
/// then the short side at least [`MIN_SIDE`]; a scaled side is rounded
/// to a multiple of 32 (and, unlike the reference, kept at 32 or more).
fn working_size(width: usize, height: usize) -> (usize, usize) {
    let (mut w, mut h) = (width.max(1), height.max(1));
    let scaled = |v: usize, ratio: f64| round32((v as f64 * ratio) as usize);
    if w.max(h) as f64 > MAX_SIDE {
        let ratio = MAX_SIDE / if h > w { h } else { w } as f64;
        (w, h) = (scaled(w, ratio), scaled(h, ratio));
    }
    if (w.min(h) as f64) < MIN_SIDE {
        let ratio = MIN_SIDE / if h < w { h } else { w } as f64;
        (w, h) = (scaled(w, ratio), scaled(h, ratio));
    }
    (w, h)
}

/// `v` rounded to a multiple of 32, halves to even as Python's `round`,
/// at least 32.
fn round32(v: usize) -> usize {
    (((v as f64 / 32.0).round_ties_even() * 32.0) as usize).max(32)
}

/// The black band above and below a `width` x `height` image
/// (`apply_vertical_padding`): none unless the image is very wide or very
/// low; then enough to make it about a quarter as high as wide.
fn band(width: usize, height: usize) -> usize {
    let (w, h) = (width as f64, height as f64);
    if h <= BAND_HEIGHT || w / h > BAND_RATIO {
        let target = ((w / BAND_RATIO).trunc()).max(BAND_HEIGHT) * 2.0;
        ((target - h).abs() / 2.0) as usize
    } else {
        0
    }
}

/// The detection input size for a `width` x `height` image
/// (`DetPreProcess.resize`): scaled up until the short side is
/// [`DET_SHORT_SIDE`], sides rounded to multiples of 32. Our addition:
/// the long side at most [`DET_LONG_SIDE`].
fn detection_size(width: usize, height: usize) -> (usize, usize) {
    let (w, h) = (width, height);
    let ratio = if (w.min(h) as f64) < DET_SHORT_SIDE {
        DET_SHORT_SIDE / if h < w { h } else { w } as f64
    } else {
        1.0
    };
    let (mut dw, mut dh) = (
        round32((w as f64 * ratio) as usize),
        round32((h as f64 * ratio) as usize),
    );
    let long = dw.max(dh);
    if long > DET_LONG_SIDE {
        let s = DET_LONG_SIDE as f64 / long as f64;
        (dw, dh) = (
            round32((dw as f64 * s) as usize).min(DET_LONG_SIDE),
            round32((dh as f64 * s) as usize).min(DET_LONG_SIDE),
        );
    }
    (dw, dh)
}

fn tensor(shape: &[usize], data: Vec<f32>) -> Result<Tensor> {
    tract_ndarray::ArrayD::from_shape_vec(shape, data)
        .map(Tensor::from)
        .map_err(|e| Error::Backend(format!("text recognition input: {e}")))
}

/// Run `plan` on `input`, checking `cancel` before every operation.
fn run(plan: &Plan, input: Tensor, cancel: &CancelToken) -> Result<TVec<TValue>> {
    let mut state = plan.spawn().map_err(backend)?;
    let result = state.run_plan_with_eval(tvec!(input.into()), |ctx, op_state, node, inputs| {
        if cancel.is_cancelled() {
            return Err(TractError::msg("cancelled"));
        }
        tract_onnx::tract_core::plan::eval(ctx, op_state, node, inputs)
    });
    result.map_err(|e| {
        if cancel.is_cancelled() {
            Error::Cancelled
        } else {
            backend(e)
        }
    })
}

fn backend(e: impl std::fmt::Display) -> Error {
    Error::Backend(format!("text recognition: {e}"))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
