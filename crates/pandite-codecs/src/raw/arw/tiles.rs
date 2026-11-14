//! Tile variants V4 (lossless CFA) and V5 (lossless YCbCr): every tile is
//! one lossless JPEG stream, decoded in parallel into its own chunk of a
//! tile-major plane.

use super::lj92::{self, LineStart, Stream, Target};
use super::meta::{Layout, Plan, TILE};
use super::plane::Plane;
use super::{fail, pool, ArwResult};
use pandite_core::CancelToken;

/// The tile byte ranges of the plan.
fn layout(plan: &Plan) -> ArwResult<(&[(usize, usize)], usize)> {
    match &plan.layout {
        Layout::Tiles { tiles, across } => Ok((tiles, *across)),
        Layout::Strips { .. } => Err(fail("not a tile layout")),
    }
}

/// The bytes of tile `t`, already checked against the file (R-ARW-68).
/// R-ARW-75: the decoder reads nothing outside this range.
fn tile_bytes<'a>(data: &'a [u8], tiles: &[(usize, usize)], t: usize) -> ArwResult<&'a [u8]> {
    let (off, len) = *tiles.get(t).ok_or_else(|| fail("tile index"))?;
    data.get(off..off.saturating_add(len))
        .ok_or_else(|| fail("tile outside the file"))
}

/// R-ARW-37: the V4 frame and scan.
pub(super) fn check_v4(s: &Stream<'_>) -> ArwResult<()> {
    let ok = s.precision == 14
        && s.samples == TILE / 2
        && s.lines == TILE / 2
        && s.components.len() == 4
        && s.components.iter().all(|c| c.h == 1 && c.v == 1)
        && s.scan.len() == 4;
    if ok {
        Ok(())
    } else {
        Err(fail("V4 tile frame differs from R-ARW-37"))
    }
}

/// R-ARW-40: the V5 frame and scan. Returns V of component 1.
fn check_v5(s: &Stream<'_>, subsampling: Option<(u32, u32)>) -> ArwResult<usize> {
    let c = &s.components;
    let ok = s.precision == 16
        && s.samples == TILE
        && s.lines == TILE
        && c.len() == 3
        && c[0].h == 2
        && (c[0].v == 1 || c[0].v == 2)
        && c[1..].iter().all(|k| k.h == 1 && k.v == 1)
        && s.scan.len() == 3;
    if !ok {
        return Err(fail("V5 tile frame differs from R-ARW-40"));
    }
    let v = c[0].v;
    // R-ARW-40 case 4: YCbCrSubSampling, when present, is 2 and V.
    if let Some(ss) = subsampling {
        if ss != (2, v as u32) {
            return Err(fail("YCbCrSubSampling disagrees with the tile frame"));
        }
    }
    Ok(v)
}

/// Decode a V4 file into a tile-major CFA plane of W x H samples on the
/// 14-bit scale (R-ARW-36, R-ARW-37, R-ARW-38).
pub(super) fn decode_cfa(data: &[u8], plan: &Plan, cancel: &CancelToken) -> ArwResult<Plane> {
    let (tiles, across) = layout(plan)?;
    let down = tiles.len() / across.max(1);
    let mut plane = Plane::tiled(TILE, TILE, across, down)?;
    let len = plane.tile_len();
    // R-ARW-38: component 1, 2, 3, 4 at frame (u, v) goes to tile pixel
    // (2u, 2v), (2u + 1, 2v), (2u, 2v + 1), (2u + 1, 2v + 1).
    let targets: Vec<Target> = (0..4)
        .map(|c| Target {
            buffer: 0,
            base: (c & 1) + (c >> 1) * TILE,
            xs: 2,
            ys: 2 * TILE,
        })
        .collect();
    let work: Vec<(usize, &mut [u16])> = plane.data.chunks_mut(len).enumerate().collect();
    // R-ARW-26: tile t is placed by its index, whatever the file order.
    pool::run(work, cancel, |(t, chunk)| {
        let s = lj92::parse(tile_bytes(data, tiles, t)?)?;
        check_v4(&s)?;
        // R-ARW-36: T.81 Annex H with the line-start rule of R-ARW-39
        // (no V4 component has V >= 2, so it changes nothing here).
        s.decode(&mut [chunk], &targets, LineStart::Sony)
    })?;
    Ok(plane)
}

/// The Y, Cb and Cr chunks of one V5 tile.
type YccTile<'a> = (&'a mut [u16], (&'a mut [u16], &'a mut [u16]));

/// The three planes of a V5 file.
pub(crate) struct YccPlanes {
    /// Luma, W x H, tile-major.
    pub y: Plane,
    /// Cb, (W / 2) x (H / V), tile-major with 256 x (512 / V) tiles.
    pub cb: Plane,
    /// Cr, like Cb.
    pub cr: Plane,
    /// Vertical sampling factor V of luma.
    pub v: usize,
}

/// Decode a V5 file into Y, Cb and Cr planes (R-ARW-36, R-ARW-39,
/// R-ARW-40).
pub(super) fn decode_ycc(data: &[u8], plan: &Plan, cancel: &CancelToken) -> ArwResult<YccPlanes> {
    let (tiles, across) = layout(plan)?;
    let down = tiles.len() / across.max(1);
    // V decides the chroma plane size: from YCbCrSubSampling when present,
    // else from the first tile; every tile must agree (R-ARW-40).
    let v = match plan.ycc.subsampling {
        Some((2, v @ 1..=2)) => v as usize,
        Some(_) => return Err(fail("YCbCrSubSampling is not 2 1 or 2 2")),
        None => check_v5(&lj92::parse(tile_bytes(data, tiles, 0)?)?, None)?,
    };
    let mut y = Plane::tiled(TILE, TILE, across, down)?;
    let mut cb = Plane::tiled(TILE / 2, TILE / v, across, down)?;
    let mut cr = Plane::tiled(TILE / 2, TILE / v, across, down)?;
    let (ylen, clen) = (y.tile_len(), cb.tile_len());
    let targets = [
        Target {
            buffer: 0,
            base: 0,
            xs: 1,
            ys: TILE,
        },
        Target {
            buffer: 1,
            base: 0,
            xs: 1,
            ys: TILE / 2,
        },
        Target {
            buffer: 2,
            base: 0,
            xs: 1,
            ys: TILE / 2,
        },
    ];
    let work: Vec<(usize, YccTile<'_>)> = y
        .data
        .chunks_mut(ylen)
        .zip(cb.data.chunks_mut(clen).zip(cr.data.chunks_mut(clen)))
        .enumerate()
        .collect();
    let expect = Some((2, v as u32));
    pool::run(work, cancel, |(t, (yt, (cbt, crt)))| {
        let s = lj92::parse(tile_bytes(data, tiles, t)?)?;
        check_v5(&s, expect)?;
        // R-ARW-36 with the line-start rule R-ARW-39 for luma (V = 2).
        s.decode(&mut [yt, cbt, crt], &targets, LineStart::Sony)
    })?;
    Ok(YccPlanes { y, cb, cr, v })
}
