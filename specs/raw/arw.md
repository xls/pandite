# Sony ARW raw format: decoder specification

Document: specs/raw/arw.md. Version 1.0. Date 2026-11-13.
Status: normative input for the pandite ARW decoder (clean-room step 1 of 2).

This document describes the Sony ARW raw file format as facts. It is written for an implementer who has only this document, TIFF 6.0 (S1) and ITU-T T.81 (S2). Every normative statement is traceable to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Conventions

- Requirements have the form R-ARW-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x49 0x49 or "49 49".
- Bit 0 is the least significant bit of a value.
- floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0).
- Pixel coordinates are (x, y): x is the column, y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (tags 0x0100 and 0x0101 of the raw IFD).

### Glossary

- ARW: the raw file format of Sony Alpha, NEX, Cyber-shot RX and ZV cameras. ARW is a TIFF file.
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2).
- IFD0, IFD1, IFD2: the first, second and third IFD of the main IFD chain.
- SubIFD: an IFD whose offset is in tag 0x014A (SubIFDs) of IFD0.
- Raw IFD: the SubIFD that describes the sensor data.
- CFA: color filter array. In all measured ARW files it is a 2x2 Bayer pattern (S11).
- CFA position: the index 2 * (y mod 2) + (x mod 2) of a pixel inside its 2x2 repeat cell (0, 1, 2 or 3).
- Variant: one storage method of the sensor data. This document defines five variants, V1 to V5, in the section Scope and variants.
- Block format: variant V3, the lossy 128-bit block coding that Sony calls "Compressed" RAW.
- Code value: an 11-bit value (0 to 2047) inside a V3 block, before the tone curve.
- Tone curve: the piecewise linear function that maps a V3 code value to a linear value.
- 14-bit scale: the common scale of all decoded sensor values in this document. On this scale the black level is near 512 (or 800 on some 1-inch sensors) and saturation is near 16383.
- Black level: the sensor value that represents zero light.
- White level: the sensor value at saturation.
- WB multipliers: the per-channel gains that make a neutral surface give equal red, green and blue values.
- Camera RGB: red, green and blue values in the spectral sensitivities of the camera.
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S12). The decoder output is linear sRGB.
- LJ92: the lossless JPEG process of ITU-T T.81 Annex H with Huffman coding (frame marker SOF3, 0xFF 0xC3).
- MCU: minimum coded unit (S2 section 3 and A.2).
- Tile: a rectangular part of the raster that is stored separately (S1 section 15).
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-13 | Container layout, Metadata tags, Sensor data layout, Compression (V5 color formulas), Decoding procedure (orientation) |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-13 | Compression (V4, V5), Embedded preview, Error handling |
| S3 | Digital Negative (DNG) Specification 1.6.0.0, Adobe, December 2021 (copy read at the URL, official landing page https://helpx.adobe.com/camera-raw/digital-negative.html) | https://paulbourke.net/dataformats/dng/dng_spec_1_6_0_0.pdf | official | 2026-11-13 | Metadata tags (WhiteLevel, DefaultCropOrigin, DefaultCropSize, PhotometricInterpretation 32803), Sensor data layout |
| S4 | Exif Version 2.32, CIPA DC-X008-Translation-2019 | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-13 | Sensor data layout (CFA value meanings and order) |
| S5 | ExifTool Tag Names: EXIF tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/EXIF.html | community documentation | 2026-11-13 | Scope and variants, Metadata tags, Embedded preview |
| S6 | ExifTool Tag Names: Sony tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/Sony.html | community documentation | 2026-11-13 | Scope and variants, Container layout (SR2Private, encrypted blocks), Open questions |
| S7 | RawDigger: detecting posterization in SONY cRAW/ARW2 files, LibRaw LLC, 2014-03-06 (article body only, comments not used) | https://www.rawdigger.com/howtouse/sony-craw-arw2-posterization-detection | community documentation | 2026-11-13 | Compression (V3 block structure, step table, tone curve, processing order) |
| S8 | Sony Help Guide ILCE-7M4, Image Quality Settings: RAW File Type | https://helpguide.sony.net/ilc/2110/v1/en/contents/TP1000659395.html | vendor | 2026-11-13 | Scope and variants (Uncompressed, Lossless Comp L/M/S, Compressed, RAW always 3:2) |
| S9 | DXOMARK camera sensor Measurements pages, tab Color response (one page per model, URL pattern https://www.dxomark.com/Cameras/Sony/<name>---Measurements, names in the Color conversion table) | https://www.dxomark.com/Cameras/Sony/A7-IV---Measurements | community documentation | 2026-11-13 | Color conversion (matrices, D50 white balance scales) |
| S10 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-13 | Decoding procedure (bilinear demosaicing) |
| S11 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-13 | Sensor data layout (Bayer arrangement) |
| S12 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-13 | Color conversion (output primaries and D65 white) |
| S13 | Measurement: ILCE-7M4 sample set DSC06673 (Uncompressed), DSC06674 (Lossless L), DSC06675 (Lossless M), DSC06676 (Lossless S), DSC06677 (Compressed), public domain (CC0) files | https://raw.pixls.us/data/Sony/ILCE-7M4/ | measurement | 2026-11-13 | all sections, records M1 to M13 |
| S14 | Measurement: ILCE-7RM5 samples 7RM5-LosslessCompressedLarge, -Medium, -Small (CC0) | https://raw.pixls.us/data/Sony/ILCE-7RM5/ | measurement | 2026-11-13 | Compression (V4, V5), Levels and white balance, records M8 to M10 |
| S15 | Measurement: ILCE-1 sample A1_full_lossless_compressed (CC0) | https://raw.pixls.us/data/Sony/ILCE-1/A1_full_lossless_compressed.ARW | measurement | 2026-11-13 | Compression (V4), record M8 |
| S16 | Measurement: ILCE-9M3 samples DSC00110 to DSC00118 (CC0) | https://raw.pixls.us/data/Sony/ILCE-9M3/ | measurement | 2026-11-13 | Scope and variants, Compression (V3), records M1, M4 |
| S17 | Measurement: ILCE-7M3 sample _DSC0009 (CC0) | https://raw.pixls.us/data/Sony/ILCE-7M3/_DSC0009.ARW | measurement | 2026-11-13 | Compression (V3), Sensor data layout, records M4, M16 |
| S18 | Measurement: ILCE-7RM3 sample "Sony - ILCE-7RM3 - 12bit uncompressed (3:2)" (CC0) | https://raw.pixls.us/data/Sony/ILCE-7RM3/ | measurement | 2026-11-13 | Compression (V1), record M6 |
| S19 | Measurement: NEX-5 sample (file name begins 20150924T092802) (CC0) | https://raw.pixls.us/data/Sony/NEX-5/ | measurement | 2026-11-13 | Compression (V3 tone curve), records M4, M5 |
| S20 | Measurement: DSLR-A700 sample DSC07249 (CC0) | https://raw.pixls.us/data/Sony/DSLR-A700/DSC07249.ARW | measurement | 2026-11-13 | Compression (V3), records M4, M5 |
| S21 | Measurement: DSLR-A900 sample _DSC7969 (CC0) | https://raw.pixls.us/data/Sony/DSLR-A900/_DSC7969.ARW | measurement | 2026-11-13 | Compression (V2), record M7 |
| S22 | Measurement: DSLR-A200 sample DSC00108 (CC0) | https://raw.pixls.us/data/Sony/DSLR-A200/DSC00108.ARW | measurement | 2026-11-13 | Scope and variants (X4), record M14 |
| S23 | Measurement: DSLR-A100 sample _DSC0258 (CC0) | https://raw.pixls.us/data/Sony/DSLR-A100/_DSC0258.ARW | measurement | 2026-11-13 | Scope and variants (X3), record M14 |
| S24 | Measurement: DSC-R1 sample _DSC1477.SR2 (CC0) | https://raw.pixls.us/data/Sony/DSC-R1/_DSC1477.SR2 | measurement | 2026-11-13 | Scope and variants (X2), record M14 |
| S25 | Measurement: DSC-F828 sample DSC06227.SRF (CC0) | https://raw.pixls.us/data/Sony/DSC-F828/DSC06227.SRF | measurement | 2026-11-13 | Scope and variants (X1), record M14 |
| S26 | Measurement: DSC-RX100 DSC00838, DSC-RX100M3 DSC00734, DSC-RX0 DSC00009 (CC0) | https://raw.pixls.us/data/Sony/DSC-RX100/DSC00838.ARW | measurement | 2026-11-13 | Levels and white balance (black level 800), record M11 |
| S27 | Measurement: container survey, first 512 KiB or 1 MiB of one file for each of 51 Sony models, 49 of them parsed (the DSLR-A550 and NEX-3 headers did not parse inside the downloaded range) (CC0 archive) | https://raw.pixls.us/data/Sony/ | measurement | 2026-11-13 | Scope and variants, Container layout, Metadata tags, record M1 |

Sample files are from the public raw sample archive raw.pixls.us. The archive states that contributors release the files into the public domain (CC0). The author downloaded the files and inspected them with measurement tools that the author wrote from S1 and S2 for this purpose. The tools are not part of the deliverable.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S13 to S27): every ARW and SR2 file starts with 49 49 2A 00 08 00 00 00 (little-endian, IFD0 at offset 8). IFD0 holds Make, Model, Orientation, SubIFDs (one offset), the preview JPEG (0x0201/0x0202), the Exif IFD (0x8769) and SR2Private (0xC634). IFD1 holds a 160x120 JPEG thumbnail. Files with FileFormat 4.0.0.0 or later have IFD2 with a full-size JPEG. The SubIFD is the raw IFD. Raw IFD entries are not in ascending tag order (for example 0x7000 follows 0x828E and 0x0111 follows 0x7020). The tags 0x7310, 0x7313, 0xC61D, 0xC61F, 0xC620 occur in the raw IFD from FileFormat 3.3.3.0 on and never in older files.
- M2 (V3 bit layout, S13 Compressed): in 176000 blocks (rows 0 to 399) read as little-endian 128-bit integers with MAX in bits 0 to 10, MIN in bits 11 to 21, IMAX in bits 22 to 25 and IMIN in bits 26 to 29, MAX >= MIN in all blocks and IMAX != IMIN in all blocks. The big-endian, most-significant-bit-first reading gives MAX < MIN in 87044 blocks. The decoded image is a clean photograph.
- M3 (V3 shift, S13): blocks with MAX - MIN = 128 have deltas of at most 64 (step 2). Over all blocks the largest delta per shift is 127 for shifts 0 to 3. A reconstructed value MIN + delta * step exceeds MAX in 40126 of 2464000 delta values, and always by at most step / 2. The camera therefore rounds deltas to the nearest step.
- M4 (V3 pixel order): mean absolute difference between pixels two columns apart is 57.8 with the interleaved order (first block to even columns, second block to odd columns) and 118.5 with a contiguous order (S13). Same result on S16, S17, S19, S20 and S26. Mean code values at CFA positions 0, 1, 2, 3 are 509, 805, 805, 521 (S13): positions 1 and 2 are the two greens.
- M5 (tone curve, S13, S16, S17, S19, S20, S26): tag 0x7010 is 8000, 10400, 12900, 14100 in all files except NEX-5 (0, 9824, 11512, 16362). The lowest code values are near 256 for the standard curve and near 128 for the NEX-5 curve. Both map to 512 with the curve of this document, which equals the BlackLevel tag value 512 of newer files. The highest code value in a sample of S13 (every 8th row) is 2029.
- M6 (V1, S13 Uncompressed, S18): samples are 16-bit little-endian. S13: minimum 491, maximum 16383, 11778 samples at 16383 in a 1/13 sample. S18 (BitsPerSample 12): every sample has its two low bits zero, minimum 504, and the darkest values are near 512. V1 values are on the 14-bit scale in both bit depths.
- M7 (V2, S21): little-endian 12-bit packing (first pixel in the low 12 bits of a 24-bit group) gives a mean same-color difference of 15.9, the alternative order gives 201.9. Mean values at CFA positions 0 to 3: 215, 374, 374, 275. Range 130 to 2992.
- M8 (V4, S13 Lossless L, S14 Large, S15): every tile is one LJ92 stream with P = 14, X = 256, Y = 256, Nf = 4, all sampling factors 1, one Huffman table, predictor 1, Pt = 0. Mapping the four components to the 2x2 cell (component 1 top-left, 2 top-right, 3 bottom-left, 4 bottom-right) gives a same-color roughness of 129, the row-run mapping gives 419. Swapping components 2 and 3 raises the green diagonal roughness from 136 to 230. TileOffsets are not in file order. Pixels right of SonyRawImageSize contain non-zero data. Pixels below it are zero.
- M9 (V5, S13 Lossless M and S, S14 Medium and Small): every tile is one LJ92 stream with P = 16, X = 512, Y = 512, Nf = 3. Component 1 has H = 2 and V = 2 (M) or V = 1 (S). Components 2 and 3 have H = V = 1. Huffman table 0 for component 1 and table 1 for components 2 and 3. Predictor 1, Pt = 0. With the T.81 line-start rule the M tiles show 1185 (S13) and 290 (S14) rows with a whole-row offset. With the line-start rule R-ARW-39 the counts fall to 7 and 12, and the remaining rows are real image edges. S tiles decode cleanly with the T.81 rule.
- M10 (V5 levels, S13 L and S, S14 Large and Small): RGB from V5 (formulas R-ARW-42) matches the V4 raw values of the same scene as RGB = WBc * (raw - 512) + p with p between 1013 and 1018 for all three channels (S13 percentile fit). In S14 the 0.01 percentile of green is 547 in V4 and 1058 in V5 (512 + 35 and 1024 + 34). The ratios of the fitted gains R/G and B/G are 2.36 and 1.74, equal to the WB_RGGBLevels ratios 2.37 and 1.76 within 1.3 percent. A comparison with the embedded full-size JPEG gives a chromaticity error of 0.026 with no further white balance and 0.158 when the WB multipliers are applied a second time.
- M11 (black level of 1-inch sensors, S26, S27): the lowest code values of DSC-RX100, DSC-RX100M3 and DSC-RX0 are near 400, which maps to 800. DSC-RX100M7 and DSC-RX10M4 carry BlackLevel 800 in tag 0x7310.
- M12 (color matrix orientation, S13 Lossless L): white-balanced camera RGB times the S9 matrix of ILCE-7M4, read with rows as output channels, matches the embedded full-size camera JPEG with a mean chromaticity error of 0.030 over 6761 blocks. The transposed matrix gives 0.165 and the identity gives 0.044.
- M13 (previews, S13 to S27): the IFD0 JPEG is a baseline JPEG (SOF0) of 1616x1080 pixels in ARW 2.x and later (640x480 in ARW 1.0 and 640x424 in SR2). The IFD1 JPEG is 160x120. The IFD2 JPEG is full size and equals DefaultCropSize (for example 7008x4672 in S13). The preview streams start with FF D8 FF DB (no APP1 segment). Some streams end with padding bytes after the EOI marker or have no EOI at the end of the stated length.
- M14 (out-of-scope structures): DSLR-A100 (S23): the SubIFDs value points to sensor data. Read as an IFD it claims 7520 entries. DSLR-A200 (S22) and the survey models DSLR-A230, A290, A300, A330, A350, A380, A390 (S27): Compression 32767 with StripByteCounts different from W * H and from 1.5 * W * H, and W not a multiple of 32. DSC-R1 (S24): Compression 1 with 16-bit samples, but read as little-endian 7838232 of the 10390272 samples exceed 16383, so the V1 reading does not apply. DSC-F828 (S25): IFD0 has no SubIFDs tag. IFD1 holds the JPEG preview.
- M15 (SR2SubIFD, S13): at the offset in tag 0x7200 the bytes do not form an IFD (the entry count reads 49917). This agrees with S6, which calls the SR2SubIFD encrypted.
- M16 (edges, S17): columns 6012 to 6047 and rows 4012 to 4023, outside DefaultCrop, hold image-like data that continues the image without a step.

### Sources encountered and not used

The page https://github.com/lclevy/sony_raw (readme.md) states that its compression section is based on decoder source code. The author stopped reading it at that statement and used nothing from it. The comments below the S7 article discuss decoder source code. Only the article body of S7 is used. See PROVENANCE.md.

## Scope and variants

### ARW generations

The maker note of an ARW file carries tag 0xB000 FileFormat (4 bytes). S6 names the values. The table lists the values, the measured models (M1, S27) and the variants seen. The decoder does not need the maker note: R-ARW-3 classifies files by the raw IFD alone.

| FileFormat bytes | Name (S6) | Measured models | Variants seen |
| --- | --- | --- | --- |
| 1 0 0 0 | SR2 | DSC-R1 | X2 |
| 2 0 0 0 | ARW 1.0 | DSLR-A100 | X3 |
| 3 0 0 0 | ARW 2.0 | DSLR-A700 (V3), DSLR-A200, A300, A350 (X4) | V3, X4 |
| 3 1 0 0 | ARW 2.1 | DSLR-A850, A900 (V2), DSLR-A450, A500 (V3), DSLR-A230, A290, A330, A380, A390 (X4) | V2, V3, X4 |
| 3 2 0 0 | ARW 2.2 | NEX-5, NEX-C3, SLT-A33, SLT-A55V, DSLR-A560, A580 | V3 |
| 3 3 0 0 | ARW 2.3 | NEX-7, SLT-A77V, DSC-RX100, DSC-RX1 | V3 |
| 3 3 1 0 | ARW 2.3.1 | ILCE-7, 7R, 7S, 7M2, 7RM2, ILCE-6000, 5100, QX1, SLT-A99V, DSC-RX100M3 | V3 |
| 3 3 2 0 | ARW 2.3.2 | ILCE-6300, 7SM2, ILCA-99M2, DSC-RX1RM2, DSC-RX0 | V3 |
| 3 3 3 0 | ARW 2.3.3 | ILCE-7M3, ILCE-9, ILCE-7RM3, DSC-RX10M4 | V1, V3 |
| 3 3 5 0 | ARW 2.3.5 | ILCE-6400, 6600, 7C, 7RM4, ZV-E10, DSC-RX100M7 | V1, V3 |
| 4 0 0 0 | ARW 4.0 | ILCE-1, ILCE-7SM3, ILCE-7RM5 | V1, V3, V4, V5 |
| 4 0 1 0 | ARW 4.0.1 | ILCE-7M4, 6700, 7CM2, 7CR, ZV-E1, ILME-FX3 | V1, V3, V4, V5 |
| 5 0 0 0 | ARW 5.0 | ILCE-9M3 | V1, V3, V4, V5 |
| 5 0 1 0, 6 0 0 0 | ARW 5.0.1, ARW 6.0 | listed by S6, no sample measured | classify by structure |

### Storage variants

S8 names three user settings for current bodies: Uncompressed, Lossless Comp with the sizes L, M and S, and Compressed. S8 states that RAW images are always 3:2 and that the M and S sizes have fewer pixels than L. The measured files map these settings and the older storage methods to the variants below.

| Variant | Camera setting | Compression (0x0103) | Photometric (0x0106) | Sensor data | SonyRawFileType (0x7000) seen |
| --- | --- | --- | --- | --- | --- |
| V1 | Uncompressed (14-bit and 12-bit modes) | 1 | 32803 | one strip, 16-bit little-endian samples | 0 |
| V2 | Uncompressed 12-bit of DSLR-A850 and A900 | 32767 | 32803 | one strip, 12-bit packed, StripByteCounts = 1.5 * W * H | 1 |
| V3 | Compressed (lossy) | 32767 | 32803 | one strip, 128-bit blocks, StripByteCounts = W * H | 2 |
| V4 | Lossless Comp L | 7 | 32803 | 512x512 tiles, LJ92 with 4 components | 4 |
| V5 | Lossless Comp M and S | 7 | 6 | 512x512 tiles, LJ92 YCbCr with 3 components | 4 |

S5 lists SonyRawFileType values 0 (Uncompressed 14-bit), 1 (Uncompressed 12-bit), 2 (Compressed), 3 (Lossless Compressed), 4 (Lossless Compressed 2) and 6 (Compressed RAW 2). The measured DSLR-A200 family (X4) carries value 3 with a variable-length coding that is not lossless JPEG. Value 6 was not measured. The decoder therefore classifies by structure and not by tag 0x7000.

### Out-of-scope relatives

| Id | Relative | Detection from file contents (M14) |
| --- | --- | --- |
| X1 | SRF (DSC-F828, DSC-V3) | IFD0 has no tag 0x014A |
| X2 | SR2 (DSC-R1) | IFD0 Model is "DSC-R1" |
| X3 | ARW 1.0 (DSLR-A100) | the first SubIFD offset does not point to a valid IFD (R-ARW-14) |
| X4 | ARW 2.x variable-length coding (DSLR-A200, A230, A290, A300, A330, A350, A380, A390) | Compression 32767 and the strip byte count matches neither V2 nor V3 |
| X5 | any other combination, including ARQ pixel-shift composites and unmeasured compressions | no rule of R-ARW-3 matches |
| X6 | big-endian files | header bytes 0 and 1 are 4D 4D |

### Requirements

- R-ARW-1: The decoder must decode the sensor data of variants V1, V2, V3, V4 and V5 as this document specifies.
- R-ARW-2: The decoder must use the preview path (section Embedded preview) for X1, X2, X3, X4, X5 and X6 files and must not interpret their sensor data.
- R-ARW-3: The decoder must classify the file by these rules, in this order, and must treat a file that matches no rule as X5:
  1. IFD0 has no tag 0x014A: X1.
  2. No SubIFD qualifies as raw IFD (R-ARW-13): X3.
  3. Model (IFD0 tag 0x0110, compared by R-ARW-10) equal to "DSC-R1": X2.
  4. Compression 1, PhotometricInterpretation 32803, SamplesPerPixel 1, total strip bytes >= 2 * W * H: V1.
  5. Compression 32767, PhotometricInterpretation 32803, W even, total strip bytes = 3 * W * H / 2: V2.
  6. Compression 32767, PhotometricInterpretation 32803, W mod 32 = 0, total strip bytes = W * H: V3.
  7. Compression 32767 in any other case: X4.
  8. Compression 7, PhotometricInterpretation 32803, tags 0x0142 to 0x0145 present: V4.
  9. Compression 7, PhotometricInterpretation 6, SamplesPerPixel 3, tags 0x0142 to 0x0145 present: V5.

"Total strip bytes" is the sum of all values of StripByteCounts (0x0117). SamplesPerPixel is 1 when the tag is absent.

## Container layout

### Byte order and header

An ARW file is a TIFF file (S1 section 2). All measured files are little-endian (M1).

| Offset | Size | Field | Value in ARW |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 49 49 ("II", little-endian) |
| 2 | 2 | TIFF magic number | 42 (bytes 2A 00) |
| 4 | 4 | offset of IFD0 | 8 in all measured files |

- R-ARW-4: The decoder must accept a file only if bytes 0 to 3 are 49 49 2A 00, and must send a file with bytes 4D 4D 00 2A to the preview path (X6).
- R-ARW-5: The decoder must read every multi-byte TIFF field and every V1, V2 and V3 sample as little-endian.

### IFD structure

An IFD (S1 section 2) is:

| Offset in IFD | Size | Field |
| --- | --- | --- |
| 0 | 2 | N, the number of entries |
| 2 + 12 * i | 2 | tag of entry i |
| 4 + 12 * i | 2 | field type of entry i |
| 6 + 12 * i | 4 | count of entry i (number of values, not bytes) |
| 10 + 12 * i | 4 | value or offset of entry i |
| 2 + 12 * N | 4 | offset of the next IFD, 0 at the end of the chain |

Field types and their sizes in bytes (S1 section 2 defines types 1 to 12). Type 13 is not defined by S1 and does not occur in the measured ARW files. The decoder treats it as LONG (R-ARW-8).

| Type | Name | Size | Meaning |
| --- | --- | --- | --- |
| 1 | BYTE | 1 | unsigned 8-bit |
| 2 | ASCII | 1 | 8-bit character, last byte 0 |
| 3 | SHORT | 2 | unsigned 16-bit |
| 4 | LONG | 4 | unsigned 32-bit |
| 5 | RATIONAL | 8 | two LONG: numerator, denominator |
| 6 | SBYTE | 1 | signed 8-bit |
| 7 | UNDEFINED | 1 | byte |
| 8 | SSHORT | 2 | signed 16-bit |
| 9 | SLONG | 4 | signed 32-bit |
| 10 | SRATIONAL | 8 | two SLONG |
| 11 | FLOAT | 4 | IEEE single |
| 12 | DOUBLE | 8 | IEEE double |
| 13 | IFD (not in S1) | 4 | unsigned 32-bit offset of an IFD |

The value bytes of an entry occupy count * size bytes. If that total is 4 or less, the value is in the 4-byte value field, starting at its first byte. If the total is more than 4, the value field holds the file offset of the value bytes.

- R-ARW-6: The decoder must read a value from the value field when count * size <= 4 and from the offset in the value field when count * size > 4.
- R-ARW-7: The decoder must find tags by tag number in any entry order, because Sony raw IFDs are not sorted (M1).
- R-ARW-8: The decoder must accept SHORT, LONG and IFD types for integer tags that this document lists as "SHORT or LONG", and BYTE for tag 0x828E.
- R-ARW-9: The decoder must skip entries with a type outside 1 to 13 and must not use their values.
- R-ARW-10: The decoder must compare ASCII values after it removes all trailing zero bytes and spaces.

### IFD chain and SubIFDs

Measured layout (M1, values from S13 Compressed):

| Structure | Where | Content in ARW |
| --- | --- | --- |
| IFD0 | offset 8 | Make, Model, Orientation, SubIFDs, preview JPEG (0x0201, 0x0202), Exif IFD (0x8769), XMP (0x02BC), SR2Private (0xC634), Compression 6 and NewSubfileType 1 |
| IFD1 | next-IFD offset of IFD0 | 160x120 thumbnail JPEG (0x0201, 0x0202) |
| IFD2 | next-IFD offset of IFD1 (FileFormat 4.0.0.0 and later) | full-size JPEG: ImageWidth and ImageLength equal DefaultCropSize, Compression 7, 0x0201 and 0x0202 |
| raw IFD | first offset in IFD0 tag 0x014A | sensor data and the tags of section Metadata tags, NewSubfileType 0 |
| Exif IFD | IFD0 tag 0x8769 | Exif data and the MakerNote (0x927C), not needed for decoding |

Example (S13 Compressed): IFD0 at 8 with 19 entries. Raw IFD at 138478 with 35 entries. Exif IFD at 4544. IFD1 at 43672. IFD2 at 138170. Preview JPEG at 192674 (498845 bytes). Full-size JPEG at 692224 (6964228 bytes). Sensor strip at 7659520 (33003520 bytes).

- R-ARW-11: The decoder must read IFD0 at the offset in header bytes 4 to 7.
- R-ARW-12: The decoder must follow the next-IFD offsets from IFD0 to collect IFD1, IFD2 and later IFDs, and must stop at offset 0, at an invalid IFD, at a repeated offset or after 16 IFDs.
- R-ARW-13: The decoder must use as the raw IFD the first SubIFD (in the order of the offsets in tag 0x014A) that is a valid IFD and has NewSubfileType 0 or no NewSubfileType tag.
- R-ARW-14: The decoder must treat an IFD as invalid in each of these cases:
  1. The offset lies outside the file.
  2. N is 0 or greater than 1024.
  3. The 2 + 12 * N + 4 bytes of the IFD do not lie completely inside the file.

### Private Sony structures

- MakerNote (Exif tag 0x927C): in ARW 2.x and later it is an IFD without header. Its offsets count from the file start. In ARW 1.0 it starts with the 12 bytes "SONY DSC " and three zero bytes (M1). It contains FileFormat (0xB000) and many other tags. Some maker-note tags (0x2010, 0x9050, 0x94xx) are enciphered with a substitution cipher (S6). This document does not describe the cipher (Open questions OQ-2).
- SR2Private (IFD0 tag 0xC634): type BYTE, count 4. The four bytes are a little-endian 32-bit offset of a private IFD (M1). S6 names the tag SR2Private and states that it uses the DNGPrivateData tag number. Its tags are 0x7200 SR2SubIFDOffset, 0x7201 SR2SubIFDLength, 0x7221 SR2SubIFDKey (4 bytes), 0x7240 IDC_IFD, 0x7241 IDC2_IFD and 0x7250 MRWInfo (S6). The SR2SubIFD is encrypted (S6, M15). It holds BlackLevel, white balance levels, a ColorMatrix (0x7800) and WhiteLevel (0x787F) according to S6. This document does not describe the decryption (Open questions OQ-1).

- R-ARW-15: The decoder must not depend on the MakerNote, SR2Private, the SR2SubIFD or enciphered maker-note tags to decode the sensor data.

## Metadata tags

All tags below are in the raw IFD unless the column IFD says otherwise. "Required" means the variant cannot be decoded without the tag. The decoder then uses the preview path. "Default" is the value to use when an optional tag is absent or invalid.

| Tag | Name | Type | Count | IFD | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0x00FE | NewSubfileType | LONG | 1 | raw, IFD0 | 0 = full-resolution raw image, IFD0 has 1 (S1) | optional | 0 |
| 0x0100 | ImageWidth | SHORT or LONG | 1 | raw | stored width W in pixels (tile-padded in V4 and V5) | required | none |
| 0x0101 | ImageLength | SHORT or LONG | 1 | raw | stored height H in pixels (tile-padded in V4 and V5) | required | none |
| 0x0102 | BitsPerSample | SHORT | 1 or 3 | raw | 8, 12 or 14 (CFA), 15 15 15 (V5), informational | optional | none |
| 0x0103 | Compression | SHORT | 1 | raw | 1, 7 or 32767 (S5 calls 32767 "Sony ARW Compressed") | required | none |
| 0x0106 | PhotometricInterpretation | SHORT | 1 | raw | 32803 = CFA (S3), 6 = YCbCr (S1) | required | none |
| 0x010F | Make | ASCII | any | IFD0 | "SONY" | optional | empty |
| 0x0110 | Model | ASCII | any | IFD0 | model name, for example "ILCE-7M4" | optional | empty |
| 0x0111 | StripOffsets | SHORT or LONG | strips | raw | file offset of each strip | V1, V2, V3 | none |
| 0x0112 | Orientation | SHORT | 1 | IFD0 | 1 to 8, S1 definition | optional | 1 |
| 0x0115 | SamplesPerPixel | SHORT | 1 | raw | 1 (CFA), 3 (V5) | optional | 1 |
| 0x0116 | RowsPerStrip | SHORT or LONG | 1 | raw | rows in each strip | optional | H |
| 0x0117 | StripByteCounts | SHORT or LONG | strips | raw | bytes in each strip | V1, V2, V3 | none |
| 0x014A | SubIFDs | LONG or IFD | 1 or more | IFD0 | offsets of SubIFDs | required | none |
| 0x0142 | TileWidth | SHORT or LONG | 1 | raw | 512 in all measured files | V4, V5 | none |
| 0x0143 | TileLength | SHORT or LONG | 1 | raw | 512 in all measured files | V4, V5 | none |
| 0x0144 | TileOffsets | LONG | tiles | raw | file offset of each tile, in tile index order | V4, V5 | none |
| 0x0145 | TileByteCounts | SHORT or LONG | tiles | raw | bytes of each tile | V4, V5 | none |
| 0x0201 | JPEGInterchangeFormat (S5: PreviewImageStart in IFD0 of ARW) | LONG | 1 | IFD0, IFD1, IFD2 | file offset of a JPEG stream | optional | none |
| 0x0202 | JPEGInterchangeFormatLength | LONG | 1 | IFD0, IFD1, IFD2 | length of that JPEG stream | optional | none |
| 0x0211 | YCbCrCoefficients | RATIONAL | 3 | raw (V5) | LumaRed, LumaGreen, LumaBlue, measured 19595/65536, 38470/65536, 7471/65536 | optional | 299/1000, 587/1000, 114/1000 (S1) |
| 0x0212 | YCbCrSubSampling | SHORT | 2 | raw (V5) | horizontal and vertical chroma subsampling: 2 2 (M) or 2 1 (S) | optional | from the JPEG frame |
| 0x0213 | YCbCrPositioning | SHORT | 1 | raw (V5) | 2 (co-sited) | ignored | none |
| 0x0214 | ReferenceBlackWhite | RATIONAL | 6 | raw (V5) | measured 0, 16383, 16384, 16384, 16384, 16384, entries 3 and 5 give the chroma zero level | optional | 0, 16383, 16384, 16384, 16384, 16384 |
| 0x7000 | SonyRawFileType | SHORT | 1 | raw | see Storage variants (S5), diagnostic only | optional | none |
| 0x7010 | SonyToneCurve | SHORT | 4 | raw | T1, T2, T3, T4 of the V3 tone curve (S5 names the tag, meaning M5, S7) | V3: optional | 8000, 10400, 12900, 14100 |
| 0x7038 | SonyRawImageSize | LONG | 2 | raw (V4, V5) | valid width VW and height VH inside the tile-padded raster (S5) | optional | W, H |
| 0x7039 | (no public name) | RATIONAL | 3 | raw (V5) | white balance correction factors for R, G, B (Open questions OQ-7) | optional | 1, 1, 1 |
| 0x7310 | BlackLevel | SHORT | 4 | raw | black level for CFA positions 0, 1, 2, 3 (S5 name, measured 512 or 800) | optional | R-ARW-45 |
| 0x7313 | WB_RGGBLevels | SSHORT | 4 | raw | as-shot white balance levels R, G, G, B (S5 name). Green is 1024 in all measured files (M1) | optional | R-ARW-46 |
| 0x74C7 | SonyCropTopLeft | LONG | 2 | raw | origin of the aspect-ratio crop chosen in the camera (S5), not used | ignored | none |
| 0x74C8 | SonyCropSize | LONG | 2 | raw | size of that crop (S5), not used | ignored | none |
| 0x828D | CFARepeatPatternDim | SHORT | 2 | raw | rows, columns of the CFA repeat: 2, 2 | optional | 2, 2 |
| 0x828E | CFAPattern | BYTE | 4 | raw | colors of CFA positions 0 to 3, 0 = red, 1 = green, 2 = blue (S4), measured 0 1 1 2 | optional | 0, 1, 1, 2 |
| 0xC61D | WhiteLevel | SHORT or LONG | 1 | raw | saturation level on the 14-bit scale (S3), measured 16380 or 16383 | optional | 16383 |
| 0xC61F | DefaultCropOrigin | SHORT, LONG or RATIONAL | 2 | raw | x, y of the final image area in raw coordinates (S3) | optional | 0, 0 |
| 0xC620 | DefaultCropSize | SHORT, LONG or RATIONAL | 2 | raw | width, height of the final image area (S3) | optional | VW, VH |

Tags observed in the raw IFD and not used: 0x011A, 0x011B, 0x011C, 0x0128, 0x0153, 0x7001, 0x7011, 0x7020, 0x7031, 0x7032, 0x7034, 0x7035, 0x7036, 0x7037, 0x703A, 0x703B, 0x703C, 0x7316 (Open questions OQ-3, OQ-4).

- R-ARW-16: The decoder must use the preview path when a tag marked "required" for the classified variant is absent or has a count of 0.
- R-ARW-17: The decoder must treat a RATIONAL with denominator 0 as invalid and use the default of its tag.
- R-ARW-18: For V1 to V4, the decoder must accept a CFA pattern only if CFARepeatPatternDim is 2, 2 and the four CFAPattern values contain exactly one 0, two 1 and one 2, and must otherwise use the preview path.
- R-ARW-19: The decoder must use H as RowsPerStrip when tag 0x0116 is absent, is 0 or is greater than H.

## Sensor data layout

### Raster, valid area and crop

The raw raster has W columns and H rows. The valid area is the rectangle from (0, 0) with width VW and height VH, where VW and VH come from tag 0x7038 (V4, V5) or equal W and H. In V4 and V5 the raster is padded to whole tiles, and the padding holds data that is not image (M8). The final image area is the DefaultCrop rectangle (S3): origin (CX, CY) from tag 0xC61F and size (CW, CH) from tag 0xC620. S8 states that RAW images are always 3:2. DefaultCropSize is 3:2 in every measured file, while SonyCropSize (0x74C8) follows the aspect ratio setting of the camera JPEG (for example 5328x4000 on an ILCE-7C file, M1).

Measured dimensions (M1):

| File | Variant | W x H | VW x VH | CX, CY | CW x CH |
| --- | --- | --- | --- | --- | --- |
| ILCE-7M4 Compressed (S13) | V3 | 7040 x 4688 | same | 12, 8 | 7008 x 4672 |
| ILCE-7M4 Uncompressed (S13) | V1 | 7040 x 4688 | same | 12, 8 | 7008 x 4672 |
| ILCE-7M4 Lossless L (S13) | V4 | 7168 x 5120 | 7032 x 4688 | 12, 8 | 7008 x 4672 |
| ILCE-7M4 Lossless M (S13) | V5 | 5120 x 3584 | 4624 x 3080 | 8, 4 | 4608 x 3072 |
| ILCE-7M4 Lossless S (S13) | V5 | 3584 x 2560 | 3516 x 2344 | 6, 4 | 3504 x 2336 |
| ILCE-7RM5 Lossless L (S14) | V4 | 9728 x 6656 | 9568 x 6376 | 32, 20 | 9504 x 6336 |
| ILCE-7M3 Compressed (S17) | V3 | 6048 x 4024 | same | 12, 12 | 6000 x 4000 |
| DSLR-A700 (S20) | V3 | 4288 x 2856 | same | no tag | no tag |
| NEX-5 (S19) | V3 | 4608 x 3072 | same | no tag | no tag |
| DSLR-A900 (S21) | V2 | 6080 x 4048 | same | no tag | no tag |

- R-ARW-20: The decoder must use (VW, VH) from tag 0x7038 when the tag is present with 1 <= VW <= W and 1 <= VH <= H, and (W, H) otherwise.
- R-ARW-21: The decoder must use the DefaultCrop rectangle when both tags 0xC61F and 0xC620 are present, CW >= 16, CH >= 16, CX + CW <= VW and CY + CH <= VH, and must otherwise use the whole valid area.
- R-ARW-22: The decoder must not apply SonyCropTopLeft and SonyCropSize.

### CFA arrangement

The CFA pattern repeats every 2 rows and 2 columns. Its origin is the stored pixel (0, 0): S3 places the origin at the top-left of the active area, and ARW has no ActiveArea tag. CFAPattern lists the colors of CFA positions 0, 1, 2, 3, which are (x even, y even), (x odd, y even), (x even, y odd), (x odd, y odd). This is the row-major order of S4 (Figure 20 and Table 14 of S4: value 0 red, 1 green, 2 blue). All measured ARW files have 0 1 1 2 (red at even x and even y). M4 and M7 confirm by channel means that positions 1 and 2 are green.

- R-ARW-23: The decoder must give pixel (x, y) the color CFAPattern[2 * (y mod 2) + (x mod 2)], with the origin of the pattern at stored pixel (0, 0) and independent of the crop origin.

### Strips (V1, V2, V3)

All measured files have one strip with RowsPerStrip = H. Row y starts at byte offset StripOffset + y * R, where R is the row size in bytes: R = 2 * W (V1), R = 3 * W / 2 (V2), R = W (V3). There is no padding between rows.

- R-ARW-24: The decoder must treat the strips as consecutive groups of RowsPerStrip rows (the last strip can hold fewer rows), each strip starting at its StripOffsets value, with the row size R of the variant.

### Tiles (V4, V5)

The raster is cut into tiles of TileWidth (TW) by TileLength (TL) pixels (S1 section 15). TilesAcross = ceil(W / TW) and TilesDown = ceil(H / TL). Tile index t = r * TilesAcross + c covers columns c * TW to c * TW + TW - 1 and rows r * TL to r * TL + TL - 1. TileOffsets and TileByteCounts list the tiles in index order. The tiles are not stored in index order in the file (M8).

- R-ARW-25: The decoder must require TW = 512 and TL = 512, W and H multiples of 512, and exactly TilesAcross * TilesDown entries in TileOffsets and in TileByteCounts.
- R-ARW-26: The decoder must place tile t at tile column t mod TilesAcross and tile row floor(t / TilesAcross), using the offset of entry t, and must not assume that tile data are stored in index order.

## Compression

This section specifies, for each variant, how the stored bytes become sensor values. The CFA variants V1 to V4 produce one value per pixel on the 14-bit scale. V5 produces Y, Cb and Cr planes, which the subsection V5 converts to RGB.

### V1: uncompressed, 16-bit container

Each pixel is a 16-bit unsigned little-endian integer. Pixel (x, y) is at byte offset StripStart(y) + 2 * x, where StripStart(y) is the start of row y from R-ARW-24. The value is already on the 14-bit scale in both 14-bit and 12-bit camera modes (M6).

- R-ARW-27: The decoder must read V1 pixel (x, y) as b0 + 256 * b1, where b0 and b1 are the bytes at offsets StripStart(y) + 2 * x and StripStart(y) + 2 * x + 1.

Worked example: bytes 0xD0 0x09 give 0xD0 + 256 * 0x09 = 208 + 2304 = 2512.

### V2: 12-bit packed

Each row holds 3 * W / 2 bytes. Pixels 2k and 2k + 1 of a row come from the three bytes b0, b1, b2 at row offset 3k. The 24-bit little-endian group G = b0 + 256 * b1 + 65536 * b2 holds the first pixel in bits 0 to 11 and the second pixel in bits 12 to 23 (M7). The values are 12-bit linear. The decoder multiplies them by 4 to reach the 14-bit scale (M7: the darkest values near 130 become 520, near the black level 512 of the other variants).

- R-ARW-28: The decoder must compute pixel 2k of a V2 row as b0 + 256 * (b1 mod 16) and pixel 2k + 1 as floor(b1 / 16) + 16 * b2, and must multiply both by 4.

Worked example (first bytes of the S21 strip): 0x88 0x40 0x09 0x87 0xD0 0x09.
- Group 1: b0 = 0x88, b1 = 0x40, b2 = 0x09. Pixel 0 = 136 + 256 * 0 = 136. Pixel 1 = 4 + 16 * 9 = 148.
- Group 2: b0 = 0x87, b1 = 0xD0, b2 = 0x09. Pixel 2 = 135 + 256 * 0 = 135. Pixel 3 = 13 + 144 = 157.
- On the 14-bit scale: 544, 592, 540, 628.

### V3: 128-bit block format and tone curve

Sources: S7 gives the block fields, their widths and their order, the step table, the tone curve and the processing order. M2 to M5 give the bit order, the pixel order, the shift rule at the boundaries and the curve parameters.

#### Rows, chunks and blocks

Each row of W pixels is W bytes. The row is cut into chunks of 32 bytes. Chunk j covers columns 32j to 32j + 31. Each chunk holds two blocks of 16 bytes. The first block (bytes 32j to 32j + 15 of the row) holds the 16 pixels of the even columns 32j, 32j + 2, ..., 32j + 30. The second block (bytes 32j + 16 to 32j + 31) holds the 16 pixels of the odd columns 32j + 1, 32j + 3, ..., 32j + 31. With the pattern 0 1 1 2, a block of an even row holds 16 red or 16 green pixels and a block of an odd row holds 16 green or 16 blue pixels (S7, M4).

- R-ARW-29: The decoder must require W mod 32 = 0 for V3.
- R-ARW-30: The decoder must place pixel i (0 to 15) of the first block of chunk j at column 32j + 2i and pixel i of the second block at column 32j + 2i + 1.

#### Bit layout of a block

Read the 16 bytes of a block as one unsigned 128-bit integer B in little-endian order: B = sum over k = 0 to 15 of byte[k] * 2^(8k). The field with start bit p and width n has the value floor(B / 2^p) mod 2^n (M2).

| Start bit p | Width n | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 11 | MAX | largest code value in the block |
| 11 | 11 | MIN | smallest code value in the block |
| 22 | 4 | IMAX | pixel index (0 to 15) that holds MAX |
| 26 | 4 | IMIN | pixel index (0 to 15) that holds MIN |
| 30 + 7 * (k - 1) | 7 | D(k), k = 1 to 14 | delta of the k-th remaining pixel |

D(1) occupies bits 30 to 36 and D(14) occupies bits 121 to 127. The sizes add up to 11 + 11 + 4 + 4 + 14 * 7 = 128 bits (S7).

- R-ARW-31: The decoder must extract MAX, MIN, IMAX, IMIN and D(1) to D(14) from the little-endian 128-bit integer B of each block with the start bits and widths of the table above.

#### Shift and reconstruction

SPAN = MAX - MIN if MAX >= MIN, otherwise 0. The shift SH is the smallest value of 0, 1, 2, 3, 4 for which floor(SPAN / 2^SH) < 128. The step is 2^SH.

| SPAN | SH | Step |
| --- | --- | --- |
| 0 to 127 | 0 | 1 |
| 128 to 255 | 1 | 2 |
| 256 to 511 | 2 | 4 |
| 512 to 1023 | 3 | 8 |
| 1024 to 2047 | 4 | 16 |

This is the table of S7 ("data span less than 128: step 1", and so on). M3 confirms that a span of exactly 128 uses step 2.

The 16 code values of the block are assigned in pixel index order i = 0 to 15, with a delta counter k that starts at 1:
1. If i = IMAX, the code value is MAX.
2. Otherwise, if i = IMIN, the code value is MIN.
3. Otherwise, if k <= 14, the code value is MIN + D(k) * 2^SH, and k increases by 1.
4. Otherwise (only possible when IMAX = IMIN), the code value is MIN.
5. A code value above 2047 becomes 2047.

The camera rounds the deltas, so a reconstructed value can exceed MAX by up to half a step (M3). The decoder keeps such values (only the limit 2047 applies).

- R-ARW-32: The decoder must compute SH as the smallest value of 0 to 4 with floor(SPAN / 2^SH) < 128, with SPAN = MAX - MIN when MAX >= MIN and SPAN = 0 otherwise.
- R-ARW-33: The decoder must assign the 16 code values of a block by steps 1 to 5 above, in pixel index order.

#### Tone curve

The block values are code values. S7 states that the camera compresses the linear sensor values with a tone curve to 11-bit values and that the converter applies the linearization curve from the file metadata before it subtracts the black level. Tag 0x7010 (SonyToneCurve) holds four values T1, T2, T3, T4 (M5). The curve is piecewise linear in the code value c. Its knots are at the code values T1 / 8, T2 / 8, T3 / 8 and T4 / 8. Its slopes in the five segments are 2, 4, 8, 16 and 32 output units per code unit.

Evidence for knots and slopes:
- S7 figure 10 marks the points (linear, code) = (2000, 1000), (3200, 1300), (5696, 1612), (8092, 1762) and (17204, 2047).
- S7 figure 13 lists the code ranges 256 to 1000, 1001 to 1300, 1301 to 1612, 1613 to 1762 and 1763 to 2047. It gives the steps 2, 4, 8, 16 and 32 and the outputs after subtraction of the black level 512.
- Knots at T / 8 for T = 8000, 10400, 12900, 14100 are 1000, 1300, 1612.5 and 1762.5. They reproduce every number of both figures exactly.
- Integer knots (1612, 1762) do not reproduce the figures. They give 8096 and 17216 in place of 8092 and 17204.

Define T0 = 0. For a code value c, let S(c) = sum over i = 0 to 4 of 2^(i + 1) * max(0, min(8c, T(i + 1)) - T(i)), where T(5) is infinity (the last segment has no upper end). The linear value is L(c) = floor(S(c) / 8).

For the standard curve T = 8000, 10400, 12900, 14100 this gives:

| Code value c | L(c) |
| --- | --- |
| 0 to 1000 | 2c |
| 1000 to 1300 | 2000 + 4 * (c - 1000) |
| 1300 to 1612 | 3200 + 8 * (c - 1300) |
| 1613 to 1762 | 16c - 20100 |
| 1763 to 2047 | 32c - 48300 |

Check values: L(256) = 512, L(1612) = 5696, L(1613) = 5708, L(1762) = 8092, L(1763) = 8116, L(2047) = 17204. These are the numbers of S7 figure 13 after adding the black level 512 (for example 5196 + 512 = 5708 and 16692 + 512 = 17204).

The NEX-5 file (S19) has T = 0, 9824, 11512, 16362. The same formula maps its darkest code values (near 128) to 512, the black level of the other variants (M5).

- R-ARW-34: The decoder must use T1 to T4 from tag 0x7010 when the tag has 4 values with T1 <= T2 <= T3 <= T4 and T4 > 0, and must use 8000, 10400, 12900, 14100 otherwise.
- R-ARW-35: The decoder must convert each V3 code value c to the 14-bit-scale value L(c) = floor(S(c) / 8) with S(c) as defined above.

#### Worked example (vector TV-3 of section Test vectors)

Block bytes: E8 23 5C B1 82 E2 41 C9 78 46 A8 76 0C 08 08 06.
- MAX = bits 0 to 10 = byte0 + 256 * (byte1 mod 8) = 0xE8 + 256 * 3 = 232 + 768 = 1000.
- MIN = bits 11 to 21 = floor(byte1 / 8) + 32 * (byte2 mod 64) = 4 + 32 * 28 = 900.
- IMAX = bits 22 to 25 = floor(byte2 / 64) + 4 * (byte3 mod 4) = 1 + 4 * 1 = 5.
- IMIN = bits 26 to 29 = floor(byte3 / 4) mod 16 = 44 mod 16 = 12.
- D(1) = bits 30 to 36 = floor(byte3 / 64) + 4 * (byte4 mod 32) = 2 + 4 * 2 = 10.
- The remaining deltas are D(2) to D(14) = 20, 30, 40, 50, 60, 70, 80, 90, 99, 0, 1, 2, 3.
- SPAN = 100, so SH = 0 and the step is 1.
- Code values for i = 0 to 15: 910, 920, 930, 940, 950, 1000 (IMAX), 960, 970, 980, 990, 999, 900, 900 (IMIN), 901, 902, 903.
- With the standard curve (all values are in the segment 0 to 1000, L = 2c): 1820, 1840, 1860, 1880, 1900, 2000, 1920, 1940, 1960, 1980, 1998, 1800, 1800, 1802, 1804, 1806.
- If this is the first block of chunk 0 of row 0, these values go to columns 0, 2, 4, ..., 30.

### V4: lossless, CFA tiles

Each tile is one complete LJ92 stream (S2 Annex H): SOI, SOF3, DHT, SOS, entropy-coded data, EOI (M8). The frame header has P = 14, X = TW / 2 = 256, Y = TL / 2 = 256 and Nf = 4 components, each with H = 1 and V = 1. The scan has Ns = 4, the predictor selection value Ss = 1 and the point transform Pt = 0 in all measured tiles. Each MCU holds one sample of each component, so each frame sample position (u, v) (u = column, v = row of the frame) carries four values. These four values are one 2x2 CFA cell (M8):

| Component (order in the frame) | Tile pixel |
| --- | --- |
| 1 | (2u, 2v) |
| 2 | (2u + 1, 2v) |
| 3 | (2u, 2v + 1) |
| 4 | (2u + 1, 2v + 1) |

The decoded values are on the 14-bit scale. TileByteCounts can include 0xFF fill bytes after EOI: some M8 tiles end with FF FF FF FF, and M13 shows the same for previews.

- R-ARW-36: The decoder must decode each V4 and V5 tile as an LJ92 stream by S2 Annex H (Huffman tables by S2 Annex C, difference decoding by S2 F.2.2.1 and Table H.2, predictors by S2 Table H.1, reconstruction modulo 2^16, output shifted left by Pt), except for the line-start rule R-ARW-39.
- R-ARW-37: The decoder must require for V4 a frame with P = 14, X = 256, Y = 256, Nf = 4, every H = 1 and V = 1, and a scan that contains all four components.
- R-ARW-38: The decoder must store the V4 sample of component 1, 2, 3, 4 at frame position (u, v) in tile pixel (2u, 2v), (2u + 1, 2v), (2u, 2v + 1), (2u + 1, 2v + 1) respectively.

Worked example: vector TV-9 decodes to component values R: 8192, 8194, 8192, 8193. G1: 8193, 8193, 8194, 8194. G2: 8191, 8187, 8191, 8189. B: 8195, 8195, 8196, 8196 at frame positions (0,0), (1,0), (0,1), (1,1). The 4x4 tile corner is:

| | x=0 | x=1 | x=2 | x=3 |
| --- | --- | --- | --- | --- |
| y=0 | 8192 | 8193 | 8194 | 8193 |
| y=1 | 8191 | 8195 | 8187 | 8195 |
| y=2 | 8192 | 8194 | 8193 | 8194 |
| y=3 | 8191 | 8196 | 8189 | 8196 |

### V5: lossless, YCbCr tiles (Lossless M and S)

Each tile is one LJ92 stream with P = 16, X = TW = 512, Y = TL = 512 and Nf = 3 (M9). Component 1 is Y with H = 2 and V = 2 (size M) or V = 1 (size S). Components 2 and 3 are Cb and Cr with H = 1 and V = 1. Huffman table 0 codes Y. Table 1 codes Cb and Cr. The predictor is 1 and Pt = 0. Inside a tile the Y plane has 512 x 512 samples and each chroma plane has 256 x (512 / V) samples. Each MCU holds 2 x V samples of Y (row by row: top-left, top-right, then the second row if V = 2), then one Cb and one Cr sample (S2 A.2.3).

#### Line-start prediction in V5

S2 H.1.2.1 predicts the first sample of each line, except the first line, from the sample above (Rb). The Sony encoder does this for every line except the first line of an MCU row of a component with V = 2. For that line it predicts the first sample from the first sample two lines above (M9). The rule does not change any other prediction.

- R-ARW-39: For a component with vertical sampling factor V >= 2, the decoder must predict the first sample (x = 0) of a component line y with y mod V = 0 and y >= V from the sample at (0, y - V) of the same component, and must use S2 H.1.2.1 for all other samples.
- R-ARW-40: The decoder must require for V5 a frame with all of these properties:
  1. P = 16, X = 512, Y = 512, Nf = 3.
  2. Component 1 has H = 2 and V = 1 or V = 2.
  3. Components 2 and 3 have H = 1 and V = 1.
  4. YCbCrSubSampling, when present, is 2 and V.

Worked example (vector TV-10): a 2-column, 4-row frame with Y (H = 2, V = 2). The first Y sample of line 2 is coded as the difference +4 from the sample (0, 0) = 32770, so its value is 32774. The T.81 rule predicts from (0, 1) = 32769 and gives 32773. Every later Y value of lines 2 and 3 is then 1 too small.

#### Chroma placement

The chroma sample (cx, cy) of a tile belongs to the luma pixels with x = 2cx or x = 2cx + 1 and y from V * cy to V * cy + V - 1. The files declare YCbCrPositioning 2 (co-sited, S1 section 21).

- R-ARW-41: The decoder must give each luma pixel (x, y) of a V5 tile the chroma samples (floor(x / 2), floor(y / V)) of the same tile.

#### YCbCr to RGB

The conversion uses the formulas of S1 section 21 (YCbCrCoefficients) with LumaRed LR, LumaGreen LG and LumaBlue LB from tag 0x0211. The chroma zero level Z is 16384 (ReferenceBlackWhite entries 3 and 5, M9). The chroma scale is 1, because the ReferenceBlackWhite chroma range of the files (16384 to 16384) is empty and M10 confirms unit scale. Let Cb' = Cb - Z and Cr' = Cr - Z.

- R = Y + (2 - 2 * LR) * Cr'
- B = Y + (2 - 2 * LB) * Cb'
- G = (Y - LB * B - LR * R) / LG

The result is camera RGB that the camera has already white balanced, with a black pedestal of 1024 (M10).

- R-ARW-42: The decoder must convert V5 samples to R, G, B with the three formulas above, with LR, LG, LB from tag 0x0211 (default 0.299, 0.587, 0.114) and Z from ReferenceBlackWhite entries 3 and 5 (default 16384).

Worked example: Y = 5000, Cb = 16484, Cr = 16184, LR = 19595/65536 = 0.2989959717, LG = 38470/65536 = 0.5870056152, LB = 7471/65536 = 0.1139984131.
- Cb' = 100, Cr' = -200.
- 2 - 2 * LR = 1.4020080566. R = 5000 - 200 * 1.4020080566 = 4719.5983887.
- 2 - 2 * LB = 1.7720031738. B = 5000 + 100 * 1.7720031738 = 5177.2003174.
- G = (5000 - 0.1139984131 * 5177.2003174 - 0.2989959717 * 4719.5983887) / 0.5870056152 = (5000 - 590.1926 - 1411.1409) / 0.5870056152 = 5108.4119.

## Levels and white balance

The decoder determines the white level first and the black levels second.

### White level

- R-ARW-43: The decoder must use the value of tag 0xC61D as the white level w when the tag is present and the value is from 4096 to 65535, and 16383 otherwise.

V3 values can exceed the white level. The curve reaches 17204, and M5 shows codes up to 2029 (L = 16628). R-ARW-47 clips these values.

### Black level

- R-ARW-44: The decoder must use the four values of tag 0x7310 as the black levels of CFA positions 0, 1, 2, 3 when the tag has 4 values and each value is less than w - 255.
- R-ARW-45: When tag 0x7310 is absent or invalid, the decoder must use the black level 800 for every CFA position if the Model begins with "DSC-RX0" or "DSC-RX10", and 512 otherwise.

The prefix "DSC-RX10" covers the DSC-RX10 and DSC-RX100 families (1-inch sensors). The value 800 comes from M11. The value 512 comes from M5, M6 and M7 (the darkest values of DSLR-A700, A900 and NEX-5, and the tag value of all newer files). Both defaults are less than 4096 - 255, so they are valid for every accepted white level.

### White balance multipliers (CFA variants V1 to V4)

The decoder computes three multipliers mR, mG, mB. Sources in priority order:
1. Tag 0x7313 WB_RGGBLevels (R, G1, G2, B) with all four values greater than 0: G = (G1 + G2) / 2, mR = R / G, mG = 1, mB = B / G.
2. The D50 white balance scales of the Model in the table of section Color conversion (S9): mR and mB from the table, mG = 1.
3. The generic scales mR = 2.512, mG = 1, mB = 1.678 (mean of the table, section Color conversion).

- R-ARW-46: The decoder must take the WB multipliers from the first available source of the list above.

Worked example: WB_RGGBLevels 2429, 1024, 1024, 1783 (S13 Compressed) give G = 1024, mR = 2429 / 1024 = 2.3720703125, mB = 1783 / 1024 = 1.7412109375.

### Normalization of CFA values

For a CFA pixel with 14-bit-scale value s, black level b of its CFA position, white level w and multiplier m of its color:
- n = (s - b) / (w - b)
- v = n * m, clipped to the range 0 to 1.

- R-ARW-47: The decoder must compute v = clip((s - b) / (w - b) * m, 0, 1) for every CFA pixel before demosaicing.

Worked example: a CFA cell with R = 2512, G1 = 4512, G2 = 4512, B = 1512, b = 512, w = 16383, multipliers from the example above:
- R: (2512 - 512) / 15871 * 2.3720703 = 0.1260160 * 2.3720703 = 0.2989188.
- G: (4512 - 512) / 15871 * 1 = 0.2520320.
- B: (1512 - 512) / 15871 * 1.7412109 = 0.0630080 * 1.7412109 = 0.1097102.

### Levels of V5

V5 values are already white balanced. They carry a pedestal of 1024 and the same range as the raw values (M10). Tag 0x7039 holds three ratios that are 1 or close to 1 (for example 2425/2429, 1024/1024, 1816/1802 in S13 Lossless M, where 2425 and 1816 equal tag 0x7313). This document treats them as correction factors (Open questions OQ-7).

- R-ARW-48: The decoder must compute for each V5 channel value c (R, G or B from R-ARW-42) the value v = clip((c - 1024) / (16383 - b) * f, 0, 1), where b is BlackLevel position 0 (default 512) and f is the ratio of tag 0x7039 for that channel (default 1. A ratio outside 0.5 to 2 counts as 1).
- R-ARW-49: The decoder must not apply WB multipliers to V5 data.

Worked example: R = G = B = 5000 (fixture F5): v = (5000 - 1024) / 15871 = 0.2505198 for each channel.

## Color conversion

### Matrix sources

ARW files carry a color matrix only inside the encrypted SR2SubIFD (tag 0x7800, S6), which this document does not decode (OQ-1). The decoder therefore uses a per-model table. S9 (DXOMARK) publishes for each tested camera a "color matrix as defined in ISO standard 17321" for the CIE D50 and CIE A illuminants, together with white balance scales. The matrix converts white-balanced camera RGB to linear sRGB. Each row of the matrix is one output channel (R, G, B of sRGB). Each column is one camera channel (R, G, B of the camera). Two facts fix this orientation: the rows sum to 1, so a white-balanced neutral stays neutral, and M12 confirms the orientation against the camera JPEG. S9 prints row labels "R raw", "G raw", "B raw". The numbers in a printed row are the coefficients of one output channel.

How to read S9: open the page https://www.dxomark.com/Cameras/Sony/<name>---Measurements, tab "Color response". The page shows two blocks (CIE D50 and CIE A) in an order that varies between pages. The D50 block is the block with the larger ratio of the R scale to the B scale. The table below gives the D50 block of each listed model, read on 2026-11-13. Values are rounded to 2 decimals by S9.

| Model (IFD0 0x0110) | S9 name | WB R | WB B | m11 | m12 | m13 | m21 | m22 | m23 | m31 | m32 | m33 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ILCE-7 | A7 | 2.46 | 1.53 | 1.93 | -0.79 | -0.14 | -0.16 | 1.59 | -0.43 | 0.03 | -0.44 | 1.41 |
| ILCE-7R | A7R | 2.52 | 1.59 | 2.02 | -0.88 | -0.13 | -0.18 | 1.62 | -0.44 | 0.04 | -0.45 | 1.41 |
| ILCE-7S | A7S | 2.54 | 1.65 | 2.29 | -1.10 | -0.20 | -0.25 | 1.71 | -0.46 | 0.12 | -0.70 | 1.58 |
| ILCE-7M2 | A7-II | 2.34 | 1.54 | 1.99 | -0.91 | -0.08 | -0.16 | 1.56 | -0.41 | 0.06 | -0.52 | 1.47 |
| ILCE-7RM2 | A7R-II | 2.43 | 1.61 | 1.95 | -0.88 | -0.06 | -0.17 | 1.55 | -0.39 | 0.05 | -0.48 | 1.43 |
| ILCE-7SM2 | A7S-II | 2.47 | 1.63 | 2.37 | -1.25 | -0.11 | -0.24 | 1.69 | -0.45 | 0.16 | -0.79 | 1.64 |
| ILCE-7M3 | A7-III | 2.32 | 1.59 | 2.10 | -0.96 | -0.14 | -0.17 | 1.58 | -0.41 | 0.04 | -0.50 | 1.46 |
| ILCE-7RM3 | A7R-III | 2.37 | 1.63 | 2.01 | -0.91 | -0.11 | -0.18 | 1.57 | -0.39 | 0.04 | -0.52 | 1.48 |
| ILCE-7RM4 | A7R-IV | 2.52 | 1.71 | 1.97 | -0.85 | -0.11 | -0.17 | 1.61 | -0.44 | 0.04 | -0.46 | 1.42 |
| ILCE-7SM3 | A7S-III | 2.44 | 1.51 | 2.31 | -1.23 | -0.08 | -0.18 | 1.65 | -0.47 | 0.00 | -0.51 | 1.51 |
| ILCE-7M4 | A7-IV | 2.47 | 1.61 | 2.03 | -0.92 | -0.11 | -0.16 | 1.59 | -0.43 | 0.04 | -0.47 | 1.44 |
| ILCE-7RM5 | A7R-V | 2.53 | 1.62 | 1.95 | -0.87 | -0.08 | -0.15 | 1.58 | -0.43 | 0.03 | -0.45 | 1.41 |
| ILCE-7C | A7C | 2.47 | 1.57 | 2.26 | -1.19 | -0.07 | -0.17 | 1.63 | -0.47 | 0.00 | -0.44 | 1.44 |
| ILCE-9 | A9 | 2.40 | 1.65 | 2.06 | -0.92 | -0.14 | -0.18 | 1.59 | -0.41 | 0.04 | -0.46 | 1.43 |
| ILCE-9M2 | A9-II | 2.45 | 1.54 | 2.28 | -1.21 | -0.07 | -0.17 | 1.65 | -0.48 | -0.01 | -0.43 | 1.44 |
| ILCE-1 | A1 | 2.39 | 1.64 | 1.85 | -0.74 | -0.11 | -0.14 | 1.48 | -0.35 | 0.02 | -0.40 | 1.38 |
| ILCE-5000 | A5000 | 2.75 | 1.69 | 1.80 | -0.69 | -0.11 | -0.17 | 1.60 | -0.43 | 0.05 | -0.43 | 1.38 |
| ILCE-5100 | A5100 | 2.75 | 1.81 | 2.03 | -0.91 | -0.12 | -0.22 | 1.65 | -0.42 | 0.09 | -0.56 | 1.47 |
| ILCE-6000 | A6000 | 2.86 | 1.75 | 2.08 | -0.96 | -0.12 | -0.23 | 1.68 | -0.45 | 0.08 | -0.54 | 1.46 |
| ILCE-6300 | A6300 | 2.59 | 1.82 | 2.04 | -0.91 | -0.13 | -0.24 | 1.65 | -0.41 | 0.06 | -0.49 | 1.44 |
| ILCE-6400 | A6400 | 2.61 | 1.81 | 2.05 | -0.96 | -0.09 | -0.19 | 1.57 | -0.38 | 0.09 | -0.56 | 1.47 |
| ILCE-6500 | A6500 | 2.59 | 1.85 | 2.11 | -1.00 | -0.11 | -0.21 | 1.61 | -0.40 | 0.08 | -0.53 | 1.44 |
| ILCE-6600 | A6600 | 2.80 | 1.83 | 2.07 | -0.99 | -0.08 | -0.19 | 1.59 | -0.40 | 0.09 | -0.55 | 1.47 |
| NEX-3N | NEX-3N | 2.66 | 1.64 | 1.90 | -0.82 | -0.08 | -0.17 | 1.59 | -0.43 | 0.04 | -0.44 | 1.39 |
| NEX-5N | NEX-5N | 2.53 | 1.61 | 1.98 | -0.88 | -0.10 | -0.17 | 1.60 | -0.43 | 0.04 | -0.47 | 1.43 |
| NEX-5R | NEX-5R | 2.56 | 1.60 | 1.88 | -0.79 | -0.09 | -0.16 | 1.57 | -0.41 | 0.04 | -0.45 | 1.41 |
| NEX-5T | NEX-5T | 2.69 | 1.64 | 1.82 | -0.69 | -0.12 | -0.16 | 1.59 | -0.43 | 0.03 | -0.42 | 1.39 |
| NEX-6 | NEX-6 | 2.47 | 1.61 | 1.82 | -0.69 | -0.14 | -0.16 | 1.56 | -0.40 | 0.04 | -0.45 | 1.41 |
| NEX-7 | NEX-7 | 2.60 | 1.52 | 1.98 | -0.92 | -0.07 | -0.16 | 1.59 | -0.42 | 0.03 | -0.45 | 1.42 |
| SLT-A77V | SLT-Alpha-77 | 2.61 | 1.54 | 2.01 | -0.94 | -0.08 | -0.17 | 1.59 | -0.42 | 0.03 | -0.47 | 1.44 |
| SLT-A99V | SLT-Alpha-99 | 2.31 | 1.53 | 1.92 | -0.79 | -0.14 | -0.15 | 1.55 | -0.40 | 0.04 | -0.49 | 1.45 |
| DSC-RX100 | Cyber-shot-DSC-RX100 | 2.14 | 2.29 | 1.76 | -0.74 | -0.03 | -0.20 | 1.44 | -0.25 | 0.09 | -0.62 | 1.53 |
| DSC-RX100M3 | Cyber-shot-DSC-RX100-III | 2.43 | 1.87 | 1.82 | -0.76 | -0.06 | -0.20 | 1.51 | -0.31 | 0.08 | -0.71 | 1.62 |
| DSC-RX100M4 | Cyber-shot-DSC-RX100-IV | 2.69 | 1.90 | 1.89 | -0.82 | -0.06 | -0.21 | 1.54 | -0.34 | 0.08 | -0.65 | 1.57 |
| DSC-RX100M7 | Cyber-shot-DSC-RX100-VII | 2.22 | 1.75 | 1.90 | -0.80 | -0.11 | -0.18 | 1.62 | -0.44 | 0.04 | -0.57 | 1.52 |
| DSC-RX1RM2 | Cyber-shot-DSC-RX1R-II | 2.45 | 1.72 | 1.98 | -0.93 | -0.05 | -0.18 | 1.57 | -0.39 | 0.06 | -0.52 | 1.46 |

The model names in the first column are the IFD0 Model strings. They were measured in the files of S13 to S27, or are the folder names of the sample archive, which uses the IFD0 Model string.

Generic matrix for models that are not in the table: the element-wise mean of the 36 matrices above, rounded to 3 decimals:

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 2.006 | -0.906 | -0.101 |
| sRGB G | -0.182 | 1.592 | -0.412 |
| sRGB B | 0.052 | -0.511 | 1.459 |

The generic WB scales are the means of the table: R 2.512, B 1.678.

### Procedure

- R-ARW-50: The decoder must select the table row whose Model column equals the IFD0 Model string (compared by R-ARW-10), and must use the generic matrix when no row matches.
- R-ARW-51: The decoder must divide each row of the selected matrix by the sum of that row before use, so that every row sums to 1.
- R-ARW-52: The decoder must compute the output (r, g, b) from the white-balanced, normalized camera values (v_R, v_G, v_B) with the normalized matrix as follows, and must clip r, g and b to the range 0 to 1:
  - r = m11 * v_R + m12 * v_G + m13 * v_B
  - g = m21 * v_R + m22 * v_G + m23 * v_B
  - b = m31 * v_R + m32 * v_G + m33 * v_B
- R-ARW-53: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S12), without the sRGB transfer function.

Worked example: the camera values of section Levels and white balance (0.2989188, 0.2520320, 0.1097102) and the ILCE-7M4 matrix. Row sums are 1.00, 1.00 and 1.01. After normalization, row 3 is 0.0396040, -0.4653465, 1.4257426.
- r = 2.03 * 0.2989188 - 0.92 * 0.2520320 - 0.11 * 0.1097102 = 0.6068052 - 0.2318694 - 0.0120681 = 0.3628676.
- g = -0.16 * 0.2989188 + 1.59 * 0.2520320 - 0.43 * 0.1097102 = -0.0478270 + 0.4007309 - 0.0471754 = 0.3057285.
- b = 0.0396040 * 0.2989188 - 0.4653465 * 0.2520320 + 1.4257426 * 0.1097102 = 0.0118384 - 0.1172822 + 0.1564185 = 0.0509747.

A neutral value (0.25, 0.25, 0.25) stays (0.25, 0.25, 0.25) for every normalized matrix.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and read the header (R-ARW-65, R-ARW-4, R-ARW-5).
2. Read IFD0 and the IFD chain. Validate every IFD (R-ARW-11, R-ARW-12, R-ARW-14, R-ARW-6, R-ARW-7, R-ARW-8, R-ARW-9).
3. Collect the preview candidates now, so that every later failure can fall back (R-ARW-59, R-ARW-60).
4. Find the raw IFD (R-ARW-13). If IFD0 has no tag 0x014A or no SubIFD qualifies, use the preview path (R-ARW-2).
5. Classify the file (R-ARW-3, R-ARW-10). For X1 to X6 use the preview path. Check the required tags (R-ARW-16, R-ARW-17).
6. Determine the valid area and the crop (R-ARW-20, R-ARW-21, R-ARW-22), the CFA pattern (R-ARW-18, R-ARW-23), the white and black levels (R-ARW-43, R-ARW-44, R-ARW-45), the WB multipliers (R-ARW-46), the strip layout (R-ARW-19) and, for V3, the tone curve (R-ARW-34).
7. Check all sizes against the file and the limits before allocating memory (section Error handling).
8. Decode the sensor data:
   - V1: R-ARW-24, R-ARW-27.
   - V2: R-ARW-24, R-ARW-28.
   - V3: R-ARW-24, R-ARW-29, R-ARW-31, R-ARW-32, R-ARW-33, R-ARW-30, R-ARW-35.
   - V4: R-ARW-25, R-ARW-26, R-ARW-36, R-ARW-37, R-ARW-38.
   - V5: R-ARW-25, R-ARW-26, R-ARW-36, R-ARW-39, R-ARW-40, R-ARW-41, R-ARW-42.
9. Normalize:
   - V1 to V4: subtract the black level, scale, apply the WB multipliers and clip (R-ARW-47).
   - V5: subtract the pedestal, scale, apply tag 0x7039 and clip (R-ARW-48, R-ARW-49).
10. Demosaic V1 to V4 inside the valid area (R-ARW-54, R-ARW-55). V5 needs no demosaicing.
11. Convert to linear sRGB (R-ARW-50, R-ARW-51, R-ARW-52).
12. Crop to the final image area (R-ARW-21, R-ARW-56).
13. Apply the orientation (R-ARW-57, R-ARW-58).
14. Deliver the image (R-ARW-53). If any step from 4 to 13 fails, deliver the preview instead (R-ARW-63).

### Demosaicing

The decoder uses bilinear interpolation, the basic non-adaptive method described in S10. For each pixel and each color that the pixel lacks, the missing value is the mean of the pixels of that color among the 8 neighbors of the pixel. With a 2x2 Bayer pattern this rule gives these cases:
- Green at a red or blue pixel: the mean of the 4 orthogonal neighbors.
- Red or blue at a green pixel: the mean of the 2 row neighbors or of the 2 column neighbors, whichever pair has that color.
- Red at a blue pixel, and blue at a red pixel: the mean of the 4 diagonal neighbors.

Other demosaicing methods are outside this document. The test vectors assume this one.

- R-ARW-54: For each pixel (x, y) of the valid area, the decoder must set its own color to its normalized value, and each missing color to the arithmetic mean of the normalized values of the pixels of that color among the 8 neighbors (x + dx, y + dy), with dx and dy from -1 to 1 and not both 0.
- R-ARW-55: The decoder must ignore neighbors outside the valid area in that mean, and must use 0 when no neighbor of the missing color lies inside the valid area.
- R-ARW-56: The decoder must demosaic with the CFA phase of the stored raster and crop afterwards, so that the result equals demosaicing the whole valid area and then cropping.

Worked example: a pixel at (1, 0) with pattern 0 1 1 2 is green. Its 8 neighbors inside the area: (0, 0) red, (2, 0) red, (0, 1) green, (1, 1) blue, (2, 1) green. Red = mean of (0, 0) and (2, 0). Blue = (1, 1) alone, because (1, -1) is outside the area.

### Orientation

- R-ARW-57: The decoder must read Orientation from IFD0 tag 0x0112, use 1 when the tag is absent or outside 1 to 8, and apply it after the crop.
- R-ARW-58: The decoder must map stored pixel (x, y) of a W' x H' image (after the crop) to the output pixel of the table below, with an output of W' x H' pixels for values 1 to 4 and H' x W' pixels for values 5 to 8.

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

These mappings follow the definitions of S1. For example, S1 defines value 6 as "the 0th row represents the visual right-hand side of the image, and the 0th column represents the visual top". Stored row 0 therefore goes to the rightmost output column, and stored column 0 goes to the top output row.

## Embedded preview

### Locations

| Location | Tags | Content (M13) |
| --- | --- | --- |
| IFD0 | 0x0201 offset, 0x0202 length | JPEG preview, 1616x1080 in ARW 2.x and later (640x480 in ARW 1.0, 640x424 in SR2) |
| IFD1 | 0x0201, 0x0202 | 160x120 JPEG thumbnail, in SRF files the only JPEG |
| IFD2 (FileFormat 4.0.0.0 and later) | 0x0201, 0x0202 (with ImageWidth, ImageLength, Compression 7) | full-size JPEG, size = DefaultCropSize |

All preview streams are baseline JPEG (SOF0, 8-bit, 3 components) and are decoded with S2 (baseline sequential DCT process). They contain no APP1 (Exif) segment, so they carry no orientation of their own (M13).

### Requirements

- R-ARW-59: The decoder must collect as preview candidates the pairs (0x0201, 0x0202) of IFD0 and of every IFD of the chain, and must keep a candidate only if offset + length lies inside the file, length >= 4, and the first two bytes are FF D8.
- R-ARW-60: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment and must choose the candidate with the largest width * height.
- R-ARW-61: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan.
- R-ARW-62: The decoder must apply the IFD0 Orientation (R-ARW-58) to the preview, except that it must not apply values 5 to 8 when the preview is taller than wide.
- R-ARW-63: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 to X6, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-ARW-64: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-ARW-53) or a preview (sRGB-encoded JPEG values, not linear).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-ARW-63).

### Limits

- R-ARW-65: The decoder must fail for a file shorter than 8 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-ARW-66: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728).
- R-ARW-67: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values.
- R-ARW-68: The decoder must check, before reading, that every value, strip and tile byte range lies completely inside the file.

### Structure checks

- R-ARW-69: The decoder must fail in each of these cases:
  1. The sum of StripByteCounts is less than the row size R times H.
  2. StripOffsets and StripByteCounts have different counts.
  3. The strip count is not ceil(H / RowsPerStrip).
  4. A strip holds fewer bytes than R times its number of rows.
- R-ARW-70: The decoder must classify a file with Compression 32767 by the exact byte counts of R-ARW-3 and must never read more than W * H bytes (V3) or 3 * W * H / 2 bytes (V2).
- R-ARW-71: The decoder must fail when the number of TileOffsets or TileByteCounts entries differs from TilesAcross * TilesDown.
- R-ARW-72: The decoder must fail on a tile stream with any of these faults:
  1. The stream does not start with FF D8.
  2. SOF3, DHT or SOS is missing before the entropy data.
  3. The frame or the scan differs from R-ARW-37 or R-ARW-40.
  4. The scan refers to an undefined Huffman table.
  5. The predictor is outside 1 to 7, or Pt >= P.
- R-ARW-73: The decoder must fail on a DHT whose 16 counts sum to more than 256 or to more than the bytes left in the segment, or whose codes overflow the code space of their length (S2 Annex C).
- R-ARW-74: The decoder must fail when a Huffman code is not in the table, when SSSS is greater than 16, or when a marker other than RST appears inside the entropy data before all samples of the tile are decoded.
- R-ARW-75: The decoder must not read bytes outside a tile's byte range, and must fail when a Huffman code or its extra bits need bits beyond the entropy-coded data of the tile (at a marker or at the end of the range), while look-ahead bits that it does not consume must not cause a failure.
- R-ARW-76: The decoder must handle DRI and RST markers by S2 (reset of the predictors at each restart), although no measured tile contains them.
- R-ARW-77: The decoder must clip every decoded sensor value to the range 0 to 65535 of its storage and every normalized value to 0 to 1 (R-ARW-47, R-ARW-48).
- R-ARW-78: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check selects the default of the quantity.

### Behaviour on error

- R-ARW-79: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-ARW-80: The decoder must discard a partly decoded raw image on failure and must not deliver it.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order.

### TV-1: TIFF header

Bytes 0x49 0x49 0x2A 0x00 0x08 0x00 0x00 0x00: little-endian TIFF, IFD0 at offset 8 (R-ARW-4). Bytes 0x4D 0x4D 0x00 0x2A: big-endian, X6 (preview path).

### TV-2: IFD entries

- Entry 03 01 03 00 01 00 00 00 FF 7F 00 00: tag 0x0103 (Compression), type 3 (SHORT), count 1, value 0x7FFF = 32767 in the first two bytes of the value field.
- Entry 10 70 03 00 04 00 00 00 14 01 00 00: tag 0x7010, type SHORT, count 4 (8 bytes > 4), so the values are at offset 0x114.
- Entry 8E 82 01 00 04 00 00 00 00 01 01 02: tag 0x828E, type BYTE, count 4, inline values 0, 1, 1, 2.
- Entry 13 73 08 00 04 00 00 00 24 01 00 00 with the bytes 7D 09 00 04 00 04 F7 06 at offset 0x124: tag 0x7313, SSHORT, values 2429, 1024, 1024, 1783.

### TV-3: V3 block, shift 0

Input: 0xE8 0x23 0x5C 0xB1 0x82 0xE2 0x41 0xC9 0x78 0x46 0xA8 0x76 0x0C 0x08 0x08 0x06.
Fields: MAX 1000, MIN 900, IMAX 5, IMIN 12, SH 0, D = 10 20 30 40 50 60 70 80 90 99 0 1 2 3.
Code values (i = 0 to 15): 910 920 930 940 950 1000 960 970 980 990 999 900 900 901 902 903.
Linear (standard curve): 1820 1840 1860 1880 1900 2000 1920 1940 1960 1980 1998 1800 1800 1802 1804 1806.

### TV-4: V3 block, shift 3

Input: 0xA4 0x66 0x22 0x3C 0xA0 0xA0 0x78 0x50 0x32 0x9E 0x11 0xAA 0x25 0xBB 0xF1 0x94.
Fields: MAX 1700, MIN 1100, IMAX 0, IMIN 15, SPAN 600, SH 3 (step 8), D = 0 5 10 15 20 25 30 35 40 45 50 55 60 74.
Code values: 1700 1100 1140 1180 1220 1260 1300 1340 1380 1420 1460 1500 1540 1580 1692 1100.
Linear (standard curve): 7100 2400 2560 2720 2880 3040 3200 3520 3840 4160 4480 4800 5120 5440 6972 2400.
Check of two values: L(1700) = 16 * 1700 - 20100 = 7100. L(1692) = 16 * 1692 - 20100 = 6972.

### TV-5: V3 block, shift 4 and the 2047 limit

Input: 0xFF 0x47 0xDF 0xC3 0x1F 0x08 0x08 0x08 0x06 0x84 0x82 0xE1 0x80 0x48 0x28 0x16.
Fields: MAX 2047, MIN 1000, IMAX 15, IMIN 0, SPAN 1047, SH 4 (step 16), D = 127 64 0 1 2 3 4 5 6 7 8 9 10 11.
Code values: 1000 (IMIN), 2047 (1000 + 127 * 16 = 3032, limited to 2047), 2024, 1000, 1016, 1032, 1048, 1064, 1080, 1096, 1112, 1128, 1144, 1160, 1176, 2047 (IMAX).

### TV-6: V3 blocks from a camera file

From S13 Compressed, row 2000, chunk starting at column 3200 (file offset 7659520 + 2000 * 7040 + 3200).
- First block: 0xE0 0xA9 0x4A 0xC3 0xC1 0xB4 0xFB 0x30 0x16 0x02 0x4C 0x51 0xC7 0xEB 0x0D 0x7F. Fields MAX 480, MIN 341, IMAX 13, IMIN 0, SH 1. Codes: 341 355 417 459 403 365 363 345 389 479 457 461 463 480 475 467. Linear: 682 710 834 918 806 730 726 690 778 958 914 922 926 960 950 934 (columns 3200, 3202, ..., 3230).
- Second block: 0x79 0xFB 0x50 0x03 0xE2 0x06 0x5C 0x18 0x02 0x88 0xDA 0x12 0xBA 0x7C 0x36 0xAD. Fields MAX 889, MIN 543, IMAX 13, IMIN 0, SH 2. Codes: 543 575 763 799 587 567 547 575 755 843 863 843 859 889 851 887. Linear: 1086 1150 1526 1598 1174 1134 1094 1150 1510 1686 1726 1686 1718 1778 1702 1774 (columns 3201, 3203, ..., 3231).

### TV-7: tone curve

Standard curve T = 8000, 10400, 12900, 14100: c -> L(c): 0 -> 0, 255 -> 510, 256 -> 512, 257 -> 514, 1000 -> 2000, 1001 -> 2004, 1300 -> 3200, 1301 -> 3208, 1612 -> 5696, 1613 -> 5708, 1762 -> 8092, 1763 -> 8116, 2021 -> 16372, 2022 -> 16404, 2047 -> 17204.
NEX-5 curve T = 0, 9824, 11512, 16362: 128 -> 512, 1228 -> 4912, 1439 -> 6600, 2046 -> 16324, 2047 -> 16356.

### TV-8: V1 and V2 samples

- V1: 0xD0 0x09 -> 2512. 0xA0 0x11 -> 4512. 0xE8 0x05 -> 1512.
- V2: 0x88 0x40 0x09 0x87 0xD0 0x09 -> 136, 148, 135, 157 -> 14-bit scale 544, 592, 540, 628.
- V2: 0x74 0x82 0x46 -> 628, 1128 -> 2512, 4512. 0x68 0xA4 0x17 -> 1128, 378 -> 4512, 1512.

### TV-9: LJ92 CFA stream (V4 rules, small frame)

A complete stream with P = 14, X = 2, Y = 2, Nf = 4. The frame is smaller than a real tile, so R-ARW-37 does not apply to this vector:

FF D8
FF C3 00 14 0E 00 02 00 02 04 01 11 00 02 11 00 03 11 00 04 11 00
FF C4 00 17 00 00 03 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 02 03
FF DA 00 0E 04 01 00 02 00 03 00 04 00 01 00 00
1A BA 33 06 36 49
FF D9

Huffman table 0: BITS = 0, 3, 1, 0, ... (three codes of length 2, one of length 3), HUFFVAL = 0, 1, 2, 3. Codes (S2 Annex C): SSSS 0 = 00, SSSS 1 = 01, SSSS 2 = 10, SSSS 3 = 110.
Decoding (initial prediction 2^(14 - 0 - 1) = 8192):

| MCU (u, v) | Component | Prediction | Bits | Difference | Value |
| --- | --- | --- | --- | --- | --- |
| (0, 0) | 1 | 8192 (first sample) | 00 | 0 | 8192 |
| (0, 0) | 2 | 8192 | 01 1 | +1 | 8193 |
| (0, 0) | 3 | 8192 | 01 0 | -1 | 8191 |
| (0, 0) | 4 | 8192 | 10 11 | +3 | 8195 |
| (1, 0) | 1 | 8192 (left) | 10 10 | +2 | 8194 |
| (1, 0) | 2 | 8193 (left) | 00 | 0 | 8193 |
| (1, 0) | 3 | 8191 (left) | 110 011 | -4 | 8187 |
| (1, 0) | 4 | 8195 (left) | 00 | 0 | 8195 |
| (0, 1) | 1 | 8192 (above) | 00 | 0 | 8192 |
| (0, 1) | 2 | 8193 (above) | 01 1 | +1 | 8194 |
| (0, 1) | 3 | 8191 (above) | 00 | 0 | 8191 |
| (0, 1) | 4 | 8195 (above) | 01 1 | +1 | 8196 |
| (1, 1) | 1 | 8192 (left) | 01 1 | +1 | 8193 |
| (1, 1) | 2 | 8194 (left) | 00 | 0 | 8194 |
| (1, 1) | 3 | 8191 (left) | 10 01 | -2 | 8189 |
| (1, 1) | 4 | 8196 (left) | 00 | 0 | 8196 |

The 47 bits are followed by one padding 1-bit. The resulting 4x4 CFA corner is the table in section Compression, V4.

### TV-10: LJ92 YCbCr stream with the Sony line-start rule

P = 16, X = 2, Y = 4, Nf = 3, component 1 H = 2 V = 2, components 2 and 3 H = 1 V = 1. One Huffman table (the table of TV-9) for all components:

FF D8
FF C3 00 11 10 00 04 00 02 03 01 22 00 02 11 00 03 11 00
FF C4 00 17 00 00 03 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 02 03
FF DA 00 0C 03 01 00 02 00 03 00 01 00 00
A6 AC 5A 26 E6 BF
FF D9

| Sample | Prediction | Bits | Difference | Value |
| --- | --- | --- | --- | --- |
| Y(0,0) | 32768 (first) | 10 10 | +2 | 32770 |
| Y(1,0) | 32770 (left) | 01 1 | +1 | 32771 |
| Y(0,1) | 32770 (above) | 01 0 | -1 | 32769 |
| Y(1,1) | 32769 (left) | 10 11 | +3 | 32772 |
| Cb(0,0) | 32768 (first) | 00 | 0 | 32768 |
| Cr(0,0) | 32768 (first) | 01 0 | -1 | 32767 |
| Y(0,2) | 32770 = Y(0,0), Sony rule | 110 100 | +4 | 32774 |
| Y(1,2) | 32774 (left) | 01 0 | -1 | 32773 |
| Y(0,3) | 32774 (above) | 01 1 | +1 | 32775 |
| Y(1,3) | 32775 (left) | 01 1 | +1 | 32776 |
| Cb(0,1) | 32768 (above) | 10 01 | -2 | 32766 |
| Cr(0,1) | 32767 (above) | 10 10 | +2 | 32769 |

Here Y(x, y) is column x, line y. Expected planes: Y lines 32770 32771 / 32769 32772 / 32774 32773 / 32775 32776. Cb 32768 / 32766. Cr 32767 / 32769. A decoder that uses the plain T.81 rule gets lines 2 and 3 as 32773 32772 / 32774 32775 and fails this vector.

### TV-11: YCbCr to RGB and V5 levels

Y = 5000, Cb = 16484, Cr = 16184, coefficients 19595/65536, 38470/65536, 7471/65536: R = 4719.5984, G = 5108.4119, B = 5177.2003. With pedestal 1024 and black 512: v = 0.2328523, 0.2573506, 0.2616849 before clipping and before tag 0x7039.

### TV-12: levels, white balance and matrix

CFA cell 2512, 4512, 4512, 1512. Black 512. White 16383. WB_RGGBLevels 2429 1024 1024 1783. Model ILCE-7M4. Normalized and balanced: 0.2989188, 0.2520320, 0.1097102. Output linear sRGB for a flat area of this cell (every pixel of a flat area gets the same three values after bilinear demosaicing): 0.3628676, 0.3057285, 0.0509747.

### TV-13: orientation

Stored 3 x 2 image with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a.

### Fixture recipes

The fixtures are complete small ARW files for the decoder tests. All use the layout below and differ in the raw IFD and the data. All multi-byte numbers are little-endian. Tags are written in ascending order (the decoder must also accept unsorted tags, R-ARW-7).

Common part:
- Offset 0x00: 49 49 2A 00 08 00 00 00.
- Offset 0x08: IFD0 with 4 entries: 0x010F Make ASCII count 5 at 0x3E ("SONY" and a zero byte). 0x0110 Model ASCII count 9 at 0x44 ("ILCE-7M4" and a zero byte). 0x0112 Orientation SHORT 1. 0x014A SubIFDs LONG 1 = 0x4E. Next-IFD offset 0.
- Offset 0x3E: 53 4F 4E 59 00 00 (with one pad byte). Offset 0x44: 49 4C 43 45 2D 37 4D 34 00 00.
- Offset 0x4E: the raw IFD. Values longer than 4 bytes follow the raw IFD, each at an even offset. The sensor data start at the next multiple of 16.
- Common raw tags of F1 to F4 (5 entries): 0x7310 BlackLevel SHORT 4 = 512 512 512 512. 0x7313 WB_RGGBLevels SSHORT 4 = 2429 1024 1024 1783. 0x828D SHORT 2 = 2 2. 0x828E BYTE 4 = 0 1 1 2. 0xC61D WhiteLevel SHORT 1 = 16383.
- Expected output for F1 to F4: every output pixel equals 0.3628676, 0.3057285, 0.0509747 (TV-12), output size equals W x H (no crop tags, orientation 1).

F3 (V3, 32 x 2 pixels), complete file of 368 bytes:

0000: 49 49 2A 00 08 00 00 00 04 00 0F 01 02 00 05 00
0010: 00 00 3E 00 00 00 10 01 02 00 09 00 00 00 44 00
0020: 00 00 12 01 03 00 01 00 00 00 01 00 00 00 4A 01
0030: 04 00 01 00 00 00 4E 00 00 00 00 00 00 00 53 4F
0040: 4E 59 00 00 49 4C 43 45 2D 37 4D 34 00 00 10 00
0050: FE 00 04 00 01 00 00 00 00 00 00 00 00 01 03 00
0060: 01 00 00 00 20 00 00 00 01 01 03 00 01 00 00 00
0070: 02 00 00 00 02 01 03 00 01 00 00 00 0E 00 00 00
0080: 03 01 03 00 01 00 00 00 FF 7F 00 00 06 01 03 00
0090: 01 00 00 00 23 80 00 00 11 01 04 00 01 00 00 00
00A0: 30 01 00 00 15 01 03 00 01 00 00 00 01 00 00 00
00B0: 16 01 03 00 01 00 00 00 02 00 00 00 17 01 04 00
00C0: 01 00 00 00 40 00 00 00 10 70 03 00 04 00 00 00
00D0: 14 01 00 00 10 73 03 00 04 00 00 00 1C 01 00 00
00E0: 13 73 08 00 04 00 00 00 24 01 00 00 8D 82 03 00
00F0: 02 00 00 00 02 00 02 00 8E 82 01 00 04 00 00 00
0100: 00 01 01 02 1D C6 03 00 01 00 00 00 FF 3F 00 00
0110: 00 00 00 00 40 1F A0 28 64 32 14 37 00 02 00 02
0120: 00 02 00 02 7D 09 00 04 00 04 F7 06 00 00 00 00
0130: 68 44 23 04 00 00 00 00 00 00 00 00 00 00 00 00
0140: B8 C5 2D 04 00 00 00 00 00 00 00 00 00 00 00 00
0150: B8 C5 2D 04 00 00 00 00 00 00 00 00 00 00 00 00
0160: F4 A2 17 04 00 00 00 00 00 00 00 00 00 00 00 00

Annotations: raw IFD at 0x4E with 16 entries (0x00FE, 0x0100 = 32, 0x0101 = 2, 0x0102 = 14, 0x0103 = 32767, 0x0106 = 32803, 0x0111 = 0x130, 0x0115 = 1, 0x0116 = 2, 0x0117 = 64, 0x7010 at 0x114, 0x7310 at 0x11C, 0x7313 at 0x124, 0x828D, 0x828E, 0xC61D). Next-IFD offset 0 at 0x110. Each data block has MAX = MIN = c, IMAX = 0, IMIN = 1 and all deltas 0, so all 16 pixels have the code value c. Row 0: first block c = 1128 (bytes 68 44 23 04, red, L = 2512), second block c = 1464 (B8 C5 2D 04, green, L = 4512). Row 1: c = 1464 (green, 4512), then c = 756 (F4 A2 17 04, blue, L = 1512).

F1 (V1, 4 x 2 pixels), complete file of 304 bytes. The raw IFD has 15 entries: the entries of F3 without 0x7010, with Compression 1, ImageWidth 4, StripOffsets 0x120 and StripByteCounts 16.

0000: 49 49 2A 00 08 00 00 00 04 00 0F 01 02 00 05 00
0010: 00 00 3E 00 00 00 10 01 02 00 09 00 00 00 44 00
0020: 00 00 12 01 03 00 01 00 00 00 01 00 00 00 4A 01
0030: 04 00 01 00 00 00 4E 00 00 00 00 00 00 00 53 4F
0040: 4E 59 00 00 49 4C 43 45 2D 37 4D 34 00 00 0F 00
0050: FE 00 04 00 01 00 00 00 00 00 00 00 00 01 03 00
0060: 01 00 00 00 04 00 00 00 01 01 03 00 01 00 00 00
0070: 02 00 00 00 02 01 03 00 01 00 00 00 0E 00 00 00
0080: 03 01 03 00 01 00 00 00 01 00 00 00 06 01 03 00
0090: 01 00 00 00 23 80 00 00 11 01 04 00 01 00 00 00
00A0: 20 01 00 00 15 01 03 00 01 00 00 00 01 00 00 00
00B0: 16 01 03 00 01 00 00 00 02 00 00 00 17 01 04 00
00C0: 01 00 00 00 10 00 00 00 10 73 03 00 04 00 00 00
00D0: 08 01 00 00 13 73 08 00 04 00 00 00 10 01 00 00
00E0: 8D 82 03 00 02 00 00 00 02 00 02 00 8E 82 01 00
00F0: 04 00 00 00 00 01 01 02 1D C6 03 00 01 00 00 00
0100: FF 3F 00 00 00 00 00 00 00 02 00 02 00 02 00 02
0110: 7D 09 00 04 00 04 F7 06 00 00 00 00 00 00 00 00
0120: D0 09 A0 11 D0 09 A0 11 A0 11 E8 05 A0 11 E8 05

F2 (V2, 4 x 2 pixels): F1 with these changes: 0x0102 BitsPerSample = 12 (byte 0x7C = 0C), 0x0103 Compression = 32767 (bytes 0x88 to 0x89 = FF 7F), 0x0117 StripByteCounts = 12 (byte 0xC4 = 0C), and the 12 data bytes at 0x120: 74 82 46 74 82 46 68 A4 17 68 A4 17. File length 300 bytes.

F4 (V4, one 512 x 512 tile). The raw IFD has 17 entries: the 5 common raw tags and these 12 entries.

| Tag | Type | Count | Value |
| --- | --- | --- | --- |
| 0x00FE | LONG | 1 | 0 |
| 0x0100 | SHORT | 1 | 512 |
| 0x0101 | SHORT | 1 | 512 |
| 0x0102 | SHORT | 1 | 14 |
| 0x0103 | SHORT | 1 | 7 |
| 0x0106 | SHORT | 1 | 32803 |
| 0x0115 | SHORT | 1 | 1 |
| 0x0142 | SHORT | 1 | 512 |
| 0x0143 | SHORT | 1 | 512 |
| 0x0144 | LONG | 1 | file offset of the tile |
| 0x0145 | LONG | 1 | 32841 |
| 0x7038 | LONG | 2 | 512 512 |

The tile (32841 bytes):
- Header: FF D8 FF C3 00 14 0E 01 00 01 00 04 01 11 00 02 11 00 03 11 00 04 11 00 FF C4 00 16 00 01 01 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0C 0D FF DA 00 0E 04 01 00 02 00 03 00 04 00 01 00 00 (P = 14, X = Y = 256, Nf = 4. Table: SSSS 0 = 0, SSSS 12 = 10, SSSS 13 = 110).
- Entropy data, 32775 bytes: C9 CF 86 7E 19 FC 5E 70, then 32767 bytes 00.
- FF D9.
The first MCU codes the differences -5680 (R, SSSS 13), -3680 (G1, SSSS 12), -3680 (G2), -6680 (B, SSSS 13) from 8192. All other differences are 0 (one 0-bit each). Expected sensor values: every cell R 2512, G 4512, G 4512, B 1512.

F5 (V5, one 512 x 512 tile, size M). The raw IFD has these 19 entries:

| Tag | Type | Count | Value |
| --- | --- | --- | --- |
| 0x00FE | LONG | 1 | 0 |
| 0x0100 | SHORT | 1 | 512 |
| 0x0101 | SHORT | 1 | 512 |
| 0x0102 | SHORT | 3 | 15 15 15 |
| 0x0103 | SHORT | 1 | 7 |
| 0x0106 | SHORT | 1 | 6 |
| 0x0115 | SHORT | 1 | 3 |
| 0x0142 | SHORT | 1 | 512 |
| 0x0143 | SHORT | 1 | 512 |
| 0x0144 | LONG | 1 | file offset of the tile |
| 0x0145 | LONG | 1 | 49220 |
| 0x0211 | RATIONAL | 3 | 19595/65536 38470/65536 7471/65536 |
| 0x0212 | SHORT | 2 | 2 2 |
| 0x0214 | RATIONAL | 6 | 0/1 16383/1 16384/1 16384/1 16384/1 16384/1 |
| 0x7038 | LONG | 2 | 512 512 |
| 0x7310 | SHORT | 4 | 512 512 512 512 |
| 0x7313 | SSHORT | 4 | 2429 1024 1024 1783 |
| 0x828D | SHORT | 2 | 2 2 |
| 0x828E | BYTE | 4 | 255 255 255 255 |

The tile (49220 bytes):
- Header: FF D8 FF C3 00 11 10 02 00 02 00 03 01 22 00 02 11 00 03 11 00 FF C4 00 15 00 01 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0F FF DA 00 0C 03 01 00 02 00 03 00 01 00 00 (P = 16, X = Y = 512, Y with H = 2, V = 2. Table: SSSS 0 = 0, SSSS 15 = 10. All components use table 0).
- Entropy data, 49160 bytes: 89 C3 89 FF 00 FC FF 00 FC, then 49151 bytes 00 (FF 00 is a stuffed FF).
- FF D9.
The first MCU codes Y -27768 (5000), Cb -16384 (16384), Cr -16384 (16384). All other differences are 0. Expected: Y 5000, Cb 16384, Cr 16384 everywhere. RGB 5000 5000 5000. Normalized value v = 0.2505198 per channel. Output linear sRGB 0.2505198 0.2505198 0.2505198.

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 SR2SubIFD decryption. S6 states that the SR2SubIFD is encrypted with a key (tag 0x7221) and lists its tags (black level, white balance, ColorMatrix 0x7800, WhiteLevel 0x787F). No permitted public document describes the algorithm. Fallback: R-ARW-15. Levels and white balance come from the raw IFD tags or the defaults. The color matrix comes from the S9 table.
- OQ-2 Enciphered maker-note tags (0x2010, 0x9050, 0x94xx). S6 names a substitution cipher but does not describe it. Fallback: not used. Files before FileFormat 3.3.3.0 therefore use the table or generic WB scales (R-ARW-46), not the as-shot white balance.
- OQ-3 Meaning of tags 0x7001, 0x7011, 0x7020, 0x7316, 0x703A, 0x703B, 0x703C. Measured values: 0x7001 = 1 (0 in SR2). 0x7011 = 4000 7200 10050 12075 in files with the standard curve and 0 4912 8212 12287 in NEX-5. 0x7020 = 0. 0x7316 = 128. Fallback: ignored.
- OQ-4 Lens correction tags 0x7031 to 0x7037 (S5 names vignetting, chromatic aberration and distortion correction). Their parameter semantics are not documented. Fallback: no lens correction.
- OQ-5 Tone curve rounding. S7 shows fractional knots (1612.5, 1762.5), which this document follows. A curve with knots rounded down differs by at most 12 units near code 1613 and by 12 units at code 2047. Fallback: R-ARW-35 as written.
- OQ-6 V5 pedestal and gain. The pedestal 1024 and the unit gain are measured on two models (ILCE-7M4, ILCE-7RM5). The pedestal is not in any tag. BlackLevel is 512 in these files. Fallback: R-ARW-48 with 1024.
- OQ-7 Tag 0x7039. Its numerators equal WB_RGGBLevels R and B in S13 Lossless M. The denominators are close values. The meaning "correction from the white balance used by the camera to the recorded white balance" is an inference. The effect is below 1 percent in all measured files. Fallback: R-ARW-48 applies the ratios.
- OQ-8 V5 line-start rule. Measured on size M of two models (V = 2). Size S (V = 1) needs no special rule. Fallback: R-ARW-39 for every component with V >= 2.
- OQ-9 Default black levels. 512 and 800 are measured on few models without tag 0x7310 (DSLR-A700, A900, NEX-5, DSC-RX100, RX100M3, RX0). Other old models can differ (for example DSC-RX1). Fallback: R-ARW-45.
- OQ-10 Order of the four BlackLevel values for patterns other than 0 1 1 2. Not observed. Fallback: by CFA position (R-ARW-44).
- OQ-11 CFA patterns other than 0 1 1 2. Not observed in ARW (SR2 has 1 0 2 1). Fallback: any 2x2 Bayer pattern is accepted (R-ARW-18).
- OQ-12 Crop of files without DefaultCrop tags (FileFormat before 3.3.3.0). The pixels outside the camera JPEG area hold plausible data (M16). The exact camera crop is unknown. Fallback: the whole valid area (R-ARW-21).
- OQ-13 Orientation of the embedded preview for rotated shots. No rotated sample was measured. Fallback: R-ARW-62.
- OQ-14 SonyRawFileType 3 and 6. Value 3 means the X4 coding in DSLR-A200 family files, while S5 calls it "Lossless Compressed RAW". Value 6 ("Compressed RAW 2", S5) was not measured. Fallback: classification by structure (R-ARW-3). Unknown structures use the preview path.
- OQ-15 FileFormat 5.0.1.0 and 6.0.0.0 files were not measured. Fallback: R-ARW-3.
- OQ-16 Big-endian ARW files were not observed. Fallback: preview path (R-ARW-4).
- OQ-17 LJ92 precision other than 14 (V4) or 16 (V5), tile sizes other than 512, strip-based LJ92. Not observed. Fallback: preview path (R-ARW-37, R-ARW-40, R-ARW-25).
- OQ-18 Several strips in V1 to V3. Not observed. Fallback: R-ARW-24.
- OQ-19 Color matrices. S9 gives D50 matrices with 2 decimals for 36 models and nothing for others (for example ILCE-7RM3A, ILCE-7RM4A, ILCE-6700, ILCE-9M3, ZV-E1, ILCE-7CM2, DSLR-A models). The D50 matrix is used for every scene illuminant. Fallback: R-ARW-50 with the generic matrix.
- OQ-20 SonyCropTopLeft and SonyCropSize describe the aspect-ratio crop of the camera JPEG. In some files SonyCropTopLeft x is 0 where DefaultCropOrigin x is 32. Fallback: ignored (R-ARW-22).
- OQ-21 Highlights. V3 values above the white level and white-balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-ARW-47.
- OQ-22 White level of files without tag 0xC61D (FileFormat before 3.3.3.0, V2). Saturation was not measured. Fallback: 16383 (R-ARW-43).
- OQ-23 X2, X3 and X4 sensor data (SR2, ARW 1.0, ARW 2.x variable-length coding). No permitted source describes them. Fallback: preview path.
- OQ-24 ARQ pixel-shift composites (S27 lists one file, 325 MB, not measured). Fallback: X5, preview path.
