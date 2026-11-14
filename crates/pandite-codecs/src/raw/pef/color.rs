//! Per-model color matrices and white balance scales (section Color
//! conversion of the specification, source S11, D50 blocks).

/// One table row: IFD0 Model, D50 WB scale R, WB scale B, and the matrix
/// m11 to m33 (rows are sRGB output channels, columns camera channels).
type Row = (&'static str, f64, f64, [f64; 9]);

/// The table of the specification, in its order.
#[rustfmt::skip]
const TABLE: &[Row] = &[
    ("PENTAX K10D", 2.08, 1.03, [1.70, -0.59, -0.10, -0.08, 1.33, -0.25, 0.01, -0.23, 1.23]),
    ("PENTAX K20D", 2.12, 1.17, [1.82, -0.66, -0.15, -0.14, 1.34, -0.20, 0.02, -0.45, 1.43]),
    ("PENTAX K200D", 2.15, 1.20, [1.70, -0.62, -0.08, -0.08, 1.35, -0.26, 0.02, -0.26, 1.24]),
    ("PENTAX K-7", 2.18, 1.07, [1.75, -0.55, -0.20, -0.10, 1.30, -0.20, 0.03, -0.38, 1.35]),
    ("PENTAX K-x", 2.22, 1.20, [1.91, -0.75, -0.15, -0.13, 1.48, -0.36, 0.01, -0.34, 1.33]),
    ("PENTAX K-r", 1.87, 1.47, [1.76, -0.62, -0.14, -0.11, 1.42, -0.32, 0.05, -0.42, 1.37]),
    ("PENTAX K-5", 2.24, 1.64, [1.82, -0.73, -0.09, -0.15, 1.54, -0.39, 0.06, -0.46, 1.40]),
    ("PENTAX K-5 II", 2.16, 1.61, [1.68, -0.57, -0.11, -0.14, 1.52, -0.38, 0.06, -0.47, 1.41]),
    ("PENTAX K-5 II s", 2.10, 1.72, [1.69, -0.57, -0.12, -0.14, 1.49, -0.35, 0.07, -0.49, 1.43]),
    ("PENTAX K-01", 2.22, 1.68, [1.77, -0.69, -0.08, -0.14, 1.51, -0.37, 0.08, -0.50, 1.42]),
    ("PENTAX K-30", 2.19, 1.71, [1.67, -0.55, -0.13, -0.18, 1.59, -0.41, 0.05, -0.46, 1.40]),
    ("PENTAX K-50", 2.19, 1.65, [1.78, -0.70, -0.08, -0.14, 1.52, -0.38, 0.08, -0.49, 1.41]),
    ("PENTAX K-500", 2.26, 1.66, [1.77, -0.70, -0.07, -0.14, 1.52, -0.38, 0.07, -0.48, 1.41]),
    ("PENTAX K-3", 2.32, 1.58, [1.82, -0.73, -0.09, -0.15, 1.55, -0.40, 0.05, -0.47, 1.42]),
    ("PENTAX K-3 II", 2.22, 1.58, [1.82, -0.74, -0.08, -0.16, 1.58, -0.42, 0.06, -0.51, 1.45]),
    ("PENTAX K-S1", 2.24, 1.72, [1.70, -0.61, -0.09, -0.16, 1.53, -0.37, 0.07, -0.46, 1.39]),
    ("PENTAX K-1", 2.07, 1.67, [1.80, -0.67, -0.13, -0.15, 1.54, -0.39, 0.07, -0.47, 1.40]),
    ("PENTAX 645D", 2.18, 1.20, [1.99, -0.88, -0.11, -0.19, 1.57, -0.38, 0.11, -0.75, 1.64]),
    ("PENTAX 645Z", 2.06, 1.61, [1.87, -0.81, -0.06, -0.16, 1.56, -0.40, 0.09, -0.59, 1.50]),
];

/// The generic matrix for models outside the table: the element-wise mean
/// of the table, rounded to 3 decimals.
#[rustfmt::skip]
const GENERIC: [f64; 9] = [
    1.780, -0.671, -0.108,
    -0.139, 1.486, -0.348,
    0.056, -0.457, 1.402,
];

/// The generic white balance scales R and B (means of the table).
pub(super) const GENERIC_WB: (f64, f64) = (2.162, 1.483);

/// The table row of `model` (compared after R-PEF-10 trimming).
fn row(model: &[u8]) -> Option<&'static Row> {
    TABLE.iter().find(|r| r.0.as_bytes() == model)
}

/// The D50 white balance scales (R, B) of `model`, when the table has it
/// (R-PEF-40 source 2).
pub(super) fn table_wb(model: &[u8]) -> Option<(f64, f64)> {
    row(model).map(|r| (r.1, r.2))
}

/// The camera-to-sRGB matrix for `model` with every row divided by its
/// sum, and whether it came from the table. R-PEF-43: the table row whose
/// Model equals the IFD0 Model, else the generic matrix. R-PEF-44: each row
/// is divided by its sum.
pub(super) fn matrix(model: &[u8]) -> ([[f64; 3]; 3], bool) {
    let (m, from_table) = match row(model) {
        Some(r) => (r.3, true),
        None => (GENERIC, false),
    };
    let mut out = [[0.0; 3]; 3];
    for (r, o) in out.iter_mut().enumerate() {
        let src = &m[3 * r..3 * r + 3];
        let sum: f64 = src.iter().sum();
        // R-PEF-67: a divisor must be greater than 0; every row sums to
        // about 1, so the guard only protects against a broken table.
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
    /// scales: this checks the transcription of all 19 rows.
    #[test]
    fn pef_color_table_matches_generic_mean() {
        assert_eq!(TABLE.len(), 19);
        for (k, generic) in GENERIC.iter().enumerate() {
            let mean = TABLE.iter().map(|r| r.3[k]).sum::<f64>() / TABLE.len() as f64;
            assert!(
                (mean - generic).abs() <= 0.0005 + 1e-9,
                "element {k}: mean {mean} generic {generic}"
            );
        }
        let n = TABLE.len() as f64;
        let wr = TABLE.iter().map(|r| r.1).sum::<f64>() / n;
        let wb = TABLE.iter().map(|r| r.2).sum::<f64>() / n;
        assert!((wr - GENERIC_WB.0).abs() <= 0.0005, "WB R mean {wr}");
        assert!((wb - GENERIC_WB.1).abs() <= 0.0005, "WB B mean {wb}");
    }
}
