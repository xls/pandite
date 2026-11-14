//! Per-model tables of the specification: the measured ColorMatrixSRGB
//! rows for files without a valid matrix tag (section Color conversion)
//! and the white level correction (section Levels and white balance).

/// One matrix row: the models (IFD0 Model, compared by R-SRW-9) and m11 to
/// m33 at scale 256 (rows are sRGB output channels, columns camera
/// channels).
type Row = (&'static [&'static str], [f64; 9]);

/// The measured matrices of tag 0xA031, in the order of the specification.
#[rustfmt::skip]
const TABLE: &[Row] = &[
    (&["NX5", "NX10", "NX11"], [408.0, -114.0, -38.0, -34.0, 348.0, -58.0, -8.0, -104.0, 368.0]),
    (&["NX20", "NX210", "NX1000", "NX1100"], [430.0, -152.0, -22.0, -36.0, 352.0, -60.0, 4.0, -128.0, 380.0]),
    (&["NX200"], [426.0, -164.0, -6.0, -24.0, 322.0, -42.0, 8.0, -120.0, 368.0]),
    (&["EX1"], [440.0, -180.0, -4.0, -80.0, 416.0, -80.0, 14.0, -188.0, 430.0]),
    (&["WB2000"], [530.0, -246.0, -28.0, -118.0, 520.0, -146.0, -26.0, -202.0, 484.0]),
];

/// The generic row for any other model: the element-wise mean of the 10
/// models above.
#[rustfmt::skip]
pub(super) const GENERIC: [f64; 9] = [
    434.0, -154.0, -24.0,
    -46.8, 371.0, -68.2,
    -1.2, -133.4, 390.6,
];

/// The white level table of R-SRW-31 rule 1.
const WHITE: &[(&str, u32)] = &[("EX1", 15881)];

/// The table matrix of `model` (R-SRW-35 source 3), unnormalized.
pub(super) fn table_matrix(model: &[u8]) -> Option<[f64; 9]> {
    TABLE
        .iter()
        .find(|r| r.0.iter().any(|m| m.as_bytes() == model))
        .map(|r| r.1)
}

/// The white level of `model` from the table (R-SRW-31 rule 1).
pub(super) fn table_white(model: &[u8]) -> Option<u32> {
    WHITE.iter().find(|r| r.0.as_bytes() == model).map(|r| r.1)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The element-wise mean of the 10 models of the table equals the
    /// generic row, and every row sums to 256: this checks the
    /// transcription of the table.
    #[test]
    fn srw_color_table_matches_generic_mean() {
        let models: usize = TABLE.iter().map(|r| r.0.len()).sum();
        assert_eq!(models, 10);
        for (k, generic) in GENERIC.iter().enumerate() {
            let mean = TABLE.iter().map(|r| r.1[k] * r.0.len() as f64).sum::<f64>() / models as f64;
            assert!((mean - generic).abs() < 1e-9, "element {k}: {mean}");
        }
        for m in TABLE.iter().map(|r| r.1).chain([GENERIC]) {
            for row in m.chunks_exact(3) {
                assert!((row.iter().sum::<f64>() - 256.0).abs() < 1e-9, "{m:?}");
            }
        }
    }
}
