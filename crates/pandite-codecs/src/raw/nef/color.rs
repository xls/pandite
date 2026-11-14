//! Per-model color matrices and white balance scales (section Color
//! conversion of the specification, source S9, D50 blocks).

/// One table row: IFD0 Model, D50 WB scale R, WB scale B, and the matrix
/// m11 to m33 (rows are sRGB output channels, columns camera channels).
type Row = (&'static str, f64, f64, [f64; 9]);

/// The table of the specification, in its order.
#[rustfmt::skip]
const TABLE: &[Row] = &[
    ("NIKON D2H", 2.62, 1.27, [1.93, -0.73, -0.20, -0.13, 1.48, -0.35, 0.06, -0.51, 1.45]),
    ("NIKON D2X", 1.85, 1.56, [1.62, -0.64, 0.02, -0.09, 1.36, -0.27, 0.02, -0.26, 1.24]),
    ("NIKON D70", 2.16, 1.64, [1.66, -0.62, -0.04, -0.10, 1.44, -0.34, 0.03, -0.36, 1.33]),
    ("NIKON D70s", 2.16, 1.64, [1.66, -0.62, -0.04, -0.10, 1.44, -0.34, 0.03, -0.36, 1.33]),
    ("NIKON D50", 2.08, 1.50, [1.61, -0.56, -0.05, -0.10, 1.46, -0.36, 0.03, -0.37, 1.34]),
    ("NIKON D200", 2.22, 1.32, [1.77, -0.67, -0.10, -0.13, 1.43, -0.30, 0.00, -0.26, 1.26]),
    ("NIKON D80", 2.20, 1.19, [1.80, -0.70, -0.10, -0.10, 1.37, -0.27, 0.01, -0.25, 1.24]),
    ("NIKON D40", 2.11, 1.52, [1.64, -0.61, -0.02, -0.11, 1.45, -0.35, 0.03, -0.34, 1.32]),
    ("NIKON D40X", 1.80, 1.39, [1.56, -0.49, -0.07, -0.12, 1.39, -0.27, 0.03, -0.33, 1.30]),
    ("NIKON D60", 1.79, 1.38, [1.52, -0.47, -0.05, -0.11, 1.37, -0.27, 0.02, -0.31, 1.29]),
    ("NIKON D90", 2.10, 1.21, [1.98, -0.87, -0.11, -0.13, 1.48, -0.35, 0.03, -0.43, 1.40]),
    ("NIKON D300", 1.56, 1.28, [1.73, -0.61, -0.12, -0.14, 1.49, -0.36, 0.06, -0.54, 1.48]),
    ("NIKON D300S", 1.86, 1.18, [1.85, -0.72, -0.13, -0.13, 1.45, -0.32, 0.04, -0.42, 1.38]),
    ("NIKON D3", 2.24, 1.20, [1.79, -0.61, -0.19, -0.13, 1.42, -0.29, -0.01, -0.25, 1.26]),
    ("NIKON D3S", 2.18, 1.23, [1.90, -0.80, -0.10, -0.13, 1.46, -0.33, 0.04, -0.40, 1.36]),
    ("NIKON D3X", 2.66, 1.25, [1.77, -0.69, -0.08, -0.14, 1.57, -0.43, 0.02, -0.38, 1.36]),
    ("NIKON D700", 1.84, 1.41, [1.65, -0.52, -0.13, -0.12, 1.43, -0.31, 0.03, -0.38, 1.35]),
    ("NIKON D3000", 2.24, 1.24, [1.73, -0.69, -0.04, -0.11, 1.38, -0.27, 0.00, -0.24, 1.24]),
    ("NIKON D3100", 2.53, 1.14, [1.86, -0.78, -0.08, -0.15, 1.52, -0.37, 0.02, -0.37, 1.35]),
    ("NIKON D3200", 2.10, 1.36, [2.02, -1.01, -0.01, -0.16, 1.54, -0.38, 0.03, -0.56, 1.54]),
    ("NIKON D3300", 2.12, 1.47, [1.85, -0.74, -0.10, -0.17, 1.56, -0.39, 0.05, -0.47, 1.42]),
    ("NIKON D3400", 2.37, 1.75, [1.52, -0.46, -0.06, -0.09, 1.35, -0.26, 0.02, -0.33, 1.32]),
    ("NIKON D5000", 2.18, 1.15, [1.93, -0.86, -0.07, -0.18, 1.52, -0.34, -0.01, -0.43, 1.44]),
    ("NIKON D5100", 2.09, 1.52, [1.87, -0.80, -0.07, -0.16, 1.53, -0.37, 0.06, -0.50, 1.44]),
    ("NIKON D5200", 2.51, 1.74, [1.50, -0.45, -0.05, -0.07, 1.33, -0.26, 0.04, -0.39, 1.35]),
    ("NIKON D5300", 2.12, 1.49, [1.84, -0.73, -0.11, -0.16, 1.56, -0.40, 0.05, -0.49, 1.44]),
    ("NIKON D5500", 2.00, 1.51, [1.87, -0.83, -0.04, -0.18, 1.57, -0.39, 0.09, -0.56, 1.47]),
    ("NIKON D5600", 2.05, 1.47, [1.88, -0.79, -0.09, -0.19, 1.60, -0.41, 0.04, -0.46, 1.43]),
    ("NIKON D7000", 2.11, 1.50, [1.87, -0.81, -0.06, -0.16, 1.55, -0.39, 0.05, -0.47, 1.42]),
    ("NIKON D7100", 2.50, 1.76, [1.49, -0.44, -0.05, -0.07, 1.30, -0.23, 0.04, -0.37, 1.33]),
    ("NIKON D7200", 2.47, 1.69, [1.54, -0.52, -0.03, -0.08, 1.35, -0.27, 0.04, -0.39, 1.35]),
    ("NIKON D7500", 2.29, 1.63, [1.74, -0.74, 0.00, -0.15, 1.46, -0.32, 0.07, -0.59, 1.52]),
    ("NIKON D600", 1.95, 1.49, [1.89, -0.80, -0.09, -0.15, 1.52, -0.37, 0.07, -0.50, 1.43]),
    ("NIKON D610", 1.94, 1.49, [1.87, -0.78, -0.09, -0.16, 1.54, -0.38, 0.07, -0.50, 1.43]),
    ("NIKON D800", 2.04, 1.48, [1.94, -0.86, -0.08, -0.16, 1.54, -0.38, 0.06, -0.49, 1.43]),
    ("NIKON D800E", 2.02, 1.49, [1.89, -0.82, -0.08, -0.16, 1.55, -0.39, 0.07, -0.51, 1.44]),
    ("NIKON D810", 2.00, 1.43, [1.88, -0.77, -0.11, -0.15, 1.53, -0.38, 0.05, -0.48, 1.43]),
    ("NIKON D850", 1.93, 1.49, [1.64, -0.58, -0.06, -0.10, 1.36, -0.26, 0.04, -0.48, 1.44]),
    ("NIKON D750", 1.98, 1.44, [1.92, -0.79, -0.13, -0.17, 1.58, -0.41, 0.07, -0.55, 1.48]),
    ("NIKON D4", 2.14, 1.59, [1.69, -0.60, -0.08, -0.09, 1.34, -0.25, 0.05, -0.42, 1.37]),
    ("NIKON D4S", 2.12, 1.53, [1.71, -0.58, -0.12, -0.11, 1.40, -0.29, 0.04, -0.42, 1.37]),
    ("NIKON D5", 2.28, 1.63, [1.61, -0.52, -0.09, -0.09, 1.39, -0.30, -0.01, -0.42, 1.43]),
    ("NIKON D500", 2.32, 1.63, [1.70, -0.68, -0.02, -0.14, 1.47, -0.33, 0.06, -0.53, 1.46]),
    ("NIKON Df", 2.14, 1.56, [1.65, -0.55, -0.10, -0.09, 1.34, -0.25, 0.05, -0.40, 1.35]),
    ("NIKON Z 6", 1.75, 1.34, [1.92, -0.82, -0.10, -0.15, 1.48, -0.33, 0.05, -0.55, 1.50]),
    ("NIKON Z 7", 1.76, 1.38, [1.78, -0.70, -0.08, -0.12, 1.42, -0.29, 0.04, -0.49, 1.44]),
    ("NIKON Z 6_2", 1.73, 1.33, [1.94, -0.84, -0.10, -0.15, 1.50, -0.35, 0.04, -0.52, 1.48]),
    ("NIKON Z 7_2", 1.76, 1.37, [1.81, -0.72, -0.09, -0.14, 1.44, -0.31, 0.03, -0.46, 1.43]),
    ("NIKON Z 9", 1.86, 1.44, [1.74, -0.64, -0.10, -0.13, 1.43, -0.30, 0.02, -0.43, 1.41]),
];

/// The generic matrix for models outside the table: the element-wise mean
/// of the 49 matrices, rounded to 3 decimals.
#[rustfmt::skip]
pub(super) const GENERIC: [f64; 9] = [
    1.766, -0.686, -0.079,
    -0.130, 1.458, -0.329,
    0.038, -0.427, 1.389,
];

/// The generic white balance scales R and B (R-NEF-38 source 5).
pub(super) const GENERIC_WB: (f64, f64) = (2.099, 1.443);

/// The table row of `model`, compared after R-NEF-10 (the caller passes
/// the Model up to its first zero byte without trailing spaces).
fn row(model: &[u8]) -> Option<&'static Row> {
    TABLE.iter().find(|r| r.0.as_bytes() == model)
}

/// The D50 white balance scales (R, B) of `model`, when the table has it
/// (R-NEF-38 source 4).
pub(super) fn table_wb(model: &[u8]) -> Option<(f64, f64)> {
    row(model).map(|r| (r.1, r.2))
}

/// Each row divided by its sum (R-NEF-42), or `None` when a row sum is not
/// greater than 0.
fn normalized(m: &[f64; 9]) -> Option<[[f64; 3]; 3]> {
    let mut out = [[0.0; 3]; 3];
    for (r, o) in out.iter_mut().enumerate() {
        let src = &m[3 * r..3 * r + 3];
        let sum: f64 = src.iter().sum();
        // R-NEF-67: the divisor is checked to be greater than 0.
        if sum <= 0.0 {
            return None;
        }
        for (c, v) in o.iter_mut().enumerate() {
            *v = src[c] / sum;
        }
    }
    Some(out)
}

/// The camera-to-sRGB matrix for `model`, and whether it came from the
/// table. R-NEF-41: the row whose Model equals the IFD0 Model, else the
/// generic matrix. R-NEF-42: each row divided by its sum, and the generic
/// matrix when a row sum of the selected matrix is not greater than 0.
pub(super) fn matrix(model: &[u8]) -> ([[f64; 3]; 3], bool) {
    select(row(model).map(|r| r.3))
}

/// The normalized selected matrix and true, or the normalized generic
/// matrix and false when there is none or a row sum of the selected one is
/// not greater than 0 (R-NEF-42).
pub(super) fn select(selected: Option<[f64; 9]>) -> ([[f64; 3]; 3], bool) {
    if let Some(m) = selected.as_ref().and_then(normalized) {
        return (m, true);
    }
    // The generic rows sum to 0.999 to 1.001, so the identity is never used.
    let generic =
        normalized(&GENERIC).unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    (generic, false)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The element-wise mean of the table equals the generic matrix of the
    /// specification to 3 decimals, and the WB means equal the generic WB
    /// scales: this checks the transcription of all 49 rows.
    #[test]
    fn nef_color_table_matches_generic_mean() {
        assert_eq!(TABLE.len(), 49);
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
        // Every row sums to about 1 and is positive (R-NEF-42).
        for r in TABLE {
            for k in 0..3 {
                let s: f64 = r.3[3 * k..3 * k + 3].iter().sum();
                assert!((s - 1.0).abs() <= 0.02 + 1e-9, "{} row {k}: {s}", r.0);
            }
        }
        // Model names are distinct.
        for (i, r) in TABLE.iter().enumerate() {
            assert!(TABLE[..i].iter().all(|o| o.0 != r.0), "{} twice", r.0);
        }
    }
}
