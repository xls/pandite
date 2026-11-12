# Test fixtures

Most decode tests build their input in the test code itself: PNG, APNG, GIF, lossless WebP, BMP, TIFF, TGA, ICO, QOI, Radiance HDR, PSD, DDS, KTX2, OpenEXR, SVG and DNG files are written by the crate's own encoders, by encoder crates that are already dependencies, or byte by byte from the format specification.

The files in this folder cover formats that have no encoder among the dependencies. Every image here is a synthetic pattern of solid color blocks drawn by `make_fixtures.py` or `make_heic.rs.txt`. No photograph or third-party artwork is included.

License: the fixtures were created for pandite and are released under the project license (MIT). They carry no third-party rights.

## Regenerate

1. Install Pillow 12 (with libwebp and libavif) and ffmpeg 8 (with libjxl and libaom).
2. Run `python -I make_fixtures.py` in this folder.
3. For `blocks.heic`, build `make_heic.rs.txt` as a Rust program on Windows with the HEIF Image Extensions and HEVC Video Extensions installed, then run it with the output path.

Lossy encoders can produce different bytes on other versions. The tests compare colors with a tolerance, so regenerated files still pass.

## Files

| File | Format | Content | Made with |
|---|---|---|---|
| `lossy.webp` | WebP lossy (VP8) | 64 x 32: left (200, 40, 40), right (30, 90, 210) | Pillow 12.2.0, libwebp 1.6.0, quality 90 |
| `lossy_alpha.webp` | WebP lossy with ALPH | 32 x 32 green (40, 180, 60): top opaque, bottom alpha 128 | Pillow 12.2.0, libwebp 1.6.0, alpha_quality 100 |
| `lossy_anim.webp` | Animated WebP, lossy frames | 32 x 32: frame 1 red 80 ms, frame 2 adds a blue quarter 120 ms, infinite loop | Pillow 12.2.0, libwebp 1.6.0 |
| `blocks.avif` | AVIF, 8-bit 4:4:4 | 64 x 64: red, green, blue, white quadrants | Pillow 12.2.0, libavif 1.4.1, quality 90 |
| `alpha.avif` | AVIF with alpha item | 32 x 32 yellow (250, 200, 20): top opaque, bottom alpha 64 | Pillow 12.2.0, libavif 1.4.1 |
| `rotated.avif` | AVIF with `irot` | stored 64 x 32 (left red, right blue), EXIF orientation 6 written as `irot` | Pillow 12.2.0, libavif 1.4.1 |
| `deep.avif` | AVIF, 10-bit 4:2:0 | 64 x 64 quadrants as in `blocks.avif` | ffmpeg 8.0.1, libaom, `-pix_fmt yuv420p10le -crf 8` |
| `grid.avif` | AVIF grid item | 2 x 1 grid of 32 x 32 tiles (red, blue), output 60 x 32 | Tiles from Pillow 12.2.0 with libavif 1.4.1; container assembled byte by byte by `grid_avif()` in `make_fixtures.py` |
| `blocks.jxl` | JPEG XL lossless | 40 x 24: left (230, 30, 30) opaque, right (30, 40, 220) alpha 128 | ffmpeg 8.0.1, libjxl, `-distance 0` |
| `lossy.jxl` | JPEG XL lossy (XYB) | 40 x 24: left red, right blue | ffmpeg 8.0.1, libjxl, `-distance 1` |
| `anim.jxl` | JPEG XL animation | 16 x 16, two frames at 5 fps: red, then blue | ffmpeg 8.0.1, libjxl_anim, `-distance 0` |
| `jpeg.tif` | TIFF, JPEG compression (YCbCr) | 64 x 32: left red, right blue | Pillow 12.2.0, libtiff 4.7.1, quality 95 |
| `blocks.heic` | HEIC (HEVC) | 64 x 64 quadrants as in `blocks.avif` | Windows Imaging Component HEIF encoder (HEIF Image Extensions 1.2.65.0, HEVC Video Extensions 2.5.48.0, Windows 11), program `make_heic.rs.txt` |

`make_heic.rs.txt` is kept as text so cargo does not build it with the test suite. It needs the `windows` crate 0.62 with the features listed at its top.
