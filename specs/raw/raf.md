# Fujifilm RAF raw format: decoder specification

Document: specs/raw/raf.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite RAF decoder (clean-room step 1 of 2).

This document describes the Fujifilm RAF raw file format as facts. It is written for an implementer who has only this document, TIFF 6.0 (S1), ITU-T T.81 (S2) for the embedded JPEG preview, and Exif 2.32 (S4) for the orientation tag. Every normative statement traces to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

The structure follows the Sony specification `arw.md`. Where a rule is the same as a rule of `arw.md`, the text says so, so that an implementer of both formats can share code. This document repeats every such rule in full, so `arw.md` is not needed to implement RAF.

### Conventions

- Requirements have the form R-RAF-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x49 0x49 or "49 49".
- Bit 0 is the least significant bit of a value.
- floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0).
- Pixel coordinates are (x, y): x is the column, y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (FujiIFD tags 0xF001 and 0xF002).
- "BE32" is an unsigned 32-bit big-endian integer. "BE16" is an unsigned 16-bit big-endian integer. "LE16" and "LE32" are the little-endian forms.

### Glossary

- RAF: the raw file format of Fujifilm cameras (FinePix, X series and GFX). A RAF file is not a TIFF file. It has its own header, which points to an embedded JPEG, a metadata container and a CFA container.
- Header: the first 148 bytes of a RAF file.
- Meta container: a list of tagged records in big-endian byte order (S6, S7). ExifTool (S5) calls its tags "FujiFilm RAF Tags".
- CFA container: the block that holds the sensor data. In all files in scope it is a small little-endian TIFF structure with one private IFD, the FujiIFD, followed by the sensor data.
- FujiIFD: the IFD that the CFA container's IFD0 tag 0xF000 points to (S5 name "FujiIFD").
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2).
- CFA: color filter array. RAF files in scope use either a 2x2 Bayer pattern or the 6x6 X-Trans pattern.
- X-Trans: the 6x6 color filter array of Fujifilm X series cameras (S8). Each 6x6 cell holds 20 green, 8 red and 8 blue pixels.
- Raster: the stored W x H array of sensor values.
- Crop rectangle: the part of the raster that forms the final image (meta tags 0x0110 and 0x0111).
- Variant: one storage method of the sensor data. This document defines three, V1 to V3, in the section Scope and variants.
- Layout: the color filter arrangement, C1 (X-Trans) or C2 (Bayer RGGB).
- Black level: the sensor value that represents zero light.
- White level: the sensor value at saturation.
- WB multipliers: the per-channel gains that make a neutral surface give equal red, green and blue values.
- Camera RGB: red, green and blue values in the spectral sensitivities of the camera.
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S10). The decoder output is linear sRGB.
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Container layout (CFA container IFDs, field types), Decoding procedure (orientation values) |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Embedded preview (marker structure, SOF0, baseline decoding), Error handling |
| S3 | Digital Negative (DNG) Specification 1.6.0.0, Adobe, December 2021 (copy read at the URL, official landing page https://helpx.adobe.com/camera-raw/digital-negative.html) | https://paulbourke.net/dataformats/dng/dng_spec_1_6_0_0.pdf | official | 2026-11-14 | Color conversion (meaning of ColorMatrix2 and CalibrationIlluminant2) |
| S4 | Exif Version 2.32, CIPA DC-X008-Translation-2019 | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-14 | Decoding procedure (APP1 Exif segment, Orientation tag 0x0112), Embedded preview (IFD1 thumbnail) |
| S5 | ExifTool Tag Names: FujiFilm tags table, including "FujiFilm RAFHeader Tags", "FujiFilm RAF Tags" and "FujiFilm IFD Tags" (metadata tag reference documentation) | https://exiftool.org/TagNames/FujiFilm.html | community documentation | 2026-11-14 | Scope and variants (RAFCompression), Container layout, Metadata tags |
| S6 | libopenraw file format documentation: Fujifilm RAF (format description page, prose only) | https://libopenraw.pages.freedesktop.org/formats/raf/ | community documentation | 2026-11-14 | Container layout (magic, header fields, big-endian order, meta container record format) |
| S7 | libopenraw wiki: Fuji RAF (format description page, prose only, last edited 2013) | https://wiki.freedesktop.org/libopenraw/Fuji_RAF/ | community documentation | 2026-11-14 | Container layout (header fields, meta container record format) |
| S8 | Fujifilm Corp. (assignee), Color imaging element, US Patent 8,456,553 B2, filed 2012-07-27, priority 2011-02-21, published 2013-06-04 | https://patents.google.com/patent/US8456553B2/en | patent | 2026-11-14 | Sensor data layout (6x6 X-Trans basic array pattern, 20 G, 8 R, 8 B) |
| S9 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear interpolation as the non-adaptive method) |
| S10 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (primaries 0.64/0.33, 0.30/0.60, 0.15/0.06 and white point 0.3127/0.3290) |
| S11 | Measurement: tags of the X100S DNG sample fujifilm-x100s-daylight-DSCF9505.dng, written by Adobe DNG Converter 10.3 (first 256 KiB read, CC0) | https://raw.pixls.us/data/Fujifilm/X100S/fujifilm-x100s-daylight-DSCF9505.dng | measurement | 2026-11-14 | Color conversion, record M12 |
| S12 | Measurement: FinePix X100 sample DSCF1676 (CC0) | https://raw.pixls.us/data/Fujifilm/FinePix%20X100/DSCF1676.RAF | measurement | 2026-11-14 | all sections, records M1 to M14 |
| S13 | Measurement: X-Pro1 sample DSCF2131 (CC0) | https://raw.pixls.us/data/Fujifilm/X-Pro1/DSCF2131.RAF | measurement | 2026-11-14 | all sections, records M1 to M14 |
| S14 | Measurement: X-A5 sample DSCF0617 (CC0) | https://raw.pixls.us/data/Fujifilm/X-A5/DSCF0617.RAF | measurement | 2026-11-14 | Compression (V2), records M5, M9 to M13 |
| S15 | Measurement: X-T1 sample 20171229_110916 (CC0) | https://raw.pixls.us/data/Fujifilm/X-T1/20171229_110916.RAF | measurement | 2026-11-14 | Compression (V3), Sensor data layout, records M6 to M13 |
| S16 | Measurement: X-T3 sample AFXT2720 (CC0) | https://raw.pixls.us/data/Fujifilm/X-T3/AFXT2720.RAF | measurement | 2026-11-14 | all sections, test vectors TV-1 to TV-5 |
| S17 | Measurement: X-T4 samples DSCF0267 (lossless compressed) and DSCF0268 (lossy compressed) (CC0) | https://raw.pixls.us/data/Fujifilm/X-T4/ | measurement | 2026-11-14 | Scope and variants (X1, X2), record M15 |
| S18 | Measurement: GFX 50S sample 20170525_0036TEST (CC0) | https://raw.pixls.us/data/Fujifilm/GFX%2050S/20170525_0036TEST.RAF | measurement | 2026-11-14 | Compression (V3), Sensor data layout (Bayer phase), records M6, M9 to M13 |
| S19 | Measurement: FinePix S5Pro sample 2018_06150003 and X10 sample DSCF0514 (CC0) | https://raw.pixls.us/data/Fujifilm/ | measurement | 2026-11-14 | Scope and variants (X3, X4), record M15 |
| S20 | Measurement: container survey of 143 RAF files from 79 camera folders (header, meta container and first 4 KiB of the CFA container read with HTTP range requests, CC0 files only, the 8 folders with CC BY-NC-SA files skipped) | https://raw.pixls.us/data/Fujifilm/ | measurement | 2026-11-14 | Scope and variants, Container layout, Metadata tags, Sensor data layout, records M1 to M3, M15 |

Sample files are from the public raw sample archive raw.pixls.us. The archive lists a license for each file, and every file used here is marked CC0 (public domain). The author downloaded the files and inspected them with measurement tools that the author wrote from S1, S2, S4 and the hypotheses of this document. The tools are not part of the deliverable. One comparison decoded the cameras' own embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (header, S12 to S20): all 143 survey files and all 10 local files start with the 16 bytes "FUJIFILMCCD-RAW " (S6). Bytes 16 to 19 are "0201" in 142 files and "0200" in the FinePix S2Pro file, whose JPEG starts at offset 108. In every "0201" file the JPEG starts at offset 148 and the six BE32 fields at offsets 84 to 104 are, in order, JPEG offset, JPEG length, meta container offset, meta container length, CFA container offset and CFA container length (S6, S7). The meta container ends exactly at the CFA container offset in all local single-image files. The CFA container ends at the end of the file in all local single-image files. In all 10 local files the meta container starts 0 to 110480 bytes after the JPEG end, and every byte between them is 0.
- M2 (RAFCompression, S16, S17, S20): among the 142 survey files of format version "0201", the BE32 value at offset 108 is 0 in 82 files, 2 in 38 files and 3 in 22 files. S5 names header index 108 "RAFCompression" with 0 = Uncompressed, 2 = Lossless, 3 = Lossy. The archive labels agree: X-T4 DSCF0267 ("lossless compressed") has 2 and DSCF0268 ("compressed") has 3. In the 60 files with value 2 or 3 the sensor data hold 0.59 to 1.28 bytes per pixel, which matches none of V1 to V3. The words at offsets 116 to 147 are zero in all in-scope files. The word at offset 112 is 0 or equal to the CFA container length.
- M3 (meta container and FujiIFD, S12 to S20): the meta container is a BE32 record count followed by records of BE16 tag, BE16 size and size data bytes (S6, S7). Records 0x0100 hold (H, W) as two BE16 values, equal to FujiIFD 0xF002 and 0xF001 in all 61 in-scope files. In every in-scope file the CFA container starts with 49 49 2A 00 08 00 00 00. IFD0 has one entry, tag 0xF000, type 13, count 1, value 26, so the FujiIFD starts at byte 26 of the CFA container. FujiIFD offsets count from the first byte of the CFA container: the sensor data start at CFA container offset + 0xF007 (2048 in 9 local files, 1024 in X-A5), and CFA container length = 0xF007 + 0xF008 in all local files. No in-scope meta container or FujiIFD holds a tag with 9 or more signed values that could be a color matrix.
- M4 (V1 packing, S12, S13): with BitsPerSample 12 and StripByteCounts = 3 * W * H / 2, the reading "first pixel in the low 12 bits of a little-endian 24-bit group" gives a mean absolute difference between pixels two columns apart of 4.64 (X100), the big-endian reading gives 37.9. In X-Pro1 the means of the 36 positions of the 6x6 cell form three tight clusters (580 to 583, 797 to 799, 1038 to 1043) with the little-endian reading only. The mean same-color difference alternates between 12.50 to 12.52 and 11.63 to 11.66 and does not change with x mod 8 otherwise, so the pixel order inside the 3-byte groups is consistent.
- M5 (V2 packing, S14): with BitsPerSample 14 and StripByteCounts = 7 * W * H / 4, eight bit orders were tested on 40 rows. Reading each 4 bytes as a LE32 word and taking 14-bit values most significant bit first gives a same-color difference of 64.0 (horizontal) and 74.1 (vertical). The seven other orders (single bytes and 16-bit and 64-bit words, each most or least significant bit first) give 2901 or more. The difference does not change with x mod 16 (118 to 121), so the order inside a 16-pixel group is consistent.
- M6 (V3, S15, S16, S18): with StripByteCounts = 2 * W * H, LE16 samples give a mean absolute difference between pixels 6 columns apart of 99 (X-T1) and 73 (X-T3), BE16 samples give 17050 and 20382. The LE16 maximum is 16383 in the 14-bit files X-T1 and GFX 50S.
- M7 (X-Trans layout, S13, S15, S16, S8): for the 36 positions (x mod 6, y mod 6) of the raster the mean sensor values form three clusters. The 36 bytes of meta record 0x0131 read in reverse order (the last byte for (0, 0), then right to left and bottom to top) assign one value to each cluster with a spread of at most 31 inside a cluster, in all three files. The bytes in forward order mix the clusters (spread 460 or more). With 0 = red, 1 = green, 2 = blue the reversed reading is the S8 basic array pattern shifted by 2 rows and 2 columns (X-Pro1, X-T3) and has 20 green, 8 red and 8 blue positions. All 37 X-Trans in-scope survey files carry one of three 0x0131 byte strings, and each is a shift of the S8 pattern.
- M8 (red and blue of X-Trans, S13, S15, S16): with color 0 as red and 2 as blue, the mean chromaticity error against the embedded JPEG (method of M12, no color matrix) is 0.115 (X-Pro1), 0.040 (X-T1) and 0.051 (X-T3). With 0 as blue and 2 as red it is 0.168, 0.256 and 0.156.
- M9 (Bayer phase, S12, S14, S18): all three files carry meta record 0x0130 = 0A 0B 09 08 and no record 0x0131. Red at (even x, even y) and blue at (odd x, odd y) gives a chromaticity error (method of M12, no color matrix) of 0.068 (X100), 0.057 (X-A5) and 0.047 (GFX 50S). The other three phases give 0.122 to 0.279. The 2x2 means confirm green at (1, 0) and (0, 1): 435 and 435 against 378 and 385 (X100), 5426 and 5426 against 3248 and 3747 (X-A5), 1942 and 1941 against 1123 and 1227 (GFX 50S).
- M10 (levels, S12 to S16, S18): the darkest non-zero values lie just above the FujiIFD 0xF00A values: X-T3 1011 (black 1022), GFX 50S 70 and 0.01 percentile 78 (black 65), X-A5 0.01 percentile 1047 (black 1024), X100 257 (black 254 and 255). Saturation plateaus: X-Pro1 has 16637 pixels at 4094 (12-bit), X-T1 has 113 green pixels at 16322 (14-bit), X-A5 has 7606 pixels at 16381 (14-bit). The other samples have no clipped areas.
- M11 (white balance order, S12 to S16, S18): FujiIFD 0xF00E holds three values. Reading them as G, R, B (S5 name WB_GRBLevels) and using R/G and B/G as multipliers gives, with the matrix of R-RAF-35, a chromaticity error (method of M12) of 0.024 (X-T1), 0.037 (X-A5), 0.036 (X100) and 0.019 (GFX 50S). Swapping the R and B values gives 0.104, 0.104, 0.054 and 0.022. X-T3 (R 583, B 584: 0.050 against 0.050) and X-Pro1 (0.079 against 0.075) do not discriminate.
- M12 (color matrix, S11 to S16, S18): the DNG of S11 (UniqueCameraModel "Fujifilm X100S", Software "Adobe DNG Converter 10.3 (Windows)") holds ColorMatrix2 = 1.0592 -0.4262 -0.1008 / -0.3514 1.1355 0.2465 / -0.0870 0.2025 0.6386 with CalibrationIlluminant2 = 21 (D65 in S3). The matrix of R-RAF-35 is derived from it (section Color conversion). Method: the raw image is decoded by this document, the crop is divided into 40 x 60 blocks, and each block mean is compared with the block mean of the embedded JPEG after removal of the sRGB transfer function. Blocks with any channel below 0.01 or above 0.8 (raw) or below 0.003 or above 0.9 (JPEG) are skipped. The error is the mean Euclidean distance of the chromaticities r / (r + g + b) and g / (r + g + b). Results (no matrix, matrix of R-RAF-35, the same matrix transposed): X100 0.068, 0.036, 0.073. X-Pro1 0.115, 0.079, 0.134. X-A5 0.057, 0.037, 0.094. X-T1 0.040, 0.024, 0.078. X-T3 0.051, 0.050, 0.100. GFX 50S 0.047, 0.019, 0.086. The matrix improves or keeps the agreement in every file, and the transposed matrix is worse in every file.
- M13 (crop, S12, S13, S15, S16, S18): the crop rectangle of meta tags 0x0110 and 0x0111 matches the framing of the embedded JPEG. A search over shifts of -12 to +12 raw pixels finds the best correlation at (0, 0) for X-T1, within 2 raw pixels for X100 and X-T3, and at (-5, -3) for X-Pro1, which is 2 JPEG pixels. No pixel inside the crop rectangle and no pixel of the 1-pixel ring around it is 0. Outside the ring the rasters have bands of zeros: X100 columns 0 to 67 and 4378 to 4447, X-Pro1 columns 4952 to 5119, X-T1 columns 0 to 3 and 4940 to 4991, X-T3 rows 0 to 4 and 4181, GFX 50S columns 8280 to 9215 and rows 6208 and 6209.
- M14 (previews, S12 to S19): the JPEG at header offsets 84/88 is a baseline JPEG (SOF0) in all 10 local files: 1920 x 1280 (X-Pro1, X-T1, X-A5), 2176 x 1448 (X100), 2048 x 1536 (X10), 1440 x 960 (S5Pro), 4000 x 3000 (GFX 50S), 4416 x 2944 (X-T3, X-T4). Each starts with FF D8 FF E1 and an Exif APP1 segment with a little-endian TIFF header, Orientation 1 in IFD0, and a 160 x 120 JPEG thumbnail in IFD1. Each ends with FF D9 at the stated length. All decode with a baseline JPEG decoder.
- M15 (out-of-scope structures, S17, S19, S20): legacy files (14 folders: FinePix S2Pro, S3Pro, S5Pro, S100FS, S200EXR, S5000, S5200, S5500, S6000fd, S6500fd, S7000, S9500, S9600, DBP for GX680) have no TIFF header at the CFA container offset. Two-image files (8 folders: FinePix F550EXR, F770EXR, HS30EXR, HS33EXR, HS50EXR, X10, X-S1, XF1) have a TIFF CFA container and a second meta container and CFA container, whose offsets are at header offsets 120 and 128. In X10 the second CFA container starts with 49 49 2A 00 at offset 10298640.
- M16 (X-Trans neighborhoods): for each of the 36 positions of the measured X-Trans pattern, the 8 neighbors contain both colors that the pixel lacks: a red pixel has 5 green and 3 blue neighbors, a blue pixel has 3 red and 5 green neighbors, a green pixel has 2 red, 4 green and 2 blue neighbors. Every 3 x 3 window of the raster therefore contains all three colors.

### Sources encountered and not used

The web searches for this document returned links to decoder source code. They were seen only as titles and short summaries in search results and were not opened: TestDisk `file_raf.c` (git.cgsecurity.org), exiv2 `rafimage.cpp` (grok.dragonflybsd.org and git.stg.centos.org), exifprobe `raf.c` (sources.debian.org), an Android framework commit (gitlab.e.foundation), a Go raw package (pkg.go.dev), and LibRaw `libraw_xtrans_compressed.cpp` (cdn.jsdelivr.net). Forum threads about Fujifilm compression and CFA parsing (libraw.org/node/2635, libraw.org/node/2619, discuss.pixls.us threads 38811 and 4527) were seen only as search results and were not opened. Search summaries that described the content of these pages were not used. The DXOMARK measurement page of the FinePix X100 and its data file were opened to look for a color matrix: the matrix is all zeros there, and nothing was used. See the author's report for the full list.

## Scope and variants

### Storage variants

The sample archive labels RAF files as uncompressed, lossless compressed and compressed (S17, S20). S5 names the header value RAFCompression with the values 0 = Uncompressed, 2 = Lossless and 3 = Lossy, and the labels agree with it (M2). Uncompressed files use one of three storage methods (M3 to M6):

| Variant | Storage | FujiIFD 0xF003 BitsPerSample | FujiIFD 0xF008 StripByteCounts | Measured in |
| --- | --- | --- | --- | --- |
| V1 | 12-bit packed, 2 pixels in 3 bytes, little-endian groups | 12 | 3 * W * H / 2 | X100 (S12), X-Pro1 (S13) |
| V2 | 14-bit packed, 16 pixels in 7 LE32 words, most significant bit first | 14 | 7 * W * H / 4 | X-A5 (S14) |
| V3 | one LE16 value per pixel | 14 or 16 | 2 * W * H | X-T1 (S15), X-T3 (S16), GFX 50S (S18) |

### Color filter layouts

| Layout | Meta tags | Pattern |
| --- | --- | --- |
| C1 X-Trans | 0x0131 present (36 bytes), 0x0130 = 0C 0C 0C 0C in all measured files | 6x6 pattern from tag 0x0131 (R-RAF-18) |
| C2 Bayer | 0x0131 absent, 0x0130 = 0A 0B 09 08 | 2x2 RGGB: red at (even x, even y) (R-RAF-19) |

### Models in scope

The survey (S20) found these combinations in uncompressed single-image files. Crop is (left, top, width x height). Black is the set of 0xF00A values seen.

| Variant | Layout | Bits | Raster W x H | Crop origin | Crop size | Black | Models |
| --- | --- | --- | --- | --- | --- | --- | --- |
| V1 | C1 | 12 | 4096 x 3012 | 16, 6 | 4000 x 3000 | 256 | X20, X30, XQ1, XQ2 |
| V1 | C1 | 12 | 5120 x 3288 | 28, 12 | 4896 x 3264 | 255 | X-E1, X-M1, X-Pro1 |
| V1 | C2 | 12 | 4448 x 2870 | 79, 11 | 4288 x 2848 | 254, 255 | FinePix X100 |
| V1 | C2 | 12 | 4624 x 3470 | 0, 0 | 4624 x 3470 | 200 | FinePix S1, FinePix SL1000 |
| V1 | C2 | 12 | 4912 x 3278 | 8, 7 | 4896 x 3264 | 256 | X-A10 |
| V1 | C2 | 12 | 5120 x 3288 | 28, 12 | 4896 x 3264 | 255 | X-A1, X-A2 |
| V2 | C2 | 14 | 6016 x 4014 | 8, 7 | 6000 x 4000 | 1024 | X-A3, X-A5, X-A7, X-T100, X-T200, XF10 |
| V3 | C1 | 14 | 4992 x 3296 | 24, 16 | 4896 x 3264 | 1020 to 1028 | X-E2, X-E2S, X-T1, X-T10, X100S, X100T, X70 |
| V3 | C1 | 14 | 6160 x 4032 | 16, 16 | 6000 x 4000 | 1020 to 1023 | X-E3, X-H1, X-Pro2, X-T2, X-T20, X100F |
| V3 | C1 | 14 | 6336 x 4182 | 6, 13 | 6240 x 4160 | 1019 to 1023 | X-M5, X-S20, X-T30 III |
| V3 | C1 | 14 | 6336 x 4188 | 12, 13 | 6240 x 4160 | 1024 | X-H2S |
| V3 | C1 | 14 | 6384 x 4182 | 6, 13 | 6240 x 4160 | 1022 | X-E4, X-Pro3, X-S10, X-T3, X-T30, X-T30 II, X-T4, X100V |
| V3 | C1 | 14 | 7872 x 5196 | 12, 21 | 7728 x 5152 | 1020 to 1023 | X-E5, X-H2, X-T5, X-T50, X100VI |
| V3 | C2 | 14 | 8472 x 6210 | 12, 8 | 8256 x 6192 | 64 | GFX50S II |
| V3 | C2 | 14 | 9216 x 6210 | 12, 8 | 8256 x 6192 | 65 | GFX 50R, GFX 50S |
| V3 | C2 | 14 | 11808 x 8754 | 8, 7 | 11648 x 8736 | 62 to 64, 1024 | GFX 100, GFX100 II, GFX100RF, GFX100S, GFX100S II |
| V3 | C2 | 16 | 11808 x 8754 | 8, 7 | 11648 x 8736 | 255, 256 | GFX 100, GFX100RF, GFX100S, GFX100S II (16-bit mode) |

Pixel data were measured on the six local in-scope files (M4 to M13). The other rows rest on the container survey and on the same tag values.

### Out-of-scope relatives

| Id | Relative | Detection from file contents |
| --- | --- | --- |
| X1 | Fujifilm lossless compressed RAF | header BE32 at offset 108 = 2 (M2) |
| X2 | Fujifilm lossy compressed RAF | header BE32 at offset 108 = 3 (M2) |
| X3 | legacy container (SuperCCD and older FinePix bodies, GX680 back) | format version not "0201", or the CFA container does not start with 49 49 2A 00 (M1, M15) |
| X4 | two-image files (EXR sensors) | header BE32 at offset 128 is not 0 (M15) |
| X5 | other color filter layouts | neither rule of R-RAF-17 and R-RAF-19 applies (for example 0x0130 = 08 09 0B 0A) |
| X6 | any other structure: other header compression values, other bit depths or byte counts, big-endian CFA containers, missing required tags | no rule of R-RAF-3 matches |

No permitted source describes the Fujifilm compressed coding (X1, X2), the SuperCCD layouts (X3) or the EXR two-image layout (X4). They stay out of scope (Open questions OQ-1 to OQ-4).

### Requirements

- R-RAF-1: The decoder must decode the sensor data of variants V1, V2 and V3 with layouts C1 and C2 as this document specifies.
- R-RAF-2: The decoder must use the preview path (section Embedded preview) for X1, X2, X3, X4, X5 and X6 files and must not interpret their sensor data.
- R-RAF-3: The decoder must classify the file by these rules, in this order, and must treat a file that matches no rule as X6:
  1. Header bytes 16 to 19 are not "0201" (30 32 30 31): X3.
  2. The BE32 value at offset 108 is 2: X1. It is 3: X2. It is any other value than 0: X6.
  3. The CFA container is absent (R-RAF-6): X6.
  4. The CFA container does not start with 49 49 2A 00: X3.
  5. The BE32 value at offset 128 is not 0: X4.
  6. The FujiIFD is missing or invalid (R-RAF-10, R-RAF-13), or a tag that R-RAF-14 requires is absent: X6.
  7. The layout is neither C1 (R-RAF-17) nor C2 (R-RAF-19): X5.
  8. BitsPerSample 12 and 2 * StripByteCounts = 3 * W * H: V1.
  9. BitsPerSample 14 and 4 * StripByteCounts = 7 * W * H: V2.
  10. BitsPerSample 14 or 16 and StripByteCounts = 2 * W * H: V3.

"BitsPerSample" is the value of FujiIFD tag 0xF003 and "StripByteCounts" the value of tag 0xF008. The products use exact integer arithmetic (at least 64 bits).

## Container layout

### Header

All header fields are big-endian (S6, S7, M1). The header is 148 bytes long in all files of format version "0201".

| Offset | Size | Field | Value in RAF |
| --- | --- | --- | --- |
| 0 | 16 | magic | "FUJIFILMCCD-RAW " (46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20), S6 |
| 16 | 4 | format version, ASCII | "0201" (M1) |
| 20 | 8 | camera identifier, ASCII | for example "FF159501" (X-T3), not used |
| 28 | 32 | camera model, ASCII, zero padded | for example "X-T3", not used |
| 60 | 4 | version, ASCII (S5: FirmwareVersion) | for example "0101", not used |
| 64 | 20 | reserved | zero in all measured files |
| 84 | 4 | JPEG offset, BE32 | 148 |
| 88 | 4 | JPEG length, BE32 | length of the embedded JPEG |
| 92 | 4 | meta container offset, BE32 | |
| 96 | 4 | meta container length, BE32 | |
| 100 | 4 | CFA container offset, BE32 | |
| 104 | 4 | CFA container length, BE32 | |
| 108 | 4 | RAFCompression, BE32 (S5) | 0 uncompressed, 2 lossless, 3 lossy (M2) |
| 112 | 4 | unnamed, BE32 | 0 or the CFA container length, not used |
| 116 | 32 | second-image fields | zero in single-image files. Two-image files hold the second meta container offset at 120 and length at 124, and the second CFA container offset at 128 and length at 132 (M15) |

Measured order of the parts (M1): header, JPEG, zero padding, meta container, CFA container. The decoder does not rely on this order.

- R-RAF-4: The decoder must accept a file as RAF only if bytes 0 to 15 are the magic of the table above, and must report "not a RAF file" otherwise.
- R-RAF-5: The decoder must read the header fields at offsets 84 to 128 as BE32 values.
- R-RAF-6: The decoder must treat a part (JPEG, meta container, CFA container) as present only if its length is greater than 0 and offset + length is at most the file size, and must treat it as absent otherwise.

### Meta container

The meta container is big-endian (S6, S7, M3):

| Offset in container | Size | Field |
| --- | --- | --- |
| 0 | 4 | N, the number of records, BE32 |
| 4 | 2 | tag of record 1, BE16 |
| 6 | 2 | size S of record 1 in bytes, BE16 |
| 8 | S | data of record 1 |
| 8 + S | ... | record 2, and so on |

Records follow each other without padding. Multi-byte values inside the record data used by this document are BE16.

- R-RAF-7: The decoder must read the N records in order, must stop at the first record whose 4-byte head or data do not lie completely inside the meta container, and must keep the records read up to that point.
- R-RAF-8: The decoder must use the first record when a tag occurs more than once.

### CFA container

In all files in scope the CFA container starts with a little-endian TIFF header (S1 section 2): 49 49 2A 00 and a LE32 offset of IFD0, which is 8 (M3). IFD0 has one entry: tag 0xF000, type 13 (S5 calls the tag FujiIFD), count 1, and a value that is the offset of the FujiIFD. All offsets inside the CFA container, including FujiIFD value offsets and the sensor data offset 0xF007, count from the first byte of the CFA container (M3).

An IFD (S1 section 2) is:

| Offset in IFD | Size | Field |
| --- | --- | --- |
| 0 | 2 | N, the number of entries, LE16 |
| 2 + 12 * i | 2 | tag of entry i |
| 4 + 12 * i | 2 | field type of entry i |
| 6 + 12 * i | 4 | count of entry i (number of values, not bytes) |
| 10 + 12 * i | 4 | value or offset of entry i |
| 2 + 12 * N | 4 | offset of the next IFD (0 in all measured files) |

Field types used in RAF (S1 section 2): 3 SHORT (2 bytes), 4 LONG (4 bytes), 9 SLONG (4 bytes), 10 SRATIONAL (8 bytes). Type 13 (4 bytes) is not defined by S1. It holds an IFD offset in IFD0 of the CFA container. The value bytes of an entry occupy count * size bytes. If that total is 4 or less, the value is in the 4-byte value field, starting at its first byte. Otherwise the value field holds the offset of the value bytes.

- R-RAF-9: The decoder must read the CFA container as a TIFF structure only if its first 8 bytes are 49 49 2A 00 followed by a LE32 IFD0 offset, and must read every TIFF field of the CFA container as little-endian.
- R-RAF-10: The decoder must take the FujiIFD offset from the IFD0 entry with tag 0xF000, type 13 or 4, count 1, and must treat the file as X6 when IFD0 has no such entry.
- R-RAF-11: The decoder must read a value from the value field when count * size <= 4 and from the offset in the value field when count * size > 4, and must add the CFA container offset to every such offset.
- R-RAF-12: The decoder must find FujiIFD tags by tag number in any entry order, must accept types 3 and 4 for the integer tags of section Metadata tags, and must skip entries of other types for those tags.
- R-RAF-13: The decoder must treat an IFD as invalid when its offset lies outside the CFA container, when N is 0 or greater than 1024, or when its 2 + 12 * N + 4 bytes do not lie completely inside the CFA container.

## Metadata tags

### Meta container records

All values are BE16 unless the table says otherwise. Names are from S5 where S5 names the tag.

| Tag | Name (S5) | Size | Content | Use |
| --- | --- | --- | --- | --- |
| 0x0100 | RawImageFullSize | 4 | H, W of the raster ("including borders", S5). Equal to FujiIFD H and W (M3) | check (R-RAF-15) |
| 0x0110 | RawImageCropTopLeft | 4 | top, left of the crop rectangle ("top margin first, then left margin", S5) | crop (R-RAF-16) |
| 0x0111 | RawImageCroppedSize | 4 | height, width of the crop rectangle (measured order, M13) | crop (R-RAF-16) |
| 0x0130 | FujiLayout | 4 | 4 bytes. 0C 0C 0C 0C with X-Trans, 0A 0B 09 08 with Bayer RGGB (M9). Other values occur only in X3, X4 and X5 files (M15). The byte meanings are not documented | layout (R-RAF-19) |
| 0x0131 | XTransLayout | 36 | 36 bytes, values 0 (red), 1 (green), 2 (blue), in reverse raster order (M7) | layout (R-RAF-17, R-RAF-18) |

Records observed and not used: 0x0112 and 0x0113 (crop width and height in two orders), 0x0115 (S5: RawImageAspectRatio, for example 2 3), 0x0141 (for example 0E 2A), 0x9650 (S5: RawExposureBias, a signed numerator and a denominator, for example -72/100), 0x9651, 0xC000 (S5: RAFData), and in legacy files 0x2FF0 (S5: WB_GRGBLevels) and many others (Open questions OQ-11, OQ-17).

### FujiIFD tags

All tags are in the FujiIFD. "Required" means the file cannot be decoded without the tag. The file is then X6. Names are from S5 except where marked.

| Tag | Name (S5) | Type | Count | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- |
| 0xF001 | RawImageFullWidth | LONG | 1 | raster width W | required | none |
| 0xF002 | RawImageFullHeight | LONG | 1 | raster height H | required | none |
| 0xF003 | BitsPerSample | LONG | 1 | 12, 14 or 16 | required | none |
| 0xF004 | (no public name) | LONG | 1 | 4 in V1 and V2, 0 in V3 | ignored | none |
| 0xF005 | (no public name) | LONG | 1 | row size in bytes / 4 in V1 and V2, 0 in V3 | ignored | none |
| 0xF007 | StripOffsets | LONG | 1 | offset of the sensor data from the start of the CFA container | required | none |
| 0xF008 | StripByteCounts | LONG | 1 | size of the sensor data in bytes | required | none |
| 0xF00A | BlackLevel | LONG | 4 (C2) or 36 (C1) | black levels, see R-RAF-29 | required | none |
| 0xF00B | GeometricDistortionParams | SRATIONAL | 19 or 23 | lens distortion data | ignored (OQ-9) | none |
| 0xF00C | WB_GRBLevelsStandard | LONG | 8 | white balance presets | ignored | none |
| 0xF00D | WB_GRBLevelsAuto | LONG | 3 | G, R, B levels of auto white balance | optional | R-RAF-33 |
| 0xF00E | WB_GRBLevels | LONG | 3 | G, R, B levels of the as-shot white balance (M11) | optional | R-RAF-33 |
| 0xF00F | ChromaticAberrationParams | SRATIONAL | 29 or 31 | lens data | ignored (OQ-9) | none |
| 0xF010 | VignettingParams | SRATIONAL | 19 or 23 | lens data | ignored (OQ-9) | none |

Tags observed and not used: 0xF006 (0), 0xF009 (0, 3, 136 or 142), 0xF011 and 0xF012 (SLONG 0), 0xF013 and 0xF014 (copies of 0xF00E in X-T4) (OQ-11).

### Exif tags of the embedded JPEG

| Tag | IFD | Meaning | Use |
| --- | --- | --- | --- |
| 0x0112 Orientation | IFD0 of the Exif APP1 segment (S4) | 1 to 8, S1 definition | orientation (R-RAF-43) |
| 0x0201, 0x0202 | IFD1 of the Exif APP1 segment (S4) | offset (from the TIFF header of the APP1 segment) and length of the thumbnail JPEG | preview candidate (R-RAF-47) |

### Requirements

- R-RAF-14: The decoder must require FujiIFD tags 0xF001, 0xF002, 0xF003, 0xF007, 0xF008 and 0xF00A with a count of at least 1, and must treat the file as X6 when one is absent.
- R-RAF-15: The decoder must treat the file as X6 when meta record 0x0100 is present with 4 or more bytes and its two BE16 values differ from H and W.

## Sensor data layout

### Raster and crop rectangle

The raster has W columns and H rows (0xF001, 0xF002). The crop rectangle has origin (CX, CY) = (left, top) from meta record 0x0110 and size (CW, CH) = (width, height) from meta record 0x0111. It matches the framing of the camera JPEG (M13). Outside the crop rectangle the raster can contain bands of zero values (M13). These bands never touch the crop rectangle in measured files: the 1-pixel ring around the crop rectangle is image data.

- R-RAF-16: The decoder must use the crop rectangle when records 0x0110 and 0x0111 are both present with at least 4 bytes, CW >= 16, CH >= 16, CX + CW <= W and CY + CH <= H, and must otherwise use the whole raster.

### X-Trans arrangement (C1)

S8 describes a 6x6 basic array pattern with green filters in every horizontal, vertical and diagonal line and red and blue filters in every horizontal and vertical line, and with 20 green, 8 red and 8 blue filters. Its rows are GBGGRG, RGRBGB, GBGGRG, GRGGBG, BGBRGR, GRGGBG. Cameras store the phase of this pattern relative to the raster in meta record 0x0131 (M7). The 36 bytes are in reverse raster order: byte 35 is the color of (0, 0), byte 34 the color of (1, 0), byte 30 the color of (5, 0), byte 29 the color of (0, 1), and byte 0 the color of (5, 5). The pattern origin is stored pixel (0, 0), not the crop origin (M7).

Example (S16, X-T3): bytes 01 00 02 01 02 00 02 01 01 00 01 01 00 01 01 02 01 01 01 02 00 01 00 02 00 01 01 02 01 01 02 01 01 00 01 01 give the pattern of TV-4.

- R-RAF-17: The decoder must accept layout C1 only if meta record 0x0131 is present with exactly 36 bytes, every byte is 0, 1 or 2, and the bytes contain exactly 8 values 0, 20 values 1 and 8 values 2.
- R-RAF-18: For C1 the decoder must give pixel (x, y) the color of byte 35 - (6 * (y mod 6) + (x mod 6)) of record 0x0131, with 0 = red, 1 = green, 2 = blue (M7, M8).

### Bayer arrangement (C2)

Files with meta record 0x0130 = 0A 0B 09 08 and no record 0x0131 use a 2x2 Bayer pattern with red at (even x, even y), green at (odd x, even y) and (even x, odd y), and blue at (odd x, odd y) of the raster (M9). The meaning of the four bytes of 0x0130 is not documented (OQ-5). This document only accepts the measured value.

- R-RAF-19: The decoder must accept layout C2 only if record 0x0131 is absent and record 0x0130 is present with the 4 bytes 0A 0B 09 08, and must then give pixel (x, y) the color red if x and y are even, blue if x and y are odd, and green otherwise.
- R-RAF-20: The decoder must place the origin of both patterns at stored pixel (0, 0), independent of the crop origin.

### Strip

All files in scope have one strip (M3). Row y starts at byte offset D + y * R, where D is the CFA container offset plus the value of FujiIFD tag 0xF007, and R is the row size in bytes: R = 3 * W / 2 (V1), R = 7 * W / 4 (V2), R = 2 * W (V3). There is no padding between rows (M3 to M6).

- R-RAF-21: The decoder must read row y of the raster at file offset D + y * R with D and R as defined above.
- R-RAF-22: The decoder must require that the strip (D, 0xF008 bytes) lies completely inside the CFA container and inside the file, and must treat the file as X6 otherwise.

## Compression

All variants in scope are uncompressed. Each subsection gives the bit layout that turns the strip bytes into one sensor value per raster pixel. The values are integers from 0 to 2^b - 1, where b is BitsPerSample. Files with RAFCompression 2 or 3 (X1, X2) use a coding that no permitted source describes (OQ-1, OQ-2). The decoder never reads their strip.

### V1: 12-bit packed

Each row holds 3 * W / 2 bytes. Pixels 2k and 2k + 1 of a row come from the bytes b0, b1, b2 at row offset 3k. The 24-bit little-endian group G = b0 + 256 * b1 + 65536 * b2 holds pixel 2k in bits 0 to 11 and pixel 2k + 1 in bits 12 to 23 (M4). The values are on the 12-bit scale and are not scaled. This is the bit layout of variant V2 of `arw.md` without its multiplication by 4.

- R-RAF-23: The decoder must compute pixel 2k of a V1 row as b0 + 256 * (b1 mod 16) and pixel 2k + 1 as floor(b1 / 16) + 16 * b2.
- R-RAF-24: The decoder must treat a V1 file with an odd W as X6.

Worked example (S13, X-Pro1, row 1500, columns 2400 to 2405, file offset 12418576): bytes 1D D6 61 AD 13 60 26 A6 43.
- Group 1: b0 = 0x1D = 29, b1 = 0xD6 = 214, b2 = 0x61 = 97. Pixel 2400 = 29 + 256 * 6 = 1565. Pixel 2401 = 13 + 16 * 97 = 1565.
- Group 2: AD 13 60. Pixel 2402 = 173 + 256 * 3 = 941. Pixel 2403 = 1 + 16 * 96 = 1537.
- Group 3: 26 A6 43. Pixel 2404 = 38 + 256 * 6 = 1574. Pixel 2405 = 10 + 16 * 67 = 1082.
- Colors (R-RAF-18 with the X-Pro1 record, 1500 mod 6 = 0): G, G, R, G, G, B.

### V2: 14-bit packed

Each row holds 7 * W / 4 bytes. Read the row as consecutive groups of 4 bytes. Each group is one LE32 word w = c0 + 256 * c1 + 65536 * c2 + 16777216 * c3. Write the words of the row one after another, each from its bit 31 down to its bit 0. This gives a bit sequence of 14 * W bits. Pixel x of the row is the 14-bit value formed by bits 14x to 14x + 13 of that sequence, the first of them being the most significant bit (M5). Seven words hold 16 pixels.

- R-RAF-25: The decoder must decode a V2 row by the bit sequence above: pixel x is the sum over j = 0 to 13 of 2^(13 - j) * s(14x + j), where s(n) is bit 31 - (n mod 32) of word floor(n / 32) of the row.
- R-RAF-26: The decoder must treat a V2 file with W mod 16 not equal to 0 as X6, so that every decoded row is a whole number of words.

Worked example (S14, X-A5, row 2000, columns 3008 to 3010, file offset 21952656): the first 8 bytes are 92 2C 6D 2B 2D B9 D2 BD.
- Word 0 = 0x2B6D2C92 = binary 0010 1011 0110 1101 0010 1100 1001 0010. Word 1 = 0xBDD2B92D = binary 1011 1101 1101 0010 1011 1001 0010 1101.
- Pixel 3008 = word 0 bits 31 to 18 = 00 1010 1101 1011 = 0x0ADB = 2779.
- Pixel 3009 = word 0 bits 17 to 4 = 01 0010 1100 1001 = 0x12C9 = 4809.
- Pixel 3010 = word 0 bits 3 to 0 (0010) followed by word 1 bits 31 to 22 (1011 1101 11) = 00 1010 1111 0111 = 0x0AF7 = 2807.
- All 16 values of the 28 bytes are in TV-7.

### V3: 16-bit containers

Each pixel is one LE16 value (M6). With BitsPerSample 14 the values are at most 16383. With BitsPerSample 16 they use the full 16-bit range.

- R-RAF-27: The decoder must read V3 pixel (x, y) as b0 + 256 * b1, where b0 and b1 are the bytes at D + y * R + 2 * x and D + y * R + 2 * x + 1.

Worked example (S16, X-T3, row 2000, columns 3000 to 3005, file offset 28233584): bytes 2B 06 A7 06 E5 08 5F 06 AE 05 F0 07 give 0x062B = 1579, 0x06A7 = 1703, 0x08E5 = 2277, 0x065F = 1631, 0x05AE = 1454, 0x07F0 = 2032. Colors (R-RAF-18, 2000 mod 6 = 2, 3000 mod 6 = 0): B, R, G, R, B, G.

## Levels and white balance

The decoder determines the white level first and the black level second.

### White level

- R-RAF-28: The decoder must use w = 2^b - 1 as the white level, where b is BitsPerSample (4095, 16383 or 65535).

The measured saturation plateaus are 4094 (X-Pro1, 12-bit), 16322 (X-T1, 14-bit, green only) and 16381 (X-A5, 14-bit) (M10). A plateau below w gives clipped highlights a level slightly below 1 (for example 0.996 for X-T1 green) and a faint tint (OQ-7).

### Black level

FujiIFD 0xF00A holds 4 values in C2 files and 36 values in C1 files (M3). In every measured file the values are equal or differ by 1 (M10, S20). The order of the values relative to the pattern positions is not established (OQ-6). The decoder therefore uses one black level for all pixels.

- R-RAF-29: The decoder must use as black level b the arithmetic mean of all values of tag 0xF00A, as a real number.
- R-RAF-30: The decoder must treat the file as X6 when b is not less than w - 255.
- R-RAF-31: The decoder must use the same black level b for every pixel of the raster.

Worked example: X-T3 has 36 values 1022, so b = 1022. X100 has 254, 255, 255, 254, so b = 254.5.

### White balance multipliers

The decoder computes three multipliers mR, mG, mB. Sources in priority order:
1. Tag 0xF00E WB_GRBLevels (G, R, B) with all three values greater than 0: mR = R / G, mG = 1, mB = B / G (M11).
2. Tag 0xF00D WB_GRBLevelsAuto (G, R, B) with all three values greater than 0, by the same formulas.
3. The generic multipliers mR = 1.830, mG = 1, mB = 1.849, the means of the 0xF00E ratios of the 61 in-scope survey files (S20).

- R-RAF-32: The decoder must read 0xF00E and 0xF00D as three values in the order G, R, B.
- R-RAF-33: The decoder must take the WB multipliers from the first available source of the list above.

Worked example (S16, X-T3): 0xF00E = 302, 583, 584 gives mR = 583 / 302 = 1.9304635762 and mB = 584 / 302 = 1.9337748344.

### Normalization

For a pixel with sensor value s, black level b, white level w and the multiplier m of its color:
- n = (s - b) / (w - b)
- v = n * m, clipped to the range 0 to 1.

This is R-ARW-47 of `arw.md` with one black level for all positions.

- R-RAF-34: The decoder must compute v = clip((s - b) / (w - b) * m, 0, 1) for every pixel of the raster before demosaicing.

Worked example (X-T3 values of the V3 example, b = 1022, w = 16383, w - b = 15361):
- B 1579: n = 557 / 15361 = 0.03626066, v = 0.03626066 * 1.9337748 = 0.07011995.
- R 1703: n = 681 / 15361 = 0.04433305, v = 0.04433305 * 1.9304636 = 0.08558334.
- G 2277: n = 1255 / 15361 = 0.08170041, v = 0.08170041.

## Color conversion

### Matrix source

RAF files carry no color matrix. Neither S5 nor the measured meta containers and FujiIFDs of in-scope files contain a matrix-like tag (M3, S20). DXOMARK, the source of the matrices of `arw.md`, shows an all-zero matrix on the page of the FinePix X100, the one in-scope model that the author found there (section Sources encountered and not used). The decoder therefore uses one fixed matrix for all models. It is derived from the Adobe ColorMatrix2 of the public X100S DNG sample (S11, M12):

1. ColorMatrix2 (CM) maps CIE XYZ to camera RGB for illuminant 21, which is D65 (S3): CM = 1.0592 -0.4262 -0.1008 / -0.3514 1.1355 0.2465 / -0.0870 0.2025 0.6386.
2. The matrix P that maps linear sRGB to XYZ follows from the primaries and white point of S10 (red 0.64, 0.33, green 0.30, 0.60, blue 0.15, 0.06, white 0.3127, 0.3290): P = 0.4123908 0.3575843 0.1804808 / 0.2126390 0.7151687 0.0721923 / 0.0193308 0.1191948 0.9505322.
3. A = CM * P maps linear sRGB to camera RGB. A neutral sRGB value maps to the camera neutral, so dividing each row of A by its row sum gives the matrix that maps sRGB to white-balanced camera RGB.
4. The inverse of that row-normalized matrix is the matrix below. It maps white-balanced camera RGB to linear sRGB, and each of its rows sums to 1.

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 1.4008393 | -0.2085722 | -0.1922671 |
| sRGB G | -0.2049913 | 1.7113128 | -0.5063215 |
| sRGB B | 0.0190791 | -0.5294124 | 1.5103333 |

Each row is one output channel and each column one camera channel (M12: the transposed matrix is worse in every measured file). The matrix was checked against the previews of six cameras: X100, X-Pro1, X-A5, X-T1, X-T3 and GFX 50S (M12). Per-model matrices are an open question (OQ-8).

### Procedure

- R-RAF-35: The decoder must use the matrix of the table above for every model.
- R-RAF-36: The decoder must divide each row of the matrix by the sum of that row before use, so that every row sums to 1.
- R-RAF-37: The decoder must compute the output (r, g, b) from the demosaiced values (v_R, v_G, v_B) as r = m11 * v_R + m12 * v_G + m13 * v_B, g = m21 * v_R + m22 * v_G + m23 * v_B, b = m31 * v_R + m32 * v_G + m33 * v_B, and must clip r, g and b to the range 0 to 1.
- R-RAF-38: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S10), without the sRGB transfer function.

Worked example: the values of the normalization example (v_R, v_G, v_B) = (0.08558334, 0.08170041, 0.07011995):
- r = 1.4008393 * 0.08558334 - 0.2085722 * 0.08170041 - 0.1922671 * 0.07011995 = 0.11988851 - 0.01704043 - 0.01348176 = 0.08936631.
- g = -0.2049913 * 0.08558334 + 1.7113128 * 0.08170041 - 0.5063215 * 0.07011995 = -0.01754384 + 0.13981496 - 0.03550324 = 0.08676788.
- b = 0.0190791 * 0.08558334 - 0.5294124 * 0.08170041 + 1.5103333 * 0.07011995 = 0.00163285 - 0.04325321 + 0.10590450 = 0.06428414.

A neutral value (0.25, 0.25, 0.25) stays (0.25, 0.25, 0.25).

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and the magic (R-RAF-54, R-RAF-4). Read the header (R-RAF-5, R-RAF-6).
2. Collect the preview candidates and the orientation now, so that every later failure can fall back (R-RAF-43, R-RAF-44, R-RAF-47, R-RAF-48).
3. Read the meta container (R-RAF-7, R-RAF-8).
4. Read the CFA container and the FujiIFD (R-RAF-9 to R-RAF-13, R-RAF-14, R-RAF-15).
5. Classify the file (R-RAF-3, R-RAF-17, R-RAF-19). For X1 to X6 use the preview path (R-RAF-2).
6. Determine the crop rectangle (R-RAF-16), the color pattern (R-RAF-18 or R-RAF-19, R-RAF-20), the white level (R-RAF-28), the black level (R-RAF-29 to R-RAF-31) and the WB multipliers (R-RAF-32, R-RAF-33).
7. Check all sizes against the file and the limits before allocating memory (section Error handling, R-RAF-22).
8. Decode the sensor data (R-RAF-21):
   - V1: R-RAF-23, R-RAF-24.
   - V2: R-RAF-25, R-RAF-26.
   - V3: R-RAF-27.
9. Normalize every raster pixel (R-RAF-34).
10. Demosaic the crop rectangle (R-RAF-39 to R-RAF-42).
11. Convert to linear sRGB (R-RAF-35 to R-RAF-37).
12. Apply the orientation (R-RAF-45, R-RAF-46).
13. Deliver the image (R-RAF-38). If any step from 3 to 12 fails, deliver the preview instead (R-RAF-52).

### Demosaicing

The decoder uses bilinear interpolation, the non-adaptive method that S9 describes as satisfactory in smooth regions. For each pixel and each color that the pixel lacks, the missing value is the mean of the pixels of that color among the 8 neighbors of the pixel. For the Bayer layout this is the method of R-ARW-54 of `arw.md`. For the X-Trans layout the same rule works without change, because every 8-neighborhood of the pattern contains both missing colors (M16): a red pixel takes green from 5 neighbors and blue from 3, a blue pixel takes red from 3 and green from 5, and a green pixel takes red from 2 and blue from 2. At the raster border some neighbors are missing. If no neighbor of a missing color lies inside the raster, the decoder uses the 5 x 5 neighborhood. Every 3 x 3 window of both patterns contains all three colors (M16), so the 5 x 5 rule always finds a value at a corner or an edge. The comparison of M8, M9, M11 and M12 used this method.

- R-RAF-39: For each pixel (x, y) of the crop rectangle, the decoder must set its own color to its normalized value v, and each missing color to the arithmetic mean of the normalized values of the pixels of that color among (x + dx, y + dy) with dx and dy from -1 to 1, not both 0, that lie inside the raster.
- R-RAF-40: When no pixel of a missing color lies in that 3 x 3 neighborhood inside the raster, the decoder must use the mean over (x + dx, y + dy) with dx and dy from -2 to 2, not both 0, inside the raster, and must use 0 when that set is also empty.
- R-RAF-41: The decoder must use neighbors outside the crop rectangle when they lie inside the raster, so that the result equals demosaicing the whole raster and cropping afterwards.
- R-RAF-42: The decoder must demosaic with the color pattern of the stored raster (R-RAF-18, R-RAF-19, R-RAF-20).

Other demosaicing methods are outside this document. The test vectors assume this one.

### Orientation

RAF files have no orientation field in the header or the meta container. The orientation is the Exif Orientation tag (S4) in the APP1 segment of the embedded JPEG (M14). The Exif segment starts with the 6 bytes "Exif" 00 00, followed by a TIFF header (S1 section 2, byte order 49 49 or 4D 4D) whose offsets count from the first byte of that TIFF header.

- R-RAF-43: The decoder must read the orientation from tag 0x0112 of IFD0 of the first APP1 segment whose data start with 45 78 69 66 00 00, among the marker segments of the embedded JPEG before its first SOS marker, and must use 1 when the tag, the segment or the JPEG is absent, or the value is outside 1 to 8.
- R-RAF-44: The decoder must read the orientation even when the embedded JPEG cannot be decoded as an image, and must keep every Exif offset inside the APP1 segment.
- R-RAF-45: The decoder must apply the orientation after the crop and the color conversion, mapping stored pixel (x, y) of a W' x H' image to the output pixel of the table below.
- R-RAF-46: The decoder must deliver W' x H' pixels for orientations 1 to 4 and H' x W' pixels for orientations 5 to 8.

| Orientation | Output pixel | Output size |
| --- | --- | --- |
| 1 | (x, y) | W' x H' |
| 2 | (W' - 1 - x, y) | W' x H' |
| 3 | (W' - 1 - x, H' - 1 - y) | W' x H' |
| 4 | (x, H' - 1 - y) | W' x H' |
| 5 | (y, x) | H' x W' |
| 6 | (H' - 1 - y, x) | H' x W' |
| 7 | (H' - 1 - y, W' - 1 - x) | H' x W' |
| 8 | (y, W' - 1 - x) | H' x W' |

These mappings follow the definitions of S1 and are the table of R-ARW-58 of `arw.md`. For example, S1 defines value 6 as "the 0th row represents the visual right-hand side of the image, and the 0th column represents the visual top". Stored row 0 therefore goes to the rightmost output column.

## Embedded preview

### Locations

| Location | How to find it | Content (M14) |
| --- | --- | --- |
| main JPEG | header offset 84 (offset) and 88 (length) | baseline JPEG, 1440 x 960 to 4416 x 2944 in measured files, with an Exif APP1 segment |
| Exif thumbnail | tags 0x0201 (offset) and 0x0202 (length) of IFD1 in the Exif APP1 segment of the main JPEG, offsets from the first byte of its TIFF header | 160 x 120 baseline JPEG |

The main JPEG is the largest preview in every measured file. It exists in every RAF file, including the out-of-scope variants X1 to X4 (M14), so it is the fallback for every file that this document does not decode. Measured previews are baseline JPEG (SOF0, 8-bit, 3 components) and are decoded with S2 (baseline sequential DCT process).

### Requirements

- R-RAF-47: The decoder must collect as preview candidates the main JPEG of the header and the IFD1 thumbnail of its Exif segment.
- R-RAF-48: The decoder must keep a candidate only if it lies completely inside the file (the thumbnail also inside the APP1 segment), its length is at least 4, and its first two bytes are FF D8.
- R-RAF-49: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment and must choose the candidate with the largest width * height.
- R-RAF-50: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan.
- R-RAF-51: The decoder must apply the orientation of R-RAF-43 to the preview by the table of R-RAF-45, except that it must not apply values 5 to 8 when the preview is taller than wide.
- R-RAF-52: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 to X6, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-RAF-53: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-RAF-38) or a preview (sRGB-encoded JPEG values, not linear).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-RAF-52).

### Limits

- R-RAF-54: The decoder must reject a file shorter than 148 bytes and must fail for a file longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-RAF-55: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728).
- R-RAF-56: The decoder must fail when the meta container record count N is greater than 4096, and must never allocate memory in proportion to N or to a record size before checking that the bytes lie inside the meta container.
- R-RAF-57: The decoder must fail when count * type size of a FujiIFD tag that it reads exceeds the CFA container length, before it allocates memory for the values.
- R-RAF-58: The decoder must check, before reading, that every header part, record, IFD, value and strip byte range lies completely inside the file.

### Structure checks

- R-RAF-59: The decoder must classify by the exact byte counts of R-RAF-3 and must never read more than StripByteCounts bytes of the strip.
- R-RAF-60: The decoder must fail when the strip offset 0xF007 plus StripByteCounts exceeds the CFA container length.
- R-RAF-61: The decoder must fail when BitsPerSample is not 12, 14 or 16, or does not match the variant of R-RAF-3.
- R-RAF-62: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check selects the default of the quantity or fails.

### Behaviour on error

- R-RAF-63: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-RAF-64: The decoder must discard a partly decoded raw image on failure and must not deliver it.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order.

### TV-1: header

S16 (X-T3), bytes 0x54 to 0x73: 0x00 0x00 0x00 0x94 0x00 0x28 0xB2 0xDD 0x00 0x28 0xB3 0xA8 0x00 0x00 0x56 0x58 0x00 0x29 0x0A 0x00 0x03 0x2E 0xC9 0x40 0x00 0x00 0x00 0x00 0x03 0x2E 0xC9 0x40.
- JPEG offset 148, JPEG length 2667229, meta container offset 2667432, length 22104, CFA container offset 2689536, length 53397824, RAFCompression 0. Class V3 with C1.
- S17: DSCF0267 has 0x00 0x00 0x00 0x02 at offset 108 (X1). DSCF0268 has 0x00 0x00 0x00 0x03 (X2).
- S19: X10 has 0x00 0x9D 0x25 0x10 at offset 128 (second CFA container at 10298640, X4). S5Pro has 0x00 0x20 0x00 0x20 at its CFA container offset 479584 (X3).

### TV-2: meta container records

S16, from offset 2667432: 0x00 0x00 0x00 0x0B (N = 11), then:
- 01 00 00 04 10 56 18 F0: tag 0x0100, size 4, H = 0x1056 = 4182, W = 0x18F0 = 6384.
- 01 10 00 04 00 0D 00 06: tag 0x0110, top 13, left 6.
- 01 11 00 04 10 40 18 60: tag 0x0111, height 4160, width 6240. Crop rectangle: x 6 to 6245, y 13 to 4172.
- 01 30 00 04 0C 0C 0C 0C: tag 0x0130.
- 01 31 00 24 followed by the 36 bytes of TV-4: tag 0x0131, size 36.

### TV-3: CFA container and FujiIFD

S16, from offset 2689536: 49 49 2A 00 08 00 00 00, then IFD0: 01 00, entry 00 F0 0D 00 01 00 00 00 1A 00 00 00, next 00 00 00 00. FujiIFD at container offset 26 with 0x12 = 18 entries, among them:
- 01 F0 04 00 01 00 00 00 F0 18 00 00: 0xF001 = 6384.
- 02 F0 04 00 01 00 00 00 56 10 00 00: 0xF002 = 4182.
- 03 F0 04 00 01 00 00 00 0E 00 00 00: 0xF003 = 14.
- 07 F0 04 00 01 00 00 00 00 08 00 00: 0xF007 = 2048, so the strip starts at 2689536 + 2048 = 2691584.
- 08 F0 04 00 01 00 00 00 40 C1 2E 03: 0xF008 = 53395776 = 2 * 6384 * 4182, so V3.
- 0A F0 04 00 24 00 00 00 F8 00 00 00: 0xF00A, 36 LONG values at container offset 248, each FE 03 00 00 = 1022.
- 0E F0 04 00 03 00 00 00 4C 02 00 00: 0xF00E, 3 LONG values at container offset 588: 2E 01 00 00 47 02 00 00 48 02 00 00 = 302, 583, 584.

### TV-4: X-Trans layout

X-T3 (S16) record 0x0131: 0x01 0x00 0x02 0x01 0x02 0x00 0x02 0x01 0x01 0x00 0x01 0x01 0x00 0x01 0x01 0x02 0x01 0x01 0x01 0x02 0x00 0x01 0x00 0x02 0x00 0x01 0x01 0x02 0x01 0x01 0x02 0x01 0x01 0x00 0x01 0x01. Pattern by R-RAF-18 (rows y = 0 to 5, columns x = 0 to 5):

| y | x=0 | x=1 | x=2 | x=3 | x=4 | x=5 |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | G | G | R | G | G | B |
| 1 | G | G | B | G | G | R |
| 2 | B | R | G | R | B | G |
| 3 | G | G | B | G | G | R |
| 4 | G | G | R | G | G | B |
| 5 | R | B | G | B | R | G |

X-T1 (S15) record 0x0131: 0x02 0x01 0x01 0x00 0x01 0x01 0x00 0x01 0x01 0x02 0x01 0x01 0x01 0x02 0x00 0x01 0x00 0x02 0x00 0x01 0x01 0x02 0x01 0x01 0x02 0x01 0x01 0x00 0x01 0x01 0x01 0x00 0x02 0x01 0x02 0x00. Pattern rows: RBGBRG, GGRGGB, GGBGGR, BRGRBG, GGBGGR, GGRGGB.

A decoder that reads the bytes in forward order gets row 0 of X-T3 as G R B G B R and fails this vector.

### TV-5: V3 samples

0x2B 0x06 0xA7 0x06 0xE5 0x08 0x5F 0x06 0xAE 0x05 0xF0 0x07 -> 1579, 1703, 2277, 1631, 1454, 2032 (S16, row 2000, columns 3000 to 3005, colors B, R, G, R, B, G).

### TV-6: V1 samples

- 0x1D 0xD6 0x61 0xAD 0x13 0x60 0x26 0xA6 0x43 -> 1565, 1565, 941, 1537, 1574, 1082 (S13, row 1500, columns 2400 to 2405).
- 0xA7 0x21 0x19 0xBF 0xD1 0x19 -> 423, 402, 447, 413 (S12, X100, row 1000, columns 2000 to 2003, file offset 7466824, colors R, G, R, G).

### TV-7: V2 samples

0x92 0x2C 0x6D 0x2B 0x2D 0xB9 0xD2 0xBD 0xB8 0x22 0x30 0x41 0x75 0x2A 0xB5 0x51 0x11 0x9A 0xF2 0x17 0x1D 0x69 0x29 0x4D 0x63 0xD1 0xAE 0xD2 (7 words) -> 2779, 4809, 2807, 4793, 2896, 4866, 2785, 4533, 2717, 4479, 2664, 4429, 2650, 4573, 2747, 4451 (S14, row 2000, columns 3008 to 3023, colors R, G, R, G and so on).

### TV-8: levels and white balance

Black 1022, white 16383, 0xF00E = 302 583 584. Sensor values B 1579, R 1703, G 2277, R 1631, B 1454, G 2032 give v = 0.07011995, 0.08558334, 0.08170041, 0.07653488, 0.05438388, 0.06575093.

### TV-9: X-Trans demosaicing

Pattern of TV-4 on a 6 x 6 raster (the whole raster is the crop rectangle). Normalized values (any linear values work, the rule is linear):

| y | x=0 | x=1 | x=2 | x=3 | x=4 | x=5 |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 0 | 7 | 14 | 21 | 5 | 12 |
| 1 | 11 | 21 | 8 | 18 | 5 | 15 |
| 2 | 22 | 12 | 2 | 15 | 5 | 18 |
| 3 | 10 | 3 | 19 | 12 | 5 | 21 |
| 4 | 21 | 17 | 13 | 9 | 5 | 1 |
| 5 | 9 | 8 | 7 | 6 | 5 | 4 |

Expected (R, G, B):
- (1, 2), red: R = 12. G = mean of (0, 1), (1, 1), (2, 2), (0, 3), (1, 3) = (11 + 21 + 2 + 10 + 3) / 5 = 9.4. B = mean of (2, 1), (0, 2), (2, 3) = (8 + 22 + 19) / 3 = 16.333333.
- (2, 2), green: R = mean of (1, 2), (3, 2) = 13.5. G = 2. B = mean of (2, 1), (2, 3) = 13.5.
- (0, 2), blue: R = (1, 2) alone = 12. G = mean of (0, 1), (1, 1), (0, 3), (1, 3) = 11.25. B = 22.
- (3, 3), green: R = mean of (3, 2), (2, 4) = 14. G = 12. B = mean of (4, 2), (2, 3) = 12.
- (0, 0), green, corner: the 3 x 3 neighbors (1, 0), (0, 1), (1, 1) are all green, so R and B use the 5 x 5 rule (R-RAF-40). R = mean of (2, 0), (1, 2) = 13. G = 0. B = mean of (2, 1), (0, 2) = 15.

### TV-10: Bayer demosaicing

C2 pattern on a 4 x 4 raster with values (rows y = 0 to 3): 0 5 10 15 / 9 16 4 11 / 18 8 17 7 / 8 0 11 3. Expected (R, G, B):
- (1, 1), blue: R = mean of (0, 0), (2, 0), (0, 2), (2, 2) = 11.25. G = mean of (1, 0), (0, 1), (2, 1), (1, 2) = 6.5. B = 16.
- (2, 1), green: R = mean of (2, 0), (2, 2) = 13.5. G = 4. B = mean of (1, 1), (3, 1) = 13.5.
- (1, 2), green: R = mean of (0, 2), (2, 2) = 17.5. G = 8. B = mean of (1, 1), (1, 3) = 8.
- (0, 0), red, corner: R = 0. G = mean of (1, 0), (0, 1) = 7. B = (1, 1) alone = 16.

### TV-11: color matrix

(v_R, v_G, v_B) = (0.08558334, 0.08170041, 0.07011995) -> (r, g, b) = (0.08936631, 0.08676788, 0.06428414). (0.25, 0.25, 0.25) -> (0.25, 0.25, 0.25).

### TV-12: orientation

Stored 3 x 2 image with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a.

### TV-13: classification of the sample files

| File (SOURCES.txt) | Expected class |
| --- | --- |
| FinePix_X100_DSCF1676.RAF | V1, C2 |
| X-Pro1_DSCF2131.RAF | V1, C1 |
| X-A5_DSCF0617.RAF | V2, C2 |
| X-T1_20171229_110916.RAF | V3, C1 |
| X-T3_AFXT2720.RAF | V3, C1 |
| GFX_50S_20170525_0036TEST.RAF | V3, C2 |
| X-T4_DSCF0267.RAF | X1 |
| X-T4_DSCF0268.RAF | X2 |
| FinePix_S5Pro_2018_06150003.RAF | X3 |
| FinePix_X10_DSCF0514.RAF | X4 |

For the six in-scope files the decoded and cropped image has the size of the crop rectangle (for example 6240 x 4160 for X-T3) and agrees with the embedded JPEG within the chromaticity errors of M12 (at most 0.08) when both are compared by the method of M12.

### Fixture recipes

The fixtures are complete small RAF files for the decoder tests. They share one layout and differ in the meta records, the FujiIFD values and the data. Multi-byte numbers are big-endian in the header and the meta container and little-endian in the CFA container.

Common part:
- Offset 0x00: the header. Magic, "0201", camera identifier "FF000001", model "FIXTURE", version "0100". JPEG offset 0x94, JPEG length 0x28, meta container at 0xBC, RAFCompression 0, all other bytes 0.
- Offset 0x94: a 40-byte JPEG stub that holds only an Exif segment: FF D8, FF E1 00 22, 45 78 69 66 00 00, TIFF header 49 49 2A 00 08 00 00 00, IFD0 with 1 entry 12 01 03 00 01 00 00 00 OO 00 00 00 (Orientation SHORT, OO = orientation), next-IFD offset 00 00 00 00, FF D9. The stub has no frame, so it is not a decodable preview (R-RAF-44 still reads its orientation).
- Offset 0xBC: the meta container with records 0x0100, 0x0110, 0x0111, 0x0130 and, for C1, 0x0131.
- Next multiple of 16: the CFA container. IFD0 with the entry 0xF000 type 13 value 26. The FujiIFD at 26 with 7 entries: 0xF001, 0xF002, 0xF003, 0xF007, 0xF008, 0xF00A, 0xF00E (all LONG). The 0xF00A values, then the 0xF00E values, then the strip at the next multiple of 16 inside the container.

F1 (V3, C1, 8 x 8 raster, crop 6 x 6 at (1, 1), X-T3 layout, black 36 x 1022, white balance 302 583 584, flat field R 3022, G 5022, B 2022), complete file of 672 bytes:

0000: 46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20
0010: 30 32 30 31 46 46 30 30 30 30 30 31 46 49 58 54
0020: 55 52 45 00 00 00 00 00 00 00 00 00 00 00 00 00
0030: 00 00 00 00 00 00 00 00 00 00 00 00 30 31 30 30
0040: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0050: 00 00 00 00 00 00 00 94 00 00 00 28 00 00 00 BC
0060: 00 00 00 4C 00 00 01 10 00 00 01 90 00 00 00 00
0070: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0080: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0090: 00 00 00 00 FF D8 FF E1 00 22 45 78 69 66 00 00
00A0: 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00
00B0: 00 00 01 00 00 00 00 00 00 00 FF D9 00 00 00 05
00C0: 01 00 00 04 00 08 00 08 01 10 00 04 00 01 00 01
00D0: 01 11 00 04 00 06 00 06 01 30 00 04 0C 0C 0C 0C
00E0: 01 31 00 24 01 00 02 01 02 00 02 01 01 00 01 01
00F0: 00 01 01 02 01 01 01 02 00 01 00 02 00 01 01 02
0100: 01 01 02 01 01 00 01 01 00 00 00 00 00 00 00 00
0110: 49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00
0120: 00 00 1A 00 00 00 00 00 00 00 07 00 01 F0 04 00
0130: 01 00 00 00 08 00 00 00 02 F0 04 00 01 00 00 00
0140: 08 00 00 00 03 F0 04 00 01 00 00 00 0E 00 00 00
0150: 07 F0 04 00 01 00 00 00 10 01 00 00 08 F0 04 00
0160: 01 00 00 00 80 00 00 00 0A F0 04 00 24 00 00 00
0170: 74 00 00 00 0E F0 04 00 03 00 00 00 04 01 00 00
0180: 00 00 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
0190: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01A0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01B0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01C0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01D0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01E0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01F0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
0200: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
0210: FE 03 00 00 2E 01 00 00 47 02 00 00 48 02 00 00
0220: 9E 13 9E 13 CE 0B 9E 13 9E 13 E6 07 9E 13 9E 13
0230: 9E 13 9E 13 E6 07 9E 13 9E 13 CE 0B 9E 13 9E 13
0240: E6 07 CE 0B 9E 13 CE 0B E6 07 9E 13 E6 07 CE 0B
0250: 9E 13 9E 13 E6 07 9E 13 9E 13 CE 0B 9E 13 9E 13
0260: 9E 13 9E 13 CE 0B 9E 13 9E 13 E6 07 9E 13 9E 13
0270: CE 0B E6 07 9E 13 E6 07 CE 0B 9E 13 CE 0B E6 07
0280: 9E 13 9E 13 CE 0B 9E 13 9E 13 E6 07 9E 13 9E 13
0290: 9E 13 9E 13 E6 07 9E 13 9E 13 CE 0B 9E 13 9E 13

Annotations: meta container at 0xBC with 5 records, length 0x4C. CFA container at 0x110, length 0x190. FujiIFD: W 8, H 8, bits 14, strip at container offset 0x110 (file offset 0x220), 0x80 = 128 bytes, 0xF00A 36 values at container offset 0x74, 0xF00E at 0x104. Sensor values 0x139E = 5022 (G), 0x0BCE = 3022 (R), 0x07E6 = 2022 (B). Expected: normalized and balanced R 0.2513461, G 0.2603997, B 0.1258886. Every output pixel is (0.2735791, 0.3303615, 0.0570704). Output size 6 x 6 (orientation 1).

F2 (V1, C2, 8 x 4 raster, crop 6 x 2 at (1, 1), black 254 255 255 254, white balance 302 466 440, flat field R 1255, G 1755, B 1055, orientation 6), complete file of 416 bytes:

0000: 46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20
0010: 30 32 30 31 46 46 30 30 30 30 30 31 46 49 58 54
0020: 55 52 45 00 00 00 00 00 00 00 00 00 00 00 00 00
0030: 00 00 00 00 00 00 00 00 00 00 00 00 30 31 30 30
0040: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0050: 00 00 00 00 00 00 00 94 00 00 00 28 00 00 00 BC
0060: 00 00 00 24 00 00 00 E0 00 00 00 C0 00 00 00 00
0070: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0080: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0090: 00 00 00 00 FF D8 FF E1 00 22 45 78 69 66 00 00
00A0: 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00
00B0: 00 00 06 00 00 00 00 00 00 00 FF D9 00 00 00 04
00C0: 01 00 00 04 00 04 00 08 01 10 00 04 00 01 00 01
00D0: 01 11 00 04 00 02 00 06 01 30 00 04 0A 0B 09 08
00E0: 49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00
00F0: 00 00 1A 00 00 00 00 00 00 00 07 00 01 F0 04 00
0100: 01 00 00 00 08 00 00 00 02 F0 04 00 01 00 00 00
0110: 04 00 00 00 03 F0 04 00 01 00 00 00 0C 00 00 00
0120: 07 F0 04 00 01 00 00 00 90 00 00 00 08 F0 04 00
0130: 01 00 00 00 30 00 00 00 0A F0 04 00 04 00 00 00
0140: 74 00 00 00 0E F0 04 00 03 00 00 00 84 00 00 00
0150: 00 00 00 00 FE 00 00 00 FF 00 00 00 FF 00 00 00
0160: FE 00 00 00 2E 01 00 00 D2 01 00 00 B8 01 00 00
0170: E7 B4 6D E7 B4 6D E7 B4 6D E7 B4 6D DB F6 41 DB
0180: F6 41 DB F6 41 DB F6 41 E7 B4 6D E7 B4 6D E7 B4
0190: 6D E7 B4 6D DB F6 41 DB F6 41 DB F6 41 DB F6 41

Annotations: the stub holds orientation 6 (byte 0xB2). Meta container with 4 records, length 0x24. CFA container at 0xE0, length 0xC0. Strip at container offset 0x90 (file offset 0x170), 48 bytes. Even rows repeat E7 B4 6D (R 1255, G 1755), odd rows repeat DB F6 41 (G 1755, B 1055). Black b = 254.5, w = 4095. Expected: normalized and balanced R 0.4019836, G 0.3907043, B 0.3036822. Every output pixel is (0.4232362, 0.4324534, 0.2594871). Output size 2 wide x 6 high (orientation 6).

F3 (V2, C2, 16 x 4 raster, crop 14 x 2 at (1, 1), black 4 x 1024, white balance 256 421 510, flat field R 3024, G 5024, B 2024, orientation 1), complete file of 480 bytes:

0000: 46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20
0010: 30 32 30 31 46 46 30 30 30 30 30 31 46 49 58 54
0020: 55 52 45 00 00 00 00 00 00 00 00 00 00 00 00 00
0030: 00 00 00 00 00 00 00 00 00 00 00 00 30 31 30 30
0040: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0050: 00 00 00 00 00 00 00 94 00 00 00 28 00 00 00 BC
0060: 00 00 00 24 00 00 00 E0 00 00 01 00 00 00 00 00
0070: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0080: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0090: 00 00 00 00 FF D8 FF E1 00 22 45 78 69 66 00 00
00A0: 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00
00B0: 00 00 01 00 00 00 00 00 00 00 FF D9 00 00 00 04
00C0: 01 00 00 04 00 04 00 10 01 10 00 04 00 01 00 01
00D0: 01 11 00 04 00 02 00 0E 01 30 00 04 0A 0B 09 08
00E0: 49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00
00F0: 00 00 1A 00 00 00 00 00 00 00 07 00 01 F0 04 00
0100: 01 00 00 00 10 00 00 00 02 F0 04 00 01 00 00 00
0110: 04 00 00 00 03 F0 04 00 01 00 00 00 0E 00 00 00
0120: 07 F0 04 00 01 00 00 00 90 00 00 00 08 F0 04 00
0130: 01 00 00 00 70 00 00 00 0A F0 04 00 04 00 00 00
0140: 74 00 00 00 0E F0 04 00 03 00 00 00 84 00 00 00
0150: 00 00 00 00 00 04 00 00 00 04 00 00 00 04 00 00
0160: 00 04 00 00 00 01 00 00 A5 01 00 00 FE 01 00 00
0170: 02 3A 41 2F 2F A0 13 F4 F4 02 3A 41 41 2F A0 13
0180: 13 F4 02 3A 3A 41 2F A0 A0 13 F4 02 84 7E 80 4E
0190: 4E E8 07 E8 E8 84 7E 80 80 4E E8 07 07 E8 84 7E
01A0: 7E 80 4E E8 E8 07 E8 84 02 3A 41 2F 2F A0 13 F4
01B0: F4 02 3A 41 41 2F A0 13 13 F4 02 3A 3A 41 2F A0
01C0: A0 13 F4 02 84 7E 80 4E 4E E8 07 E8 E8 84 7E 80
01D0: 80 4E E8 07 07 E8 84 7E 7E 80 4E E8 E8 07 E8 84

Annotations: strip at container offset 0x90 (file offset 0x170), 112 bytes, 28 bytes per row. The first word 02 3A 41 2F is 0x2F413A02 = binary 0010 1111 0100 0001 0011 1010 0000 0010: its first 14 bits 00 1011 1101 0000 = 3024 (R) and the next 14 bits 01 0011 1010 0000 = 5024 (G). Expected: normalized and balanced R 0.2141456, G 0.2604336, B 0.1297082. Every output pixel is (0.2207258, 0.3361114, 0.0621115). Output size 14 x 2.

F4 (X1): F1 with byte 0x6F changed from 00 to 02 (RAFCompression 2). Expected: class X1. The decoder does not read the strip. The only preview candidate is the stub of F1, which has no frame, so the decoder reports an error (R-RAF-52).

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 Fujifilm lossless compressed RAF (X1, RAFCompression 2). No permitted public document describes the coding, and measurement alone cannot establish a bit-exact entropy decoder. Fallback: preview path (R-RAF-2).
- OQ-2 Fujifilm lossy compressed RAF (X2, RAFCompression 3). As OQ-1. Fallback: preview path.
- OQ-3 Legacy containers (X3: SuperCCD FinePix S2Pro, S3Pro, S5Pro, older FinePix bodies, GX680 back). Their CFA container has no TIFF structure and some use a rotated sensor layout. No permitted source describes the data. Fallback: preview path.
- OQ-4 Two-image EXR files (X4). The second image and the EXR color filter layout are not described by a permitted source. Fallback: preview path.
- OQ-5 Meaning of meta record 0x0130 (FujiLayout). Only the value 0A 0B 09 08 is tied to a measured Bayer phase (three bodies, M9). FinePix S1 and SL1000 carry the same value but their pixel data were not measured. Fallback: R-RAF-19 accepts only this value. Other values give X5.
- OQ-6 Order of the 4 or 36 BlackLevel values relative to the pattern positions. All measured values are equal or differ by 1. Fallback: one mean black level (R-RAF-29).
- OQ-7 White level. No tag holds it. Measured saturation is 4094 (12-bit), 16322 and 16381 (14-bit). The 16-bit mode of the GFX 100 family was not measured with pixel data. Fallback: 2^b - 1 (R-RAF-28).
- OQ-8 Color matrix. RAF files carry none, and no permitted source gives per-model matrices for Fujifilm cameras. The single matrix of R-RAF-35 comes from one X-Trans camera (X100S) at D65. Fallback: R-RAF-35 for every model and every illuminant.
- OQ-9 Lens correction tags 0xF00B, 0xF00F, 0xF010 (S5 names distortion, chromatic aberration and vignetting). Their parameter semantics are not documented. Fallback: no lens correction. The decoded image therefore shows the lens distortion that the camera JPEG removes (M13 found offsets of up to 5 raw pixels).
- OQ-10 Orientation of rotated shots. All measured files have Orientation 1, so it is not known whether the camera stores the main JPEG rotated. Fallback: R-RAF-43 for the raw image and R-RAF-51 for the preview.
- OQ-11 Meaning of FujiIFD tags 0xF004, 0xF005, 0xF006, 0xF009, 0xF011 to 0xF014 and meta records 0x0112, 0x0113, 0x0141, 0x9651, 0xC000. Fallback: ignored.
- OQ-12 Big-endian CFA containers (4D 4D 00 2A) were not observed. Fallback: X3 by R-RAF-3 rule 4.
- OQ-13 Multi-frame files. S5 lists "M-RAW" tags of multi-image RAF files and the maker note tags PixelShiftShots and MultipleExposure. No such file was measured. Fallback: R-RAF-3 classifies by structure, and any unmatched structure uses the preview path.
- OQ-14 Uncompressed files from crop modes (sports finder, 16:9). Only compressed files of these modes were surveyed (X-T5, X-M5). Fallback: the crop records (R-RAF-16).
- OQ-15 V2 rows with W not a multiple of 16. Not observed. Fallback: X6 (R-RAF-26).
- OQ-16 Highlights. Clipped values and white-balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-RAF-34.
- OQ-17 RawExposureBias (meta record 0x9650, S5). Measured values are negative (for example -0.72 EV for X-T3, -1.72 EV for X100), which suggests that the camera exposes the raw data lower than the JPEG in its dynamic range modes. The use of the value is not documented. Fallback: not applied. The decoded image can be darker than the preview.
- OQ-18 Generic white balance. Files without 0xF00E and 0xF00D were not observed in scope. Fallback: R-RAF-33 source 3.
