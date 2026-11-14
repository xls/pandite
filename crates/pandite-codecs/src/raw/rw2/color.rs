//! Per-model color matrices and white balance scales (section Color
//! conversion of the specification, source S6, D50 blocks).

/// One table row: IFD0 Model, D50 WB scale R, WB scale B, and the matrix
/// m11 to m33 (rows are sRGB output channels, columns camera channels).
type Row = (&'static str, f64, f64, [f64; 9]);

/// The table of the specification, in its order.
#[rustfmt::skip]
const TABLE: &[Row] = &[
    ("DMC-LX3", 2.55, 1.52, [1.81, -0.84, 0.03, -0.14, 1.63, -0.49, 0.08, -0.76, 1.68]),
    ("DMC-G1", 2.39, 1.32, [1.68, -0.65, -0.03, -0.18, 1.55, -0.36, 0.01, -0.39, 1.38]),
    ("DMC-GH1", 2.84, 1.45, [1.82, -0.79, -0.03, -0.17, 1.72, -0.55, 0.07, -0.66, 1.59]),
    ("DMC-GF1", 2.32, 1.28, [1.70, -0.71, 0.01, -0.16, 1.48, -0.33, 0.03, -0.43, 1.40]),
    ("DMC-G2", 1.95, 1.52, [1.68, -0.64, -0.04, -0.16, 1.47, -0.31, 0.05, -0.49, 1.44]),
    ("DMC-GH2", 2.33, 1.79, [1.83, -0.80, -0.03, -0.19, 1.65, -0.46, 0.09, -0.77, 1.68]),
    ("DMC-G3", 2.59, 1.71, [1.77, -0.82, 0.05, -0.21, 1.69, -0.48, 0.13, -0.77, 1.64]),
    ("DMC-GX1", 2.59, 1.74, [1.75, -0.74, 0.00, -0.20, 1.68, -0.49, 0.12, -0.80, 1.67]),
    ("DMC-GH3", 2.34, 1.97, [2.11, -1.06, -0.05, -0.24, 1.67, -0.43, 0.13, -0.71, 1.58]),
    ("DMC-G5", 2.28, 1.76, [1.91, -0.87, -0.04, -0.19, 1.68, -0.49, 0.12, -0.88, 1.76]),
    ("DMC-GX7", 2.39, 1.66, [1.77, -0.68, -0.09, -0.16, 1.59, -0.42, 0.09, -0.71, 1.63]),
    ("DMC-GM1", 2.46, 1.74, [1.70, -0.55, -0.15, -0.15, 1.56, -0.42, 0.08, -0.67, 1.59]),
    ("DMC-GH4", 2.35, 1.96, [2.05, -0.95, -0.11, -0.24, 1.67, -0.42, 0.11, -0.65, 1.54]),
    ("DMC-LX100", 2.36, 1.75, [1.77, -0.69, -0.08, -0.16, 1.57, -0.41, 0.11, -0.76, 1.65]),
    ("DMC-GX8", 2.23, 2.05, [2.10, -1.08, -0.02, -0.24, 1.64, -0.40, 0.14, -0.72, 1.58]),
    ("DMC-G80", 2.29, 1.60, [1.70, -0.55, -0.15, -0.15, 1.58, -0.43, 0.07, -0.63, 1.57]),
    ("DMC-FZ1000", 2.32, 1.98, [1.78, -0.72, -0.07, -0.21, 1.53, -0.32, 0.10, -0.72, 1.62]),
    ("DMC-ZS100", 2.42, 1.90, [1.88, -0.81, -0.07, -0.23, 1.57, -0.33, 0.08, -0.71, 1.63]),
    ("DC-GH5", 2.14, 2.01, [2.00, -0.91, -0.09, -0.22, 1.61, -0.38, 0.09, -0.61, 1.52]),
    ("DC-GH5M2", 2.19, 2.03, [2.01, -0.96, -0.05, -0.23, 1.61, -0.39, 0.10, -0.71, 1.61]),
    ("DC-S1", 2.17, 1.80, [1.91, -0.83, -0.07, -0.15, 1.48, -0.33, 0.08, -0.63, 1.56]),
    ("DC-S1R", 2.04, 1.59, [1.61, -0.52, -0.09, -0.08, 1.15, -0.07, 0.05, -0.40, 1.35]),
    ("DC-S5", 2.20, 1.78, [1.94, -0.85, -0.09, -0.17, 1.55, -0.38, 0.07, -0.63, 1.56]),
];

/// The generic matrix for models outside the table: the element-wise mean
/// of the table, rounded to 3 decimals.
#[rustfmt::skip]
const GENERIC: [f64; 9] = [
    1.838, -0.783, -0.055,
    -0.184, 1.580, -0.395,
    0.087, -0.661, 1.575,
];

/// The generic white balance scales R and B (means of the table, R-RW2-34
/// source 3).
pub(super) const GENERIC_WB: (f64, f64) = (2.337, 1.735);

/// The table row of `model`: the Model string compared after the trailing
/// zero bytes and spaces are removed (R-RW2-12).
fn row(model: &[u8]) -> Option<&'static Row> {
    TABLE.iter().find(|r| r.0.as_bytes() == model)
}

/// The D50 white balance scales (R, B) of `model`, when the table has it
/// (R-RW2-34 source 2).
pub(super) fn table_wb(model: &[u8]) -> Option<(f64, f64)> {
    row(model).map(|r| (r.1, r.2))
}

/// The camera-to-sRGB matrix for `model`, and whether it came from the
/// table. R-RW2-36: the row of the table whose Model equals the IFD0
/// Model, else the generic matrix. R-RW2-37: each row is divided by its
/// sum, so every row sums to 1.
pub(super) fn matrix(model: &[u8]) -> ([[f64; 3]; 3], bool) {
    let (m, from_table) = match row(model) {
        Some(r) => (r.3, true),
        None => (GENERIC, false),
    };
    let mut out = [[0.0; 3]; 3];
    for (r, o) in out.iter_mut().enumerate() {
        let src = &m[3 * r..3 * r + 3];
        let sum: f64 = src.iter().sum();
        // R-RW2-62: a divisor must be greater than 0. Every row of the
        // table sums to about 1; the guard only protects a broken table.
        let sum = if sum > 0.0 { sum } else { 1.0 };
        for (c, v) in o.iter_mut().enumerate() {
            *v = src[c] / sum;
        }
    }
    (out, from_table)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The element-wise mean of the table equals the generic matrix of the
    /// specification to 3 decimals, and the WB means equal the generic WB
    /// scales: this checks the transcription of all 23 rows.
    #[test]
    fn rw2_color_table_matches_generic_mean() {
        assert_eq!(TABLE.len(), 23);
        let n = TABLE.len() as f64;
        for (k, generic) in GENERIC.iter().enumerate() {
            let mean = TABLE.iter().map(|r| r.3[k]).sum::<f64>() / n;
            assert!(
                (mean - generic).abs() <= 0.0005 + 1e-9,
                "element {k}: mean {mean} generic {generic}"
            );
        }
        let wr = TABLE.iter().map(|r| r.1).sum::<f64>() / n;
        let wb = TABLE.iter().map(|r| r.2).sum::<f64>() / n;
        assert!((wr - GENERIC_WB.0).abs() <= 0.0005 + 1e-9, "WB R mean {wr}");
        assert!((wb - GENERIC_WB.1).abs() <= 0.0005 + 1e-9, "WB B mean {wb}");
        // Every row sums to 1 within 0.01 (section Matrix sources).
        for r in TABLE {
            for k in 0..3 {
                let s: f64 = r.3[3 * k..3 * k + 3].iter().sum();
                assert!((s - 1.0).abs() <= 0.01 + 1e-9, "{} row {k}: {s}", r.0);
            }
        }
    }
}
