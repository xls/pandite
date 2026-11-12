//! Per-model color matrices and white balance scales (section Color
//! conversion of the specification, source S9, D50 blocks).

/// One table row: IFD0 Model, D50 WB scale R, WB scale B, and the matrix
/// m11 to m33 (rows are sRGB output channels, columns camera channels).
type Row = (&'static str, f64, f64, [f64; 9]);

/// The table of the specification, in its order.
#[rustfmt::skip]
const TABLE: &[Row] = &[
    ("ILCE-7", 2.46, 1.53, [1.93, -0.79, -0.14, -0.16, 1.59, -0.43, 0.03, -0.44, 1.41]),
    ("ILCE-7R", 2.52, 1.59, [2.02, -0.88, -0.13, -0.18, 1.62, -0.44, 0.04, -0.45, 1.41]),
    ("ILCE-7S", 2.54, 1.65, [2.29, -1.10, -0.20, -0.25, 1.71, -0.46, 0.12, -0.70, 1.58]),
    ("ILCE-7M2", 2.34, 1.54, [1.99, -0.91, -0.08, -0.16, 1.56, -0.41, 0.06, -0.52, 1.47]),
    ("ILCE-7RM2", 2.43, 1.61, [1.95, -0.88, -0.06, -0.17, 1.55, -0.39, 0.05, -0.48, 1.43]),
    ("ILCE-7SM2", 2.47, 1.63, [2.37, -1.25, -0.11, -0.24, 1.69, -0.45, 0.16, -0.79, 1.64]),
    ("ILCE-7M3", 2.32, 1.59, [2.10, -0.96, -0.14, -0.17, 1.58, -0.41, 0.04, -0.50, 1.46]),
    ("ILCE-7RM3", 2.37, 1.63, [2.01, -0.91, -0.11, -0.18, 1.57, -0.39, 0.04, -0.52, 1.48]),
    ("ILCE-7RM4", 2.52, 1.71, [1.97, -0.85, -0.11, -0.17, 1.61, -0.44, 0.04, -0.46, 1.42]),
    ("ILCE-7SM3", 2.44, 1.51, [2.31, -1.23, -0.08, -0.18, 1.65, -0.47, 0.00, -0.51, 1.51]),
    ("ILCE-7M4", 2.47, 1.61, [2.03, -0.92, -0.11, -0.16, 1.59, -0.43, 0.04, -0.47, 1.44]),
    ("ILCE-7RM5", 2.53, 1.62, [1.95, -0.87, -0.08, -0.15, 1.58, -0.43, 0.03, -0.45, 1.41]),
    ("ILCE-7C", 2.47, 1.57, [2.26, -1.19, -0.07, -0.17, 1.63, -0.47, 0.00, -0.44, 1.44]),
    ("ILCE-9", 2.40, 1.65, [2.06, -0.92, -0.14, -0.18, 1.59, -0.41, 0.04, -0.46, 1.43]),
    ("ILCE-9M2", 2.45, 1.54, [2.28, -1.21, -0.07, -0.17, 1.65, -0.48, -0.01, -0.43, 1.44]),
    ("ILCE-1", 2.39, 1.64, [1.85, -0.74, -0.11, -0.14, 1.48, -0.35, 0.02, -0.40, 1.38]),
    ("ILCE-5000", 2.75, 1.69, [1.80, -0.69, -0.11, -0.17, 1.60, -0.43, 0.05, -0.43, 1.38]),
    ("ILCE-5100", 2.75, 1.81, [2.03, -0.91, -0.12, -0.22, 1.65, -0.42, 0.09, -0.56, 1.47]),
    ("ILCE-6000", 2.86, 1.75, [2.08, -0.96, -0.12, -0.23, 1.68, -0.45, 0.08, -0.54, 1.46]),
    ("ILCE-6300", 2.59, 1.82, [2.04, -0.91, -0.13, -0.24, 1.65, -0.41, 0.06, -0.49, 1.44]),
    ("ILCE-6400", 2.61, 1.81, [2.05, -0.96, -0.09, -0.19, 1.57, -0.38, 0.09, -0.56, 1.47]),
    ("ILCE-6500", 2.59, 1.85, [2.11, -1.00, -0.11, -0.21, 1.61, -0.40, 0.08, -0.53, 1.44]),
    ("ILCE-6600", 2.80, 1.83, [2.07, -0.99, -0.08, -0.19, 1.59, -0.40, 0.09, -0.55, 1.47]),
    ("NEX-3N", 2.66, 1.64, [1.90, -0.82, -0.08, -0.17, 1.59, -0.43, 0.04, -0.44, 1.39]),
    ("NEX-5N", 2.53, 1.61, [1.98, -0.88, -0.10, -0.17, 1.60, -0.43, 0.04, -0.47, 1.43]),
    ("NEX-5R", 2.56, 1.60, [1.88, -0.79, -0.09, -0.16, 1.57, -0.41, 0.04, -0.45, 1.41]),
    ("NEX-5T", 2.69, 1.64, [1.82, -0.69, -0.12, -0.16, 1.59, -0.43, 0.03, -0.42, 1.39]),
    ("NEX-6", 2.47, 1.61, [1.82, -0.69, -0.14, -0.16, 1.56, -0.40, 0.04, -0.45, 1.41]),
    ("NEX-7", 2.60, 1.52, [1.98, -0.92, -0.07, -0.16, 1.59, -0.42, 0.03, -0.45, 1.42]),
    ("SLT-A77V", 2.61, 1.54, [2.01, -0.94, -0.08, -0.17, 1.59, -0.42, 0.03, -0.47, 1.44]),
    ("SLT-A99V", 2.31, 1.53, [1.92, -0.79, -0.14, -0.15, 1.55, -0.40, 0.04, -0.49, 1.45]),
    ("DSC-RX100", 2.14, 2.29, [1.76, -0.74, -0.03, -0.20, 1.44, -0.25, 0.09, -0.62, 1.53]),
    ("DSC-RX100M3", 2.43, 1.87, [1.82, -0.76, -0.06, -0.20, 1.51, -0.31, 0.08, -0.71, 1.62]),
    ("DSC-RX100M4", 2.69, 1.90, [1.89, -0.82, -0.06, -0.21, 1.54, -0.34, 0.08, -0.65, 1.57]),
    ("DSC-RX100M7", 2.22, 1.75, [1.90, -0.80, -0.11, -0.18, 1.62, -0.44, 0.04, -0.57, 1.52]),
    ("DSC-RX1RM2", 2.45, 1.72, [1.98, -0.93, -0.05, -0.18, 1.57, -0.39, 0.06, -0.52, 1.46]),
];

/// The generic matrix for models outside the table: the element-wise mean
/// of the table, rounded to 3 decimals.
#[rustfmt::skip]
const GENERIC: [f64; 9] = [
    2.006, -0.906, -0.101,
    -0.182, 1.592, -0.412,
    0.052, -0.511, 1.459,
];

/// The generic white balance scales R and B (means of the table).
pub(super) const GENERIC_WB: (f64, f64) = (2.512, 1.678);

/// The table row of `model` (compared after R-ARW-10 trimming).
fn row(model: &[u8]) -> Option<&'static Row> {
    TABLE.iter().find(|r| r.0.as_bytes() == model)
}

/// The D50 white balance scales (R, B) of `model`, when the table has it
/// (R-ARW-46 source 2).
pub(super) fn table_wb(model: &[u8]) -> Option<(f64, f64)> {
    row(model).map(|r| (r.1, r.2))
}

/// The camera-to-sRGB matrix for `model` with every row divided by its
/// sum, and whether it came from the table. R-ARW-50: the table row whose
/// Model equals the IFD0 Model, else the generic matrix. R-ARW-51: each row
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
        // R-ARW-78: a divisor must be greater than 0; every table row sums
        // to about 1, so the guard only protects against a broken table.
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
    /// scales: this checks the transcription of all 36 rows.
    #[test]
    fn arw_color_table_matches_generic_mean() {
        assert_eq!(TABLE.len(), 36);
        for (k, generic) in GENERIC.iter().enumerate() {
            let mean = TABLE.iter().map(|r| r.3[k]).sum::<f64>() / TABLE.len() as f64;
            assert!(
                (mean - generic).abs() <= 0.0005 + 1e-9,
                "element {k}: mean {mean} generic {generic}"
            );
        }
        let wr = TABLE.iter().map(|r| r.1).sum::<f64>() / 36.0;
        let wb = TABLE.iter().map(|r| r.2).sum::<f64>() / 36.0;
        assert!((wr - GENERIC_WB.0).abs() <= 0.0005, "WB R mean {wr}");
        assert!((wb - GENERIC_WB.1).abs() <= 0.0005, "WB B mean {wb}");
    }
}
