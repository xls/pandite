//! A 16-bit sample plane stored either row-major or tile-major, so that
//! tiles decode in parallel into contiguous chunks.

use super::{fail, ArwResult};

/// A plane of `u16` samples. Row-major storage is one tile as wide and as
/// tall as the plane.
pub(crate) struct Plane {
    /// The samples, tile after tile, each tile row-major.
    pub data: Vec<u16>,
    /// Tile width.
    pub tw: usize,
    /// Tile height.
    pub th: usize,
    /// Tiles per tile row.
    pub across: usize,
}

/// A zeroed vector that reports allocation failure instead of aborting.
pub(super) fn zeroed(len: usize) -> ArwResult<Vec<u16>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| fail("not enough memory for the sensor data"))?;
    v.resize(len, 0);
    Ok(v)
}

impl Plane {
    /// A row-major plane of `w` x `h` samples.
    pub(crate) fn row_major(w: usize, h: usize) -> ArwResult<Self> {
        let len = w.checked_mul(h).ok_or_else(|| fail("plane size"))?;
        Ok(Plane {
            data: zeroed(len)?,
            tw: w,
            th: h,
            across: 1,
        })
    }

    /// A tile-major plane of `across` x `down` tiles of `tw` x `th`.
    pub(super) fn tiled(tw: usize, th: usize, across: usize, down: usize) -> ArwResult<Self> {
        let len = tw
            .checked_mul(th)
            .and_then(|v| v.checked_mul(across))
            .and_then(|v| v.checked_mul(down))
            .ok_or_else(|| fail("plane size"))?;
        Ok(Plane {
            data: zeroed(len)?,
            tw,
            th,
            across,
        })
    }

    /// Samples in one tile.
    pub(super) fn tile_len(&self) -> usize {
        self.tw * self.th
    }

    /// Row `y` of tile column `tc`: `tw` samples starting at plane column
    /// `tc * tw`.
    #[inline]
    pub(super) fn row(&self, y: usize, tc: usize) -> &[u16] {
        let tile = (y / self.th) * self.across + tc;
        let start = tile * self.tw * self.th + (y % self.th) * self.tw;
        self.data.get(start..start + self.tw).unwrap_or(&[])
    }
}
