//! Text from the recognition model's output, and word boxes from the time
//! steps of its characters (`CTCLabelDecode` and `CalRecBoxes` of the
//! reference).

/// One character the model read and the time step it was read at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub text: String,
    pub step: usize,
}

/// A decoded line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Decoded {
    /// The characters in order, spaces included.
    pub glyphs: Vec<Glyph>,
    /// The mean probability of the characters, 0 without characters.
    pub score: f32,
}

impl Decoded {
    /// The characters as a string.
    pub fn text(&self) -> String {
        self.glyphs.iter().map(|g| g.text.as_str()).collect()
    }
}

/// Greedy CTC decoding of `probs` (`steps` rows of `classes`
/// probabilities): the most likely class per step, repeats of the same
/// class merged, the blank (class 0) dropped. `charset[i]` is the text of
/// class `i`.
pub(crate) fn decode(probs: &[f32], steps: usize, classes: usize, charset: &[String]) -> Decoded {
    let mut glyphs = Vec::new();
    let mut sum = 0f64;
    let mut last = None;
    for (step, row) in probs.chunks_exact(classes).take(steps).enumerate() {
        // The first maximum, as numpy's argmax.
        let (mut best, mut p) = (0usize, f32::NEG_INFINITY);
        for (i, &v) in row.iter().enumerate() {
            if v > p {
                (best, p) = (i, v);
            }
        }
        if last != Some(best) && best != 0 {
            if let Some(text) = charset.get(best) {
                glyphs.push(Glyph {
                    text: text.clone(),
                    step,
                });
                sum += f64::from(p);
            }
        }
        last = Some(best);
    }
    let score = if glyphs.is_empty() {
        0.0
    } else {
        (sum / glyphs.len() as f64) as f32
    };
    Decoded { glyphs, score }
}

/// True for a CJK ideograph, CJK punctuation or a full-width form (the
/// reference's `is_chinese_char`).
fn is_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        ('\u{4e00}'..='\u{9fff}').contains(&c)
            || ('\u{3000}'..='\u{303f}').contains(&c)
            || ('\u{ff00}'..='\u{ffef}').contains(&c)
    })
}

fn is_space(text: &str) -> bool {
    !text.is_empty() && text.chars().all(char::is_whitespace)
}

/// The words of a decoded line and where each lies along it.
///
/// `width` is the crop width in pixels and `columns` the number of time
/// steps the crop itself spans (the rest of the model input is padding).
/// Each character gets a cell centered on its time step and as wide as
/// the average character; a word spans its characters' cells and words
/// that overlap meet halfway. Returns each word (text without spaces)
/// with its left and right edge in crop pixels, left to right.
pub(crate) fn words(line: &Decoded, width: f64, columns: f64) -> Vec<(String, f64, f64)> {
    if line.glyphs.is_empty() || columns <= 0.0 || width <= 0.0 {
        return Vec::new();
    }
    let col = width / columns;
    let char_width = average_char_width(line, col, width);
    let cell = |step: usize| {
        let center = (step as f64 + 0.5) * col;
        let x0 = (center - char_width / 2.0).trunc().max(0.0);
        let x1 = (center + char_width / 2.0).trunc().min(width);
        (x0, x1)
    };
    // Words: runs of characters between spaces.
    let mut out: Vec<(String, f64, f64)> = Vec::new();
    let mut current: Option<(String, f64, f64)> = None;
    for g in &line.glyphs {
        if is_space(&g.text) {
            out.extend(current.take());
            continue;
        }
        let (x0, x1) = cell(g.step);
        match &mut current {
            Some((text, a, b)) => {
                text.push_str(&g.text);
                *a = a.min(x0);
                *b = b.max(x1);
            }
            None => current = Some((g.text.clone(), x0, x1)),
        }
    }
    out.extend(current);
    // Overlapping neighbors meet in the middle (`adjust_box_overlap`).
    for i in 1..out.len() {
        let (prev_right, next_left) = (out[i - 1].2, out[i].1);
        if prev_right > next_left {
            let d = prev_right - next_left;
            out[i - 1].2 -= d / 2.0;
            out[i].1 += d - d / 2.0;
        }
    }
    out
}

/// Our refinement of the reference's word edges: an edge that cuts into
/// ink moves outward while the next column holds ink too (`ink[x]` for
/// crop column `x`), by at most `reach` pixels, never past the neighbor
/// word's edge or the crop. A word whose first or last letter is wider
/// than the average character (the w of "whistles") is then covered
/// whole. An edge in a gap stays. Words that meet after growing share
/// the overlap halfway.
pub(crate) fn grow_to_ink(words: &mut [(String, f64, f64)], ink: &[bool], reach: f64) {
    let width = ink.len();
    if width == 0 {
        return;
    }
    let edges: Vec<(f64, f64)> = words.iter().map(|w| (w.1, w.2)).collect();
    for (i, word) in words.iter_mut().enumerate() {
        let lo = if i == 0 { 0.0 } else { edges[i - 1].1 };
        let hi = if i + 1 == edges.len() {
            width as f64
        } else {
            edges[i + 1].0
        };
        // The box covers columns floor(left) to ceil(right) - 1.
        let (left, right) = (word.1, word.2);
        let first = (left.floor().max(0.0) as usize).min(width - 1);
        if ink[first] {
            let mut c = first;
            while c > 0 && (c - 1) as f64 >= lo && ink[c - 1] && left - (c - 1) as f64 <= reach {
                c -= 1;
            }
            if c < first {
                word.1 = c as f64;
            }
        }
        let last = (right.ceil().max(1.0) as usize - 1).min(width - 1);
        if ink[last] {
            let mut end = last + 1;
            while end < width
                && (end + 1) as f64 <= hi
                && ink[end]
                && (end + 1) as f64 - right <= reach
            {
                end += 1;
            }
            if end > last + 1 {
                word.2 = end as f64;
            }
        }
    }
    for i in 1..words.len() {
        if words[i - 1].2 > words[i].1 {
            let mid = (words[i - 1].2 + words[i].1) / 2.0;
            words[i - 1].2 = mid;
            words[i].1 = mid;
        }
    }
}

/// The reference's average character width: over its word groups of more
/// than one character (split at spaces, where the script changes between
/// CJK and other, and at gaps of more than 5 time steps), the mean step
/// distance between neighbors. Without such a group, the crop width over
/// the number of characters.
fn average_char_width(line: &Decoded, col: f64, width: f64) -> f64 {
    let mut widths = Vec::new();
    let mut group: Vec<usize> = Vec::new();
    let mut state: Option<bool> = None;
    let flush = |group: &mut Vec<usize>, widths: &mut Vec<f64>| {
        if group.len() > 1 {
            let span = (group[group.len() - 1] - group[0]) as f64 * col;
            widths.push(span / (group.len() - 1) as f64);
        }
        group.clear();
    };
    for (i, g) in line.glyphs.iter().enumerate() {
        if is_space(&g.text) {
            flush(&mut group, &mut widths);
            continue;
        }
        let cjk = is_cjk(&g.text);
        let gap = i > 0 && g.step - line.glyphs[i - 1].step > 5;
        if state.is_some_and(|s| s != cjk) || gap {
            flush(&mut group, &mut widths);
        }
        state = Some(cjk);
        group.push(g.step);
    }
    flush(&mut group, &mut widths);
    if widths.is_empty() {
        width / line.glyphs.len() as f64
    } else {
        widths.iter().sum::<f64>() / widths.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn charset() -> Vec<String> {
        ["blank", "a", "b", "c", " "]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Probabilities where step `i` picks class `picks[i]` at 0.9.
    fn probs(picks: &[usize]) -> Vec<f32> {
        let mut out = vec![0.025f32; picks.len() * 5];
        for (i, &c) in picks.iter().enumerate() {
            out[i * 5 + c] = 0.9;
        }
        out
    }

    #[test]
    fn decode_merges_repeats_and_drops_blanks() {
        // a a _ a b b _ _ space c c
        let p = probs(&[1, 1, 0, 1, 2, 2, 0, 0, 4, 3, 3]);
        let d = decode(&p, 11, 5, &charset());
        assert_eq!(d.text(), "aab c");
        let steps: Vec<usize> = d.glyphs.iter().map(|g| g.step).collect();
        assert_eq!(steps, vec![0, 3, 4, 8, 9]);
        assert!((d.score - 0.9).abs() < 1e-6);
        // Only blanks: no text, score 0.
        let d = decode(&probs(&[0, 0, 0]), 3, 5, &charset());
        assert_eq!(d.text(), "");
        assert_eq!(d.score, 0.0);
        // A class past the character list is ignored.
        let d = decode(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0], 1, 6, &charset());
        assert!(d.glyphs.is_empty());
    }

    #[test]
    fn words_split_at_spaces_and_follow_the_steps() {
        // "ab c" at steps 2, 4, (space 6), 10 of 20 columns over 200
        // pixels: 10 pixels per step, characters 20 pixels apart.
        let p = probs(&[0, 0, 1, 0, 2, 0, 4, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let d = decode(&p, 20, 5, &charset());
        assert_eq!(d.text(), "ab c");
        let w = words(&d, 200.0, 20.0);
        // Centers at 25, 45 and 105; cells 20 wide.
        assert_eq!(
            w,
            vec![
                ("ab".to_string(), 15.0, 55.0),
                ("c".to_string(), 95.0, 115.0)
            ]
        );
        // Without spaces, one word.
        let d = decode(&probs(&[1, 0, 2, 0, 3]), 5, 5, &charset());
        assert_eq!(words(&d, 50.0, 5.0).len(), 1);
        assert!(words(&decode(&probs(&[0]), 1, 5, &charset()), 10.0, 1.0).is_empty());
    }

    #[test]
    fn overlapping_words_meet_halfway_and_cells_stay_inside() {
        // Two one-letter words two steps apart: with no word of two
        // letters the cell is width / 3 characters = 10 pixels, centered
        // at 5 and 25 of a 30 pixel crop with 3 steps.
        let d = decode(&probs(&[1, 4, 2]), 3, 5, &charset());
        let w = words(&d, 30.0, 3.0);
        assert_eq!(
            w,
            vec![("a".to_string(), 0.0, 10.0), ("b".to_string(), 20.0, 30.0)]
        );
        // The same over 10 steps: centers 1.5 and 7.5, cells 0..6 (cut
        // at the crop's left edge) and 2..12 overlap and meet at 4.
        let w = words(&d, 30.0, 10.0);
        assert_eq!(
            w,
            vec![("a".to_string(), 0.0, 4.0), ("b".to_string(), 4.0, 12.0)]
        );
        // Cells never pass the crop's right edge.
        let d = decode(&probs(&[1, 2, 4, 3, 1]), 5, 5, &charset());
        let w = words(&d, 10.0, 5.0);
        assert_eq!(w.len(), 2);
        assert!(w[0].1 >= 0.0 && w[1].2 <= 10.0, "{w:?}");
        assert!(w[0].2 <= w[1].1, "{w:?}");
    }

    fn word(left: f64, right: f64) -> (String, f64, f64) {
        ("w".to_string(), left, right)
    }

    #[test]
    fn edges_that_cut_a_letter_grow_over_its_ink() {
        // Ink in columns 10 to 29 and 40 to 49.
        let ink: Vec<bool> = (0..60)
            .map(|x| (10..30).contains(&x) || (40..50).contains(&x))
            .collect();
        // Both edges cut the first word's ink: they grow to its ends.
        let mut w = vec![word(15.0, 28.0), word(42.0, 48.0)];
        grow_to_ink(&mut w, &ink, 10.0);
        assert_eq!(w, vec![word(10.0, 30.0), word(40.0, 50.0)]);
        // At most `reach` pixels.
        let mut w = vec![word(25.0, 28.0)];
        grow_to_ink(&mut w, &ink, 4.0);
        assert_eq!(w, vec![word(21.0, 30.0)]);
        // An edge in a gap stays, even with ink beyond it.
        let mut w = vec![word(32.0, 38.0)];
        grow_to_ink(&mut w, &ink, 10.0);
        assert_eq!(w, vec![word(32.0, 38.0)]);
        // Never past a neighbor; words that meet share the overlap.
        let ink = vec![true; 40];
        let mut w = vec![word(5.0, 15.0), word(20.0, 35.0)];
        grow_to_ink(&mut w, &ink, 100.0);
        assert_eq!(w, vec![word(0.0, 17.5), word(17.5, 40.0)]);
        // Nothing to grow over.
        let mut w = vec![word(0.0, 5.0)];
        grow_to_ink(&mut w, &[], 3.0);
        assert_eq!(w, vec![word(0.0, 5.0)]);
    }
}
