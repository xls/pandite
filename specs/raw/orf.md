# Olympus ORF raw format: decoder specification

Document: specs/raw/orf.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite ORF decoder (clean-room step 1 of 2).

This document describes the ORF raw file format of Olympus and OM Digital Solutions (OM System) cameras as facts. It is written for an implementer who has only this document, the Sony specification specs/raw/arw.md (for the parts that this document reuses by reference), TIFF 6.0 (S1) and ITU-T T.81 (S2, for the embedded JPEG preview only). Every normative statement traces to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Conventions

- Requirements have the form R-ORF-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- A reference to R-ARW-n means the requirement of that number in specs/raw/arw.md. Where this document cites such a requirement, it applies to ORF unchanged, except for the differences that the citing requirement states.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x49 0x49 or "49 49".
- Bit 0 is the least significant bit of a value. "MSB-first" means that bit 7 of a byte is read first and bit 0 last.
- floor(a / b) is division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0). For example floor(-7 / 32) = -1 and floor(-32 / 32) = -1.
- bitlength(v) of an integer v >= 0 is the number of binary digits of v without leading zeros, and bitlength(0) = 0. Examples: bitlength(1) = 1, bitlength(16) = 5, bitlength(743) = 10.
- |v| is the absolute value of v. max and min take the larger and the smaller of their arguments.
- Pixel coordinates are (x, y): x is the column, y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (IFD0 tags 0x0100 and 0x0101).

### Glossary

- ORF: the raw file format of Olympus and OM System cameras. ORF is TIFF-like: it uses the TIFF 6.0 file structure (S1) with other values in header bytes 2 and 3.
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2). IFD0 is the IFD at the offset in header bytes 4 to 7.
- Raw IFD: the IFD that describes the sensor data. In ORF it is IFD0.
- Exif IFD: the IFD at the offset in IFD0 tag 0x8769 (S3).
- Maker note: the value of Exif tag 0x927C. In ORF it holds a header and the main maker-note IFD.
- Maker-note sub-IFDs: the ImageProcessing IFD (main maker-note tag 0x2040), the CameraSettings IFD (tag 0x2020) and the RawInfo IFD (tag 0x3000). The names are those of S4.
- Note style: one of the three maker-note header forms OLYMP, OLYMPUS and OM SYSTEM (section Container layout).
- B: the bit depth of the decoded sensor values (10, 12 or 14). The white level is 2^B - 1.
- CFA: color filter array. All measured ORF files have a 2x2 Bayer pattern (S9).
- CFA position: the index 2 * (y mod 2) + (x mod 2) of a pixel inside its 2x2 repeat cell (0, 1, 2 or 3).
- Variant: one storage method of the sensor data. This document defines six variants, V1 to V6, in the section Scope and variants.
- Parity: x mod 2 of a pixel. The compressed variants keep separate coding state for the two parities of a row.
- Code: the group of bits that encodes one pixel in V5 and V6.
- Escape: the long form of a code. A code uses it when its quotient is 12 or more.
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S7). The decoder output is linear sRGB.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Container layout (header, IFD, field types), Metadata tags (IFD0 tags), Decoding procedure (Orientation definitions) |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Embedded preview (baseline JPEG decoding, marker structure) |
| S3 | Exif Version 2.32, CIPA DC-X008-Translation-2019 (tag CFAPattern 0xA302, Figure 20, Table 14) | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-14 | Metadata tags (Exif IFD, CFAPattern layout and color codes), Sensor data layout |
| S4 | ExifTool Tag Names: Olympus tags table, including the Equipment, CameraSettings, ImageProcessing and RawInfo tables (metadata tag reference documentation) | https://exiftool.org/TagNames/Olympus.html | community documentation | 2026-11-14 | Metadata tags (tag numbers and names), Levels and white balance, Color conversion, Embedded preview |
| S5 | T. Ishikawa (Olympus Imaging Corp.), Data encoding apparatus, data decoding apparatus, data encoding method, data decoding method, and recording medium recorded with program, US Patent 7,333,035 B2 (2008) | https://patents.google.com/patent/US7333035B2/en | patent | 2026-11-14 | Compression (prediction formulas and line-start rules, sign and magnitude, Golomb-Rice code, run counter, dynamic range, line reset) |
| S6 | T. Ishikawa (Olympus Imaging Corp.), Data encoding apparatus, data decoding apparatus, data encoding method, data decoding method, and recording medium recorded with program, US Patent 7,715,639 B2 (2010) | https://patents.google.com/patent/US7715639B2/en | patent | 2026-11-14 | Compression (low-order bits coded apart, recursive correction filter, k from flatness and dynamic range) |
| S7 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (output primaries and D65 white), measurement tools (inverse transfer function of the preview) |
| S8 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear demosaicing, as in arw.md) |
| S9 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-14 | Sensor data layout (Bayer arrangement) |
| S10 | Measurement: container survey, first 1 MiB of all 85 public-domain (CC0) ORF files of 63 models in the archive (makes Olympus, OLYMPUS, OLYMPUS IMAGING CORP., OM System, OM Digital Solutions), plus the first 16 bytes of the sensor strip of each compressed file. License of each file from the archive index https://raw.pixls.us/json/getrepository.php?set=all | https://raw.pixls.us/data/Olympus/ | measurement | 2026-11-14 | Scope and variants, Container layout, Metadata tags, Embedded preview, records M1 to M4, M19 |
| S11 | Measurement: V1 samples E-10 P7198607.ORF (big-endian), E-1 E_1__C106743_gredos.ORF and E-400 _A270187.ORF (CC0) | https://raw.pixls.us/data/Olympus/E-10/P7198607.ORF | measurement | 2026-11-14 | Compression (V1), Levels, records M5, M16 to M18 |
| S12 | Measurement: V2 samples E-500 _1010010.ORF, E-330 P3307182.ORF, and the first 16000000 bytes of E-M5 Mark II CB252215.ORF and PEN-F PenFHiRes.orf (high-resolution shots) (CC0) | https://raw.pixls.us/data/Olympus/E-500/_1010010.ORF | measurement | 2026-11-14 | Compression (V2), record M6 |
| S13 | Measurement: V3 samples C5060WZ (file name begins C5060WZ_5_7_22_8mm), C7070WZ PC270085.ORF, SP510UZ PA210583.ORF, SP565UZ 2019-09-26--10.03.53001.ORF (CC0) | https://raw.pixls.us/data/Olympus/C5060WZ/ | measurement | 2026-11-14 | Compression (V3), Sensor data layout (fields), record M7 |
| S14 | Measurement: V4 samples XZ-2 p7166537.orf, SH-2 PC260009.ORF, XZ-10 P1240016.ORF (CC0) | https://raw.pixls.us/data/Olympus/XZ-2/p7166537.orf | measurement | 2026-11-14 | Compression (V4), Decoding procedure (orientation 6), records M8, M20 |
| S15 | Measurement: V5 sample E-3 E_3__3099164_casco_antiguo_petrer.ORF (CC0), used for every hypothesis test of the compressed format | https://raw.pixls.us/data/Olympus/E-3/E_3__3099164_casco_antiguo_petrer.ORF | measurement | 2026-11-14 | Compression (V5), records M9 to M12 |
| S16 | Measurement: V5 samples E-510 Olympus_E510-4.ORF, E-5 _7061961_copy.ORF, E-PL1 PB120976.ORF, E-P1 P1080385.ORF, E-M5 P2082705.ORF, E-M1 sc000877.orf, E-PM2 P1010001.ORF, E-M10 Mark II (file name begins 2022-04-15T17_31_58), E-M1MarkII Olympus_EM1mk2_Standard_20MP.ORF, OM-1 OM_Systems_OM1_RAW_Image_20mpx_P1010025.ORF (CC0) | https://raw.pixls.us/data/Olympus/E-M1MarkII/Olympus_EM1mk2_Standard_20MP.ORF | measurement | 2026-11-14 | Compression (V5), Levels and white balance, Color conversion, records M12, M15 to M18 |
| S17 | Measurement: V6 sample OM-1 Mark II 50mp-14bit-handheld.orf (CC0), and the first 16 strip bytes of OM-1 Mark II 80mp-14bit-tripod.orf, OM-3 P5048185.ORF and OM-3 P5048186.ORF | https://raw.pixls.us/data/OM%20System/OM-1%20Mark%20II/50mp-14bit-handheld.orf | measurement | 2026-11-14 | Compression (V6), Levels (14-bit black scale), records M13, M15 |

Sample files are from the public raw sample archive raw.pixls.us. The archive index marks each file with its license. Only files marked CC0 (public domain) were used. Eight Olympus files of the archive carry another license (C5050Z, C8080WZ, E-20, E-30, E-520, E-600, SP350, SP500UZ) and were not downloaded. The author inspected the files with measurement tools that the author wrote from S1 to S7 for this purpose. The tools are not part of the deliverable. One comparison decoded the embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S10): every one of the 85 files starts with 49 49 52 4F (78 files), 49 49 52 53 (6 files, the V3 cameras) or 4D 4D 4F 52 (E-10), followed by the IFD0 offset 8. IFD0 holds ImageWidth, ImageLength, BitsPerSample, Compression 1, the strip tags and the Exif IFD pointer, and no SubIFDs tag. In 6 OLYMP-style files (E-10, E-1, E-300, E-330, E-400, E-500) the next-IFD offset of IFD0 leads to an IFD1 with a 160x120 JPEG (tags 0x0201, 0x0202). In all other files it is 0.
- M2 (maker note, S10): the maker note starts with "OLYMP" 00 (12 files: E-10, E-1, E-300, E-330, E-400, E-500 and the 6 V3 cameras), "OLYMPUS" 00 "II" 03 00 (53 files) or "OM SYSTEM" 00 00 00 "II" 04 00 (20 files). OLYMPUS and OM SYSTEM style: the sub-IFD pointers 0x2010 to 0x2050 have type 13 and count 1. Read relative to the maker-note start, they give valid IFDs (E-3: Equipment IFD at 3344 + 114 with 23 entries and version "0100"), and PreviewImageStart points to FF D8 in all 73 files. Read as file offsets, PreviewImageStart points to FF D8 in none of them. OLYMP style: the sub-IFD entries have type 7 and hold the sub-IFD at their value offset. The ThumbnailImage offset of E-10 equals the IFD1 JPEG offset 6424, and PreviewImageStart of E-1, E-300, E-330, E-400 and E-500 points to FF D8 only as a file offset.
- M3 (storage classes, S10 to S17): the total strip bytes T and the first strip bytes classify all 85 files by R-ORF-3: V1 3 files (T = 2 * W * H exactly), V2 5 files (T = 1.6 * W * H, W multiple of 10), V3 6 files and V4 3 files (T = 1.5 * W * H, BitsPerSample 12), V5 64 files, V6 4 files, X1 none. The V5 and V6 files have T from 0.75 to 1.10 bytes per pixel. All W are even, all V4 W are multiples of 8.
- M4 (tag types, S10): types and counts of the tags of section Metadata tags in all files: IFD0 ImageWidth and ImageLength LONG, BitsPerSample SHORT. CFAPattern UNDEFINED count 8 (85 files). ImageProcessing in 78 files with WB_RBLevels SHORT count 4 (71 files) or 2 (7 files), ColorMatrix SHORT count 9, BlackLevel2 SHORT 4, GainBase SHORT 1 (value 256 in all 78), ValidBits SHORT 2, CropLeft and CropTop SHORT 2, CropWidth and CropHeight LONG 1. RawInfo in 5 files with CropLeft and CropTop SHORT 1. Main maker-note ColorMatrix, BlackLevel, RedBalance, BlueBalance and ValidBits in the 12 OLYMP-style files.
- M5 (V1, S11): reading the E-10 samples big-endian gives a same-color roughness (mean absolute difference of pixels two columns apart) of 377, little-endian gives 20418. E-1 little-endian: 1024, big-endian: 21783. Low bits: in E-10 all but 3 of 3799104 samples are multiples of 64 (B = 10), in E-1 all 5158784 samples are multiples of 16 and in E-400 all but 793 of 10701120 (B = 12). After R-ORF-22 the E-1 maximum is 4095 (10 samples) and the darkest E-10 values are 30 to 33 next to BlackLevel 32.
- M6 (V2, S12): every row of E-500 is 336 groups of 16 bytes. Byte 15 of every group is 0 (841344 groups in E-500, 905728 in the E-M5 Mark II part, 905840 in the PEN-F part). Little-endian 12-bit pairs give a roughness of 23.7 (E-500), 9.1 (E-M5 Mark II) and 30.8 (PEN-F). The MSB-first reading gives 324, 72 and 349.
- M7 (V3, S13): MSB-first pairs give a roughness of 55.8 in C5060WZ, the little-endian reading 586. In stored order, vertically adjacent stored rows are more alike than stored rows two apart (67.5 against 93.7), which a Bayer raster cannot show. With the first ceil(H / 2) stored rows as the even image rows, the diagonal green difference is 59.0 (C5060WZ), 36.8 (C7070WZ), 81.6 (SP510UZ) and 38.7 (SP565UZ), close to the same-color vertical difference. The stored order gives 329, 210, 270 and 411. Shifting the fields by one row moves the green phase against the CFAPattern tag. In all 4 files each field starts at the start of a strip: the strips hold 16 stored rows, except the last strip of the even field (15, 3, 3 and 11 rows) and the last strip of the odd field. Every StripByteCounts value is a multiple of 3 * W / 2. The only unused bytes between strips lie between the two fields. The strip count is therefore 146 for H = 2309 (C7070WZ, SP510UZ), one more than ceil(H / 16).
- M8 (V4, S14): in XZ-2 rows 1500 to 1563 the six readings little-endian pairs, MSB-first pairs, both after swapping bytes in 16-bit words, and both after reversing the bytes of 32-bit words give same-color roughness 1160, 1096, 1086, 1200, 316 and 24.2. Only the 32-bit reversal with MSB-first pairs (R-ORF-25) gives a photograph. SH-2 and XZ-10 decode with the same reading (M18).
- M9 (V5 stream header, S10, S15): the strips of all 64 V5 files start with 00 00 00 00 01 00 00. The eighth byte varies. Decoding the E-3 strip with the code bits starting 0 to 10 bytes after the strip offset succeeds only for 7. Start 0 fails at row 542, every other start within the first 2 rows (a value outside 0 to 4095).
- M10 (V5 codes and state, S15): hypothesis grids decoded the E-3 strip. A hypothesis passes only if all 10416000 values are within 0 to 4095 and the bit string ends in the last byte of the strip. (1) Code structure, 192 hypotheses: bit order MSB-first or LSB-first, 6 orders of sign, low bits and Golomb-Rice code, 0 to 3 low bits, separate sign bit or the interleaved mapping of S5 FIG. 12, unary part of zeros closed by 1 or of ones closed by 0. Only the structure of R-ORF-29 passes. All others fail within 8 rows. (2) Code length, 3136 hypotheses k = max(K, bitlength(A) + d) with separate (K, d) for N < 3 and N >= 3, K from 0 to 7, d from -4 to 2: only (4, -2) and (2, 0) pass (R-ORF-28). (3) Run counter, 270 hypotheses: counted limit 0 to 40, 63, 64, 127, 128 and run length 1 to 6: only M <= 16 and N >= 3 pass. (4) State scope, 12 hypotheses: neighbours at distance 1 with one state or at distance 2 with a state per parity, state reset per row or never, k from M, from abs(D) or from abs(e): only distance 2, per-parity state, reset per row and k from M pass (R-ORF-27). (5) Escape, 189 hypotheses: zero limit 8 to 16, field width (total minus k) for total 12 to 18, right shift 0 to 2: six pass, those with zero limit plus total = 28 and shift 1, which give the same codes on this file (OQ-1). The same six, and no other, pass on the OM-1 file of S16 (36446 escapes). The passing decode ends 4 bits before the end of the strip and contains 2368 escapes.
- M11 (V5 values, S15): with the parse fixed, these value rules were varied over the whole E-3 strip. A lossless decode must give no value outside 0 to 4095. (1) Prediction: averaging threshold t of the prediction table for t = 0, 1, 2, 4, 8, 15, 16, 24, 30, 31, 32, 33, 34, 40, 48, 63, 64, 128, never averaging (S5 Formula 1 alone) and always averaging, plus the left neighbour alone. Only t = 32 gives no value outside the range. t = 31 gives 10137 such values, t = 33 gives 85147, S5 Formula 1 alone 19367, always averaging 4395128. The same-color roughness is also lowest for t = 32 (25.90). (2) Sign: E = -M in place of -M - 1 gives 6858472 values outside the range. (3) Correction filter C' = floor((a * D + b * C) / 2^s) for a from 0 (no filter) to 8, b from 0 to 4 and s from 0 to 8, 361 combinations: 116 diverge, 243 give 15 or more values outside the range, and only a = 3, b = 1, s = 5 and the identical function a = 6, b = 2, s = 6 give none. On the darker E-M1MarkII file, 100 filters near this one (a 1 to 5, b 0 to 3, s 3 to 7) leave three without values outside the range, and a = 3, b = 1, s = 5 has the lowest roughness of them (6.42 against 6.54). The other two fail on E-3.
- M12 (V5 full decodes, S15, S16): with all rules of section Compression, the strips of E-3, E-510, E-5, E-PL1, E-P1, E-M5, E-M1, E-PM2, E-M10 Mark II, E-M1MarkII and OM-1 decode with every value within 0 to 4095 and end with 4, 1, 0, 5, 5, 5, 0, 4, 6, 0 and 7 unused bits. The bit after every escaped Q is 1 (2368 of 2368 in E-3, 36446 of 36446 in OM-1). Re-encoding the first 8 decoded values of E-3 row 0 with the rules of this document reproduces the first 8 code bytes of the strip (TV-9).
- M13 (V6, S17): the strips of the 4 V6 files start with 00 00 00 00 00 01 00 00. With the code bits starting at byte 8 and every rule of V5 unchanged, the OM-1 Mark II strip decodes to 8172 x 6132 values from 920 to 16383 and ends with 4 unused bits. The bit after every escaped Q is 1 (713779 of 713779). Starting at byte 7, 12 or 16 fails within the first 2 rows for all 192 tested combinations of low-bit count, escape width, code length rule and run limit. The decoded image matches the preview (M18). The escape grid of M10 leaves five readings on this file: the zero limit 16 with total 12 fails here, because escaped Q values up to 230 need more bits.
- M14 (CFA, S11, S13, S14, S16): for 6 files with 4 different CFAPattern values, the comparison with the preview (M18 method) gives a chromaticity error of 0.020 to 0.082 with the pattern of the tag placed at stored pixel (0, 0), and 0.13 to 0.72 with each of the three other Bayer phases.
- M15 (black level scale, S16, S17): the V6 file has BlackLevel2 255 254 254 255 and its darkest values are 1045 to 1048 (0.001 to 0.01 percentile). With the black levels multiplied by 4 the chromaticity error against the preview is 0.028. With the tag values as they are it is 0.238. E-M5, E-P1 and E-PM2 have BlackLevel2 0 0 0 0 and darkest values of 0 to 6.
- M16 (black level values, S10 to S16): black level tags of all 85 files: 32 (E-10, 10-bit), 63 to 96 (E-1 to E-PL3 generation), 200 (SH-2, XZ-2, XZ-10, STYLUS1), 252 to 257 (later models), 0 (E-M5, E-P1, E-PM2, V3 cameras). In 16 of 17 decoded files the 0.01 percentile of the values lies between 3 below and 32 above the black level of the tag. In the exception, E-PL1 at ISO 1600, it lies 58 below, and the noise reaches 0. The pixels outside the crop rectangle hold image data (for example per-CFA-position medians of 371 to 938 in columns 0 to 51, left of the E-3 crop at column 54), so the raster has no masked border.
- M17 (clip levels, S11 to S16): the decoded rasters of E-500, E-330, SH-2, C5060WZ, C7070WZ, E-P1 and OM-1 have 123 to 575219 pixels at 4095, SP565UZ has 246404 at 4094. E-M5 values pile up at 3791 to 3796, E-PL1 at 3952 to 3958, XZ-2 at 4092. SensorCalibration value 1 is 4044 (E-M5), 3932 (E-PL1), 3895 (XZ-2, SH-2), 3645 (E-P1) and 4095 (OM-1, E-M1MarkII), so it does not give the clip level.
- M18 (white balance and matrix, 24 files of S11 to S17): method: the raw raster is black-subtracted and white-balanced per CFA cell (half resolution, crop applied), multiplied by the matrix, averaged over a grid of 24 x 32 blocks, and compared with the preview, which was decoded with Pillow and linearized with the inverse sRGB transfer function (S7). The error is the mean absolute difference of the chromaticities (r/(r+g+b), g/(r+g+b)) over blocks that are neither clipped nor dark. Results: matrix 0.016 to 0.116, identity higher in all 24 files, transposed matrix 0.092 to 0.355 and higher in 23 files (not in E-PL1, picture mode i-Enhance). The correlation of log luminance with the preview is 0.94 to 0.999 in 22 files, 0.48 in E-10 (a detailed scene in a 160x120 preview) and 0.84 in SH-2 (lens distortion corrected in the camera JPEG). White balance (11 files): unit multipliers give 4 to 30 times the error of the tag multipliers, and exchanged red and blue multipliers give a higher error in all 11 files. Row sums of the matrices: 256 in 79 of 85 files.
- M19 (previews, S10): CameraSettings previews in 78 files: 3200x2400 in 68 files, 1600x1200 in 9 files (E-300, E-330, E-400, E-500, E-3, E-410, E-420, E-450, E-510), 1280x960 in E-1. The main maker-note thumbnail (160x120) is present in all 85 files. Every preview stream starts with FF D8 FF DB (baseline JPEG, no APP1). Some thumbnails do not end with FF D9 at their stated length (C5060WZ ends with FF FF, XZ-2 with other bytes). The previews of TG-6 and XZ-10, shot with the aspect ratio 3:2, are 3200x2400 with image content in the top and bottom rows (not letterboxed).
- M20 (orientation, S14): XZ-10 has Orientation 6. Its preview is 3200x2400 (landscape). The unrotated raw image matches the unrotated preview with a chromaticity error of 0.022 and a log luminance correlation of 0.990.
- M21 (crop, S10): ImageProcessing crop tags in 78 files, RawInfo crop tags in 5 files, none in E-10 and C5060WZ. Every crop rectangle lies inside the raster and has the aspect ratio 4:3 (for example 4608 x 3456 at 8, 8 in a 4640 x 3472 raster). Five files have AspectRatio 3:2 or 16:9 and an AspectFrame inside the crop rectangle.

### Sources encountered and not used

A web search for a prose description of the ORF compression returned, among other results, the titles and short summaries of four pull requests and a roadmap file of a raw-processing project on GitHub (storytold/lightcraft pull requests 716, 2068, 3747 and 3949, ROADMAP.md). The pull requests implement or document an ORF decoder. The author did not open them. The search summary named the two Olympus patents (S5, S6), which the author then read at their official patent pages. The summary also repeated a few phrases from the pull requests (a header of 7 bytes, parity states per row, a tag 0x0611). The author did not use them: M9 to M13 establish the stream header, the coding state and the bit depth by measurement. Other search results (format overview pages and converter advertisements) were seen as titles only and not used.

## Scope and variants

### Generations

The measured files fall into three generations by their header bytes and maker-note style (M1, M3). The decoder does not need the generation: R-ORF-3 classifies files by structure alone.

| Header bytes 0 to 3 | Note style | Measured models | Variants seen |
| --- | --- | --- | --- |
| 4D 4D 4F 52 ("MMOR", big-endian) | OLYMP | E-10 | V1 |
| 49 49 52 4F ("IIRO") | OLYMP | E-1, E-300, E-330, E-400, E-500 | V1, V2 |
| 49 49 52 53 ("IIRS") | OLYMP | C5060WZ, C7070WZ, SP510UZ, SP550UZ, SP565UZ, SP570UZ | V3 |
| 49 49 52 4F | OLYMPUS | E-3, E-410, E-420, E-450, E-5, E-510, E-620, E-P1, E-P2, E-P3, E-P5, E-P7, E-PL1, E-PL2, E-PL3, E-PL5 to E-PL10, E-PM1, E-PM2, PEN-F, E-M1, E-M1MarkII, E-M1MarkIII, E-M1X, E-M5, E-M5MarkII, E-M5MarkIII, E-M10, E-M10MarkII, E-M10 Mark III, E-M10MarkIIIS, E-M10MarkIV, XZ-1, XZ-2, XZ-10, SH-2, STYLUS1, TG-4, TG-5, TG-6 | V2, V4, V5 |
| 49 49 52 4F | OM SYSTEM | OM-1, OM-1MarkII, OM-3, OM-5, OM-5MarkII, TG-7 | V5, V6 |

### Storage variants

The sensor data of every measured file is one CFA raster of W x H values. Six storage methods occur (M3 to M13). In all of them IFD0 tag 0x0103 (Compression) is 1, so the Compression tag does not identify the method. The total number of strip bytes, BitsPerSample, the header bytes and the first bytes of the strip identify it.

| Variant | Sensor data | Total strip bytes | BitsPerSample | Measured models |
| --- | --- | --- | --- | --- |
| V1 | 16-bit samples in file byte order, B significant bits in the high part of each sample | 2 * W * H | 16 | E-10 (B = 10, big-endian), E-1, E-400 (B = 12) |
| V2 | 12-bit samples, 10 samples in 15 bytes plus 1 pad byte | 16 * W * H / 10 | 16 | E-300, E-330, E-500, E-M5 Mark II and PEN-F high-resolution shots |
| V3 | 12-bit samples, 2 samples in 3 bytes, MSB-first, rows stored as two fields | 3 * W * H / 2 | 12 | C5060WZ, C7070WZ, SP510UZ, SP550UZ, SP565UZ, SP570UZ |
| V4 | 12-bit samples, MSB-first bit string in 32-bit little-endian words | 3 * W * H / 2 | 12 | SH-2, XZ-2, XZ-10 |
| V5 | Olympus lossless compression, B = 12 | any other count, strip starts with 00 00 00 00 01 00 00 | 16 | 47 models, from E-3 to OM-5 Mark II |
| V6 | Olympus lossless compression, B = 14 | any other count, strip starts with 00 00 00 00 00 01 00 00 | 16 | OM-1 Mark II, OM-3 (14-bit high-resolution modes) |

S5 and S6, two Olympus patents, describe a lossless compression for camera RAW data. V5 and V6 follow the method of these patents. The measurements M9 to M13 fix every detail that the patents leave open.

### Out-of-scope relatives

| Id | Relative | Detection from file contents |
| --- | --- | --- |
| X1 | a file that matches no rule of R-ORF-3, for example a model with another strip layout that no public CC0 sample shows (the archive has non-CC0 samples of C5050Z, C8080WZ, E-20, SP350 and SP500UZ, which the author did not measure) | no rule of R-ORF-3 matches |
| X2 | big-endian files with a layout other than V1 | header bytes 0 and 1 are 4D 4D and the total strip bytes differ from 2 * W * H |
| X3 | ORI files (the companion files of high-resolution shots, extension .ORI), not measured (OQ-14) | none: the decoder classifies them by content like any other file (R-ORF-3) |

### Requirements

- R-ORF-1: The decoder must decode the sensor data of variants V1, V2, V3, V4, V5 and V6 as this document specifies.
- R-ORF-2: The decoder must use the preview path (section Embedded preview) for X1 and X2 files and must not interpret their sensor data.
- R-ORF-3: The decoder must classify the file by these rules, in this order, where n = W * H, T is the total of all StripByteCounts values, BPS is the first value of IFD0 tag 0x0102 and "first bytes" are the first bytes of the first strip:
  1. T = 2 * n: V1.
  2. Header bytes 0 and 1 are 4D 4D: X2.
  3. W mod 10 = 0 and T = 16 * n / 10: V2.
  4. BPS = 12, header bytes 2 and 3 are 52 53, W mod 2 = 0 and T = 3 * n / 2: V3.
  5. BPS = 12, header bytes 2 and 3 are 52 4F, W mod 8 = 0 and T = 3 * n / 2: V4.
  6. One strip, W mod 2 = 0, and the first 7 bytes are 00 00 00 00 01 00 00: V5.
  7. One strip, W mod 2 = 0, and the first 8 bytes are 00 00 00 00 00 01 00 00: V6.
  8. Any other case: X1.

Before it applies these rules, the decoder checks the required IFD0 tags (R-ORF-14). When the first strip lies outside the file or is shorter than 8 bytes, rules 6 and 7 do not match.

The measured W values satisfy these divisibility conditions in every file (M3). For V4 the condition W mod 8 = 0 makes every row a whole number of 32-bit words (3 * W / 2 bytes, a multiple of 4).

## Container layout

### Byte order and header

An ORF file has the TIFF 6.0 structure (S1 section 2) with a different number in bytes 2 and 3. TIFF 6.0 puts 42 there. ORF puts the two letters "RO" or "RS" there (M1).

| Offset | Size | Field | Value in ORF |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 49 49 ("II", little-endian) or 4D 4D ("MM", big-endian) |
| 2 | 2 | ORF signature | 52 4F or 52 53 when little-endian (16-bit values 0x4F52 or 0x5352), 4F 52 when big-endian (value 0x4F52) |
| 4 | 4 | offset of IFD0 | 8 in all measured files |

- R-ORF-4: The decoder must accept a file only if bytes 0 to 3 are 49 49 52 4F, 49 49 52 53 or 4D 4D 4F 52, and must report an error (no raw image, no preview) for any other file that reaches the ORF decoder, and also when IFD0 is invalid by R-ARW-14.
- R-ORF-5: The decoder must read every multi-byte value of the TIFF structure, of the maker note (unless its header states another byte order, R-ORF-12) and of V1 samples in the byte order of header bytes 0 and 1.

### IFD structure

The IFD layout, the field types 1 to 13 and the rule for values in the value field or at an offset are those of arw.md, section Container layout, subsection IFD structure (from S1 section 2). ORF files use type 13 (IFD) for maker-note sub-IFD pointers (M2).

- R-ORF-6: The decoder must parse IFDs as R-ARW-6, R-ARW-7, R-ARW-9 and R-ARW-10 specify, must treat an IFD as invalid in the cases of R-ARW-14, and must use the first entry when a tag occurs more than once in one IFD.
- R-ORF-7: The decoder must accept the types SHORT, LONG and IFD (13) for every unsigned integer tag of this document, whatever type the tables list, and must read SSHORT (type 8) values and SHORT values that this document marks "signed" as signed 16-bit integers.

### IFD0 and the IFD chain

In ORF, IFD0 is the raw IFD: it holds ImageWidth, ImageLength, the strip tags of the sensor data and the Exif IFD pointer (M1). No measured ORF file has a SubIFDs tag (0x014A). Older files (OLYMP note style) have an IFD1 with a 160x120 JPEG thumbnail in tags 0x0201 and 0x0202. Newer files have no IFD1. The previews are in the maker note (section Embedded preview).

Example (E-M10 Mark II file of S16): IFD0 at offset 8 with 24 entries and next-IFD offset 0, Exif IFD at 302, maker note at 3572 (1481740 bytes, OLYMPUS style, it contains the previews), preview JPEG at 3572 + 48652 = 52224 (1029414 bytes), sensor strip at 1485312 (13786930 bytes).

- R-ORF-8: The decoder must read IFD0 at the offset in header bytes 4 to 7 and must use it as the raw IFD.
- R-ORF-9: The decoder must follow the next-IFD offsets from IFD0 as R-ARW-12 specifies, only to collect preview candidates.
- R-ORF-10: The decoder must find the Exif IFD at the offset in IFD0 tag 0x8769, and the maker note as the value of Exif tag 0x927C (type UNDEFINED, count = maker-note length in bytes).

### Maker note

The maker note starts with a header. Three header forms occur (M2). The header fixes the byte order of the maker note, the position of the main maker-note IFD and the origin of all offsets inside the maker note.

| Note style | Header bytes | Length | Main IFD at | Offsets inside the maker note count from | Byte order |
| --- | --- | --- | --- | --- | --- |
| OLYMP | "OLYMP", 00, then 2 version bytes (01 00, 02 00 or 02 01 measured) | 8 | maker-note start + 8 | start of the file | of the file |
| OLYMPUS | "OLYMPUS", 00, then "II" or "MM", then 2 version bytes (03 00 measured) | 12 | maker-note start + 12 | start of the maker note | from bytes 8 and 9 of the header |
| OM SYSTEM | "OM SYSTEM", 00 00 00, then "II" or "MM", then 2 version bytes (04 00 measured) | 16 | maker-note start + 16 | start of the maker note | from bytes 12 and 13 of the header |

"Offsets count from the start of the maker note" means: the file offset of a value is the maker-note start plus the 4-byte value of the entry. This applies to the value offsets of every entry in the main IFD and in all sub-IFDs, to the sub-IFD pointers and to PreviewImageStart (M2).

The main maker-note IFD points to sub-IFDs with these tags (S4 names):

| Tag | Name (S4) | Content used by this document |
| --- | --- | --- |
| 0x2020 | CameraSettings | preview JPEG position and length |
| 0x2040 | ImageProcessing | white balance, color matrix, black level, bit depth, crop |
| 0x3000 | RawInfo | the same data in older compact cameras (C7070WZ, SP510UZ, SP550UZ, SP565UZ, SP570UZ) |

In the OLYMPUS and OM SYSTEM styles these entries have type 13 (IFD) or 4 (LONG), count 1, and the value is the offset of the sub-IFD (counted from the maker-note start). In the OLYMP style the entries have type 7 (UNDEFINED): the sub-IFD itself is the value, so it starts at the value offset of the entry (counted from the file start), and the offsets inside it also count from the file start (M2).

- R-ORF-11: The decoder must recognize the note style by the header bytes of the table above without checking the version bytes, and must use the preview path for the raw data (not for the previews in IFD0 and IFD1) when the maker note is absent, shorter than its header or has another header.
- R-ORF-12: The decoder must read the main maker-note IFD and its sub-IFDs with the byte order, the IFD position and the offset origin that the table above gives for the note style.
- R-ORF-13: The decoder must locate a sub-IFD at (offset origin + entry value) when the entry type is 13 or 4 and the count is 1, and at the value offset of the entry when the entry type is 7 and the count is greater than 4, must read the offsets inside every sub-IFD with the offset origin of the note style, and must treat a sub-IFD as absent in every other case and when it is invalid by R-ARW-14.

The decoder checks maker-note values, sub-IFDs and previews against the file size only. They need not lie inside the byte range of the maker note: in OLYMP-style files the preview lies outside it (E-1: maker note of 3038 bytes at 788, preview at 34156).

## Metadata tags

"Required" means the decoder cannot decode the raw data without the tag. The decoder then uses the preview path. "Default" is the value to use when an optional tag is absent or invalid. The types and counts are those of all measured files (M4).

### IFD0

| Tag | Name | Type | Count | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- |
| 0x0100 | ImageWidth | LONG | 1 | stored width W (S1) | required | none |
| 0x0101 | ImageLength | LONG | 1 | stored height H (S1) | required | none |
| 0x0102 | BitsPerSample | SHORT | 1 | 16 (V1, V2, V5, V6) or 12 (V3, V4), used by R-ORF-3 only | required | none |
| 0x0103 | Compression | SHORT | 1 | 1 in every measured file, also for V5 and V6, not used | ignored | none |
| 0x0106 | PhotometricInterpretation | SHORT | 1 | 1, or 2 in V3 files, not used | ignored | none |
| 0x010F | Make | ASCII | any | "OLYMPUS OPTICAL CO.,LTD", "OLYMPUS CORPORATION", "OLYMPUS IMAGING CORP." or "OM Digital Solutions" | optional | empty |
| 0x0110 | Model | ASCII | any | model name, not used for decoding | optional | empty |
| 0x0111 | StripOffsets | SHORT or LONG | strips | file offset of each strip | required | none |
| 0x0112 | Orientation | SHORT | 1 | 1 to 8 (S1). Measured 1, and 6 in one file (M20) | optional | 1 |
| 0x0115 | SamplesPerPixel | SHORT | 1 | 1 | ignored | none |
| 0x0116 | RowsPerStrip | SHORT or LONG | 1 | rows in each strip (16 in V3, 32 to 130 in V1, H in V2, V4, V5, V6) | optional | H |
| 0x0117 | StripByteCounts | SHORT or LONG | strips | bytes in each strip | required | none |
| 0x8769 | ExifIFD | LONG or IFD | 1 | offset of the Exif IFD | required | none |
| 0x0201, 0x0202 | JPEGInterchangeFormat, JPEGInterchangeFormatLength | LONG | 1 | in IFD1 of OLYMP-style files: 160x120 thumbnail | optional | none |

### Exif IFD

| Tag | Name | Type | Count | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- |
| 0xA302 | CFAPattern | UNDEFINED | 8 | S3: SHORT horizontal repeat, SHORT vertical repeat (both in the file byte order, both 2 in all files), then the colors of CFA positions 0, 1, 2, 3 as bytes (0 red, 1 green, 2 blue) | required | none |
| 0x927C | MakerNote | UNDEFINED | maker-note length | section Container layout | required | none |

S3 orders the CFA values by column first: value [0.0], [1.0], then [0.1], [1.1], where the first index is the column. This is the order of the CFA positions 0, 1, 2, 3 of this document. The measured patterns are 0 1 1 2 (RGGB), 1 0 2 1 (GRBG), 2 1 1 0 (BGGR) and 1 2 0 1 (GBRG) (M14).

### Main maker-note IFD

| Tag | Name (S4) | Type | Count | Meaning | Use |
| --- | --- | --- | --- | --- | --- |
| 0x0100 | ThumbnailImage | UNDEFINED | JPEG length | the bytes of a 160x120 JPEG thumbnail, at the value offset (M19) | preview candidate |
| 0x1011 | ColorMatrix | SHORT, signed | 9 | color matrix, OLYMP style only | color source 3 |
| 0x1012 | BlackLevel | SHORT | 4 | black levels on the data scale, OLYMP style only | black source 3 |
| 0x1017 | RedBalance | SHORT | 2 | value 1: red white balance level, 256 = gain 1. Value 2 (62 or 64) is not used | white balance source 3 |
| 0x1018 | BlueBalance | SHORT | 2 | value 1: blue white balance level, 256 = gain 1. Value 2 is not used | white balance source 3 |
| 0x102C | ValidBits | SHORT | 2 | value 1: bit depth B (10 or 12 measured) | bit depth source 3 |
| 0x2020 | CameraSettings | IFD, LONG or UNDEFINED | 1 or block length | sub-IFD (R-ORF-13) | previews |
| 0x2040 | ImageProcessing | IFD, LONG or UNDEFINED | 1 or block length | sub-IFD (R-ORF-13) | levels and color |
| 0x3000 | RawInfo | UNDEFINED | block length | sub-IFD (R-ORF-13) | levels and color of compact cameras |

### ImageProcessing IFD (tag 0x2040)

| Tag | Name (S4) | Type | Count | Meaning | Use |
| --- | --- | --- | --- | --- | --- |
| 0x0100 | WB_RBLevels | SHORT | 2 or 4 | value 1: red level, value 2: blue level. With 4 values, values 3 and 4 are 256 to 260 (S4: "presumably RBGG levels") and are not used | white balance source 1 |
| 0x0200 | ColorMatrix | SHORT, signed | 9 | color matrix, row by row, 256 = 1 | color source 1 |
| 0x0600 | BlackLevel2 | SHORT | 4 | black levels of CFA positions 0 to 3 on the 12-bit scale (M15, M16) | black source 1 |
| 0x0610 | GainBase | SHORT | 1 | 256 in all files, the divisor of the white balance levels | white balance |
| 0x0611 | ValidBits | SHORT | 2 | value 1: bit depth B (12 or 14 measured) | bit depth source 1 |
| 0x0612 | CropLeft | SHORT | 2 | value 1: x of the final image area | crop source 1 |
| 0x0613 | CropTop | SHORT | 2 | value 1: y of the final image area | crop source 1 |
| 0x0614 | CropWidth | LONG | 1 | width of the final image area | crop source 1 |
| 0x0615 | CropHeight | LONG | 1 | height of the final image area | crop source 1 |
| 0x0805 | SensorCalibration | SSHORT | 2 | S4: "recommended maximum, calibration midpoint". Not used (M17) | ignored |
| 0x1112, 0x1113 | AspectRatio, AspectFrame | BYTE, SHORT | 2, 4 | the aspect ratio chosen for the camera JPEG and its frame inside the crop area. Not applied (OQ-9) | ignored |

### RawInfo IFD (tag 0x3000)

| Tag | Name (S4) | Type | Count | Meaning | Use |
| --- | --- | --- | --- | --- | --- |
| 0x0100 | WB_RBLevelsUsed | SHORT | 2 | red level, blue level, 256 = gain 1 | white balance source 2 |
| 0x0200 | ColorMatrix2 | SHORT, signed | 9 | color matrix, row by row, 256 = 1 | color source 2 |
| 0x0600 | BlackLevel2 | SHORT | 4 | black levels on the data scale (0 in all measured files) | black source 2 |
| 0x0611 | ValidPixelDepth | SHORT | 2 | value 1: bit depth B (12) | bit depth source 2 |
| 0x0612, 0x0613 | CropLeft, CropTop | SHORT | 1 | x and y of the final image area | crop source 2 |
| 0x0614, 0x0615 | CropWidth, CropHeight | LONG | 1 | size of the final image area | crop source 2 |

### CameraSettings IFD (tag 0x2020)

| Tag | Name (S4) | Type | Count | Meaning |
| --- | --- | --- | --- | --- |
| 0x0100 | PreviewImageValid | LONG | 1 | 1 = a preview is present (1 in all files) |
| 0x0101 | PreviewImageStart | LONG | 1 | offset of the preview JPEG, counted from the offset origin of the note style (M19) |
| 0x0102 | PreviewImageLength | LONG | 1 | length of the preview JPEG in bytes |

- R-ORF-14: The decoder must use the preview path when a tag that the tables above mark "required" is absent or has a count of 0, or when a required offset (StripOffsets with StripByteCounts, ExifIFD, the MakerNote value) points outside the file.
- R-ORF-15: The decoder must accept the CFA pattern only if the CFAPattern value has 8 bytes, its two SHORT values are 2 and 2, and its four color bytes contain exactly one 0, two 1 and one 2, and must otherwise use the preview path.
- R-ORF-16: The decoder must read the 9 values of a color matrix tag as signed 16-bit integers (a stored value v >= 32768 means v - 65536).

## Sensor data layout

### Raster and final image area

The raw raster has W columns and H rows. All pixels of the raster hold image data: the measured files have no masked (optical black) border (M16). The final image area is the crop rectangle with origin (CX, CY) and size (CW, CH) from the ImageProcessing tags 0x0612 to 0x0615 or the RawInfo tags of the same numbers. The crop rectangle has the aspect ratio 4:3 of the camera JPEG in every measured file, also when the user chose another aspect ratio (M21).

Measured dimensions (M3, M21):

| File | Variant | W x H | CX, CY | CW x CH | Preview |
| --- | --- | --- | --- | --- | --- |
| E-10 (S11) | V1 | 2256 x 1684 | no tags | no tags | 160x120 |
| E-1 (S11) | V1 | 2624 x 1966 | 36, 34 | 2560 x 1920 | 1280x960 |
| E-500 (S12) | V2 | 3360 x 2504 | 23, 45 | 3264 x 2448 | 1600x1200 |
| C5060WZ (S13) | V3 | 2608 x 1949 | no tags | no tags | 160x120 |
| C7070WZ (S13) | V3 | 3088 x 2309 | 6, 0 | 3072 x 2304 | 160x120 |
| XZ-2 (S14) | V4 | 3984 x 2986 | 8, 10 | 3968 x 2976 | 3200x2400 |
| E-3 (S15) | V5 | 3720 x 2800 | 54, 22 | 3648 x 2736 | 1600x1200 |
| E-M10 Mark II (S16) | V5 | 4640 x 3472 | 8, 8 | 4608 x 3456 | 3200x2400 |
| E-M1MarkII (S16) | V5 | 5240 x 3912 | 12, 12 | 5184 x 3888 | 3200x2400 |
| OM-1 (S16) | V5 | 5220 x 3912 | 12, 12 | 5184 x 3888 | 3200x2400 |
| OM-1 Mark II 14-bit (S17) | V6 | 8172 x 6132 | 6, 6 | 8160 x 6120 | 3200x2400 |

- R-ORF-17: The decoder must use the crop rectangle of the ImageProcessing IFD when its four tags are present with CW >= 2, CH >= 2, CX + CW <= W and CY + CH <= H, else the crop rectangle of the RawInfo IFD under the same conditions, and else the whole raster (0, 0, W, H).
- R-ORF-18: The decoder must not apply the AspectFrame tag (0x1113).

### CFA arrangement

The CFA pattern repeats every 2 rows and 2 columns. Its origin is pixel (0, 0) of the raster, not the crop origin (M14). For V3 the raster is the image after the field placement of R-ORF-21, so y is the image row, not the stored row (M7). CFAPattern lists the colors of CFA positions 0, 1, 2, 3, which are (x even, y even), (x odd, y even), (x even, y odd), (x odd, y odd) (S3).

- R-ORF-19: The decoder must give pixel (x, y) of the raster the color CFAPattern[2 * (y mod 2) + (x mod 2)], with the origin of the pattern at pixel (0, 0) and, for V3, y counted in image rows after R-ORF-21.

### Strips

IFD0 lists one or more strips (S1 sections 3 and 8). Each strip of V1 to V4 holds a whole number of stored rows, StripByteCounts / RB, where RB is the number of bytes per stored row. The stored rows follow each other strip by strip, in the order of the strips. V1 files have several contiguous strips of RowsPerStrip rows, the last one shorter. In V3 files each field starts with a new strip: the even field fills strips of 16 rows and ends with a shorter strip, then unused bytes follow, then the odd field fills strips in the same way (M7). V2, V4, V5 and V6 files have one strip.

| Variant | Strips (measured) | Bytes per row RB |
| --- | --- | --- |
| V1 | 15 strips of 116 rows (E-10), 16 of 130 (E-1), 89 of 32 (E-400) | 2 * W |
| V2 | 1 | 16 * W / 10 |
| V3 | 122 to 172 strips of 16 rows, the last strip of each field shorter | 3 * W / 2 |
| V4 | 1 | 3 * W / 2 |
| V5, V6 | 1 | not defined (the rows are bit strings without alignment) |

- R-ORF-20: For V1 to V4 the decoder must read the stored rows strip by strip in the order of the strips, taking StripByteCounts / RB rows of RB bytes from each strip at its StripOffsets value (R-ORF-58 requires a whole number), must stop after H rows, and must not assume that one strip follows the other or that every strip holds RowsPerStrip rows.

### Stored row order of V3

V3 files store the rows as two fields: first all even rows (0, 2, 4, ...), then all odd rows (1, 3, 5, ...) (M7). With He = ceil(H / 2), stored row s holds image row 2 * s for s < He and image row 2 * (s - He) + 1 for s >= He. The strips hold stored rows in stored order.

- R-ORF-21: For V3 the decoder must place stored row s at image row 2 * s when s < ceil(H / 2), and at image row 2 * (s - ceil(H / 2)) + 1 otherwise.

Example: H = 5 gives He = 3. Stored rows 0, 1, 2, 3, 4 hold image rows 0, 2, 4, 1, 3.

## Compression

This section specifies, for each variant, how the stored bytes become sensor values s(x, y). The values are integers from 0 to 2^B - 1, where B is the bit depth of section Levels and white balance (B = 12 for V2 to V5, B = 14 for V6, B from the ValidBits tags for V1).

### V1: 16-bit samples

Each pixel is a 16-bit unsigned integer in the byte order of the file. Pixel (x, y) is at byte 2 * x of its stored row. The B significant bits are the high bits of the sample: in the measured files the low 16 - B bits are 0 in all but a few samples (M5: 99.9999 percent of the samples in E-10 with B = 10, 100 percent in E-1 and 99.993 percent in E-400 with B = 12).

- R-ORF-22: The decoder must read a V1 sample as b0 + 256 * b1 (little-endian file) or 256 * b0 + b1 (big-endian file), where b0 and b1 are the bytes at offsets 2 * x and 2 * x + 1 of the row, and must use s = floor(sample / 2^(16 - B)).

Worked example (first samples of the E-10 strip, big-endian, B = 10): 0x0C 0x00 gives 3072 and s = floor(3072 / 64) = 48. The next three samples 0x0C 0xC0, 0x0B 0x80, 0x0D 0x80 give 51, 46 and 54.

### V2: 10 samples in 16 bytes

Each row is a sequence of groups of 16 bytes. Group g of a row (g = 0 to W / 10 - 1) holds the 10 pixels with x = 10 * g to 10 * g + 9. In a group, byte 15 is a pad byte (0 in all measured groups, M6). Bytes 0 to 14 hold 5 pairs of pixels. Pair j (j = 0 to 4) is the 3 bytes c0, c1, c2 at group bytes 3 * j to 3 * j + 2. The 24-bit little-endian value c0 + 256 * c1 + 65536 * c2 holds the first pixel of the pair in bits 0 to 11 and the second pixel in bits 12 to 23 (M6).

- R-ORF-23: The decoder must compute V2 pixel 10 * g + 2 * j as c0 + 256 * (c1 mod 16) and pixel 10 * g + 2 * j + 1 as floor(c1 / 16) + 16 * c2, and must ignore byte 15 of each group.

Worked example (first group of the E-500 strip): 6B 22 61 BA A2 5E 97 02 5E D5 F2 5D BF 72 5E 00.
- Pair 0: c0 = 0x6B, c1 = 0x22, c2 = 0x61. Pixel 0 = 107 + 256 * 2 = 619. Pixel 1 = 2 + 16 * 97 = 1554.
- Pairs 1 to 4 give 698, 1514, 663, 1504, 725, 1503, 703, 1511. The pad byte is 0x00.

### V3: 12-bit pairs, MSB-first

Each stored row holds 3 * W / 2 bytes. Pixels 2k and 2k + 1 of the stored row come from the 3 bytes c0, c1, c2 at row offset 3k. The 24-bit value 65536 * c0 + 256 * c1 + c2 holds the first pixel in bits 12 to 23 and the second pixel in bits 0 to 11 (M7). The stored rows are then placed by R-ORF-21.

- R-ORF-24: The decoder must compute V3 pixel 2k of a stored row as 16 * c0 + floor(c1 / 16) and pixel 2k + 1 as 256 * (c1 mod 16) + c2.

Worked example (first bytes of the C5060WZ strip): 2B F5 93 31 15 8F.
- Pair 0: 16 * 43 + 15 = 703 and 256 * 5 + 147 = 1427.
- Pair 1: 16 * 49 + 1 = 785 and 256 * 5 + 143 = 1423.

### V4: 12-bit values in 32-bit little-endian words

Each row holds 3 * W / 2 bytes, which is 3 * W / 8 words of 4 bytes. Word i of the row is the little-endian 32-bit integer U(i) = w0 + 256 * w1 + 65536 * w2 + 16777216 * w3 of its bytes w0 to w3. The 32 bits of U(0), U(1), ..., each word from its most significant bit to its least significant bit, form one bit string. Pixel x is the 12-bit number formed by bits 12 * x to 12 * x + 11 of that string, the first of them most significant (M8). Every group of 3 words holds 8 pixels.

- R-ORF-25: The decoder must take the V4 pixels of a row as consecutive 12-bit fields, most significant bit first, of the bit string that the row's little-endian 32-bit words form when each word is taken from its most significant bit to its least significant bit.

Worked example (XZ-2 file of S14, row 1500, pixels 400 to 407, bytes at row offset 600): 79 A4 CE 7A 0E 79 B4 CE D8 4E 7C E4.
- Words: 0x7ACEA479, 0xCEB4790E, 0xE47C4ED8.
- Bit string in hexadecimal digits: 7ACEA479 CEB4790E E47C4ED8. Each pixel is 3 hexadecimal digits: 7AC, EA4, 79C, EB4, 790, EE4, 7C4, ED8.
- Pixels: 1964, 3748, 1948, 3764, 1936, 3812, 1988, 3800.

### V5 and V6: Olympus lossless compression

Sources: S5 and S6 describe the method in prose and formulas. Each pixel value is predicted from already decoded neighbours (S5 Formula 1 and its rules for the start of a line). The prediction error is split into low-order bits that are stored as they are (S6 Formulas 10, 13 and 19), a sign and a magnitude (S5 Formula 10). The magnitude is a Golomb-Rice code: a unary part of zeros closed by a 1, then a fixed part of k bits (S5 FIG. 15). The code length k depends on the magnitude of the previous code (S5 and S6, "dynamic range") and on a run counter of small magnitudes that is reset at the start of each line (S5 FIG. 7 to FIG. 10). A recursive correction filter links the codes (S6 Formulas 11, 14 and 17). S5 and S6 give example constants but leave the bit order, the field order, the constants of the camera format, the escape for long codes and the treatment of the Bayer pattern open. The measurements M9 to M13 fix them.

#### Stream

The strip starts with a stream header: the 7 bytes 00 00 00 00 01 00 00 in V5, the 8 bytes 00 00 00 00 00 01 00 00 in V6 (M9, M13). The code bits start at the first byte after the header. They form one bit string, read MSB-first, for the whole raster: rows follow each other without any alignment to a byte boundary. "Read n bits" takes the next n bits of the string and forms an unsigned integer with the first bit read as its most significant bit (n = 0 gives 0). After the last pixel, between 0 and 7 bits of the last strip byte are unused in all measured files (M12, M13).

- R-ORF-26: The decoder must start reading code bits at strip offset + 7 for V5 and at strip offset + 8 for V6, MSB-first, and must continue the bit string across row ends without skipping bits, and must ignore any bits and bytes of the strip after the last pixel.

#### Coding state

The coding state of a row has three integers per parity p (p = x mod 2): A(p), the magnitude of the previous code of the same parity, N(p), the run count, and C(p), the carry of the correction filter. The decoder sets all six integers to 0 at the start of every row (M10).

- R-ORF-27: The decoder must keep A, N and C separately for the even and the odd columns of a row, and must set them to 0 before the first pixel of every row.

#### Code of one pixel

The decoder decodes the pixels of a row in the order x = 0, 1, ..., W - 1, and the rows in the order y = 0, 1, ..., H - 1. For pixel (x, y) with parity p, it takes A = A(p), N = N(p), C = C(p) and performs these steps (M10, M11):

1. Code length: if N >= 3, k = max(2, bitlength(A)). Otherwise k = max(4, bitlength(A) - 2).
2. Read 1 bit: the sign bit S.
3. Read 2 bits: the low bits L (0 to 3).
4. Unary part: read single bits until a bit is 1 or until 12 bits have been read. If a 1 bit ends the run after Z zero bits (Z = 0 to 11), the quotient is Q = Z. If 12 zero bits were read (escape), read 15 - k bits as Q, then read 1 more bit and ignore its value (it was 1 in all 752593 measured escapes, M12, M13).
5. Read k bits as the remainder R. The magnitude is M = Q * 2^k + R.
6. Signed value: E = M if S = 0, and E = -M - 1 if S = 1 (S5 Formula 10).
7. Corrected value: D = E + C.
8. Prediction error: e = 4 * D + L.
9. Pixel value: s(x, y) = P + e, where P is the prediction of the next subsection.
10. State update of parity p: C(p) = floor((3 * D + C) / 32) with the old C of step 7, A(p) = M, and N(p) = N + 1 if M <= 16, otherwise N(p) = 0.

Bounds: when N >= 3 the previous M was at most 16, so k <= 5. Otherwise M < 2^15 (step 4 and step 5 never give more), so bitlength(A) <= 15 and k <= 13. Step 4 therefore reads at least 2 bits for Q. All intermediate values fit in a signed 32-bit integer.

- R-ORF-28: The decoder must compute k as max(2, bitlength(A)) when N >= 3 and as max(4, bitlength(A) - 2) otherwise.
- R-ORF-29: The decoder must read the fields of a code in the order sign bit, 2 low bits, unary part, remainder of k bits, as steps 2 to 5 specify, including the escape after 12 zero bits.
- R-ORF-30: The decoder must compute E, D and e by steps 6 to 8 and must update C, A and N by step 10, with floor division for negative values (for example floor(-7 / 32) = -1).

#### Prediction

The prediction uses the decoded values of the same CFA color: Ra = s(x - 2, y) (left), Rb = s(x, y - 2) (above) and Rc = s(x - 2, y - 2) (upper left), named as in S5 (M10, M11).

| Position | Condition | Prediction P |
| --- | --- | --- |
| x < 2 and y < 2 | none | 0 |
| y < 2 and x >= 2 | none | Ra |
| x < 2 and y >= 2 | none | Rb |
| x >= 2 and y >= 2 | Rc strictly between Ra and Rb (Ra < Rc < Rb or Rb < Rc < Ra), and abs(Ra - Rc) > 32 or abs(Rb - Rc) > 32 | Ra + Rb - Rc |
| x >= 2 and y >= 2 | Rc strictly between Ra and Rb, abs(Ra - Rc) <= 32 and abs(Rb - Rc) <= 32 | floor((Ra + Rb) / 2) |
| x >= 2 and y >= 2 | Rc not strictly between Ra and Rb, abs(Ra - Rc) > abs(Rb - Rc) | Ra |
| x >= 2 and y >= 2 | Rc not strictly between Ra and Rb, abs(Ra - Rc) <= abs(Rb - Rc) | Rb |

abs(v) is |v|. The last two rows give the value of S5 Formula 1 (max(Ra, Rb) when Rc is below both, min(Ra, Rb) when Rc is above both, the other value when Rc equals one of them). S5 also names the prediction 0 at the head of a line and the use of the pixel above at the left end of a line (S5, description of FIG. 16, and the note after Formula 8). The averaging rule with the threshold 32 is not in S5 or S6. M11 establishes it.

- R-ORF-31: The decoder must compute the prediction P of every pixel by the table above.
- R-ORF-32: The decoder must treat the stream as corrupt (preview path) when a decoded value s(x, y) is less than 0 or greater than 2^B - 1, with B = 12 for V5 and B = 14 for V6.

#### Worked example (vector TV-9 of section Test vectors)

The first code bits of the E-3 file of S15 (after the 7-byte header) are 4B 32 86 2A 8A 22 A2 67, that is the bits 0100 1011 0011 0010 1000 0110 0010 1010 1000 1010 0010 0010 1010 0010 0110 0111.
- Pixel (0, 0): A = 0 and N = 0, so k = max(4, 0 - 2) = 4. Bits 0, 10, 01, 0110: S = 0, L = 2, one zero closed by a 1 gives Q = 1, R = 6. M = 16 + 6 = 22, E = 22, D = 22 + 0 = 22, e = 4 * 22 + 2 = 90. P = 0, so s(0, 0) = 90. Update of parity 0: C = floor((66 + 0) / 32) = 2, A = 22, N = 0 (22 > 16).
- Pixel (1, 0): parity 1 is in its initial state, k = 4. Bits 0, 11, 001, 0100: S = 0, L = 3, Q = 2, R = 4. M = 36, E = 36, D = 36, e = 147, P = 0, s(1, 0) = 147. Update of parity 1: C = floor(108 / 32) = 3, A = 36, N = 0.
- Pixel (2, 0): parity 0 with A = 22 and N = 0: k = max(4, 5 - 2) = 4. Bits 0, 01, 1, 0001: S = 0, L = 1, Q = 0, R = 1. M = 1, E = 1, D = 1 + 2 = 3, e = 13. P = Ra = s(0, 0) = 90, so s(2, 0) = 103. Update: C = floor((9 + 2) / 32) = 0, A = 1, N = 1.
- Pixel (3, 0): parity 1 with A = 36 and N = 0: k = max(4, 6 - 2) = 4. Bits 0, 10, 1, 0100: S = 0, L = 2, Q = 0, R = 4. M = 4, E = 4, D = 4 + 3 = 7, e = 30. P = s(1, 0) = 147, so s(3, 0) = 177.
- The next four pixels of the row are 109, 197, 121 and 250.

#### Differences between V5 and V6

V6 uses the same codes, state, prediction and constants as V5 (M13). Only the stream header (8 bytes in place of 7) and the value range (0 to 16383 in place of 0 to 4095) differ.

## Levels and white balance

The decoder determines the bit depth B first, then the white level, the black levels and the white balance multipliers. All levels come from the file. No per-model table is needed.

### Bit depth and white level

- R-ORF-33: The decoder must use B = 12 for V2, V3, V4 and V5 and B = 14 for V6.
- R-ORF-34: For V1 the decoder must use as B the first value of the first present tag of this list: ImageProcessing 0x0611 (ValidBits), RawInfo 0x0611 (ValidPixelDepth), main maker-note 0x102C (ValidBits), and must use B = 12 when no tag is present or when the value of the first present tag is outside 10 to 16 (without trying the next tag).
- R-ORF-35: The decoder must use the white level w = 2^B - 1 (4095 for B = 12, 16383 for B = 14, 1023 for B = 10).

The white level 4095 is the value at which highlights clip in most files: the decoded rasters of E-500, E-330, SH-2, C5060WZ, C7070WZ, E-P1 and OM-1 hold 123 to 575219 pixels at 4095, and SP565UZ holds 246404 pixels at 4094 (M17). Some models record values that clip lower (OQ-5). Tag 0x0805 (SensorCalibration) does not predict the clip level and is not used (M17).

### Black level

The four black levels b0, b1, b2, b3 belong to the CFA positions 0, 1, 2, 3. Sources in priority order:
1. ImageProcessing tag 0x0600 (BlackLevel2), 4 values on the 12-bit scale: b_i = value_i * 2^(B - 12). This source applies only when B >= 12. No measured file with B < 12 has the tag. The 14-bit files of V6 carry values near 255 and their darkest pixels are near 1020 (M15).
2. RawInfo tag 0x0600 (BlackLevel2), 4 values on the data scale: b_i = value_i.
3. Main maker-note tag 0x1012 (BlackLevel), 4 values on the data scale: b_i = value_i (for example 32 in the E-10 file with B = 10).

Measured values (M16): 32 in E-10 (B = 10), 63 to 96 in the E-1 to E-PL3 generation, 200 in SH-2, XZ-2, XZ-10 and STYLUS1, 252 to 257 in most later models, 0 in E-M5, E-P1, E-PM2 and in the V3 compact cameras. In the files with 0 the darkest decoded values are 0 to 6: these cameras store values from which the black level is already subtracted (M15).

- R-ORF-36: The decoder must take the black levels from the first source of the list above that applies and whose tag has a count of exactly 4 with every scaled value b_i at most w / 4, and must use the preview path when no source qualifies.
- R-ORF-37: The decoder must apply b_i to the pixels of CFA position i.

### White balance multipliers

The decoder computes three multipliers mR, mG = 1 and mB. Sources in priority order:
1. ImageProcessing tag 0x0100 (WB_RBLevels), values 1 and 2: mR = value1 / G0 and mB = value2 / G0, where G0 is the value of ImageProcessing tag 0x0610 (GainBase, 256 in all files) when it lies from 64 to 4096, and 256 otherwise.
2. RawInfo tag 0x0100 (WB_RBLevelsUsed): mR = value1 / 256, mB = value2 / 256.
3. Main maker-note tags 0x1017 and 0x1018 (RedBalance, BlueBalance), both present, first value of each: mR = value / 256, mB = value / 256.
4. The generic multipliers mR = 1.944 and mB = 1.801 (the mean of the 85 measured files).

M18 shows that these levels give the white balance of the camera JPEG: in 11 files the chromaticity error against the preview is 4 to 30 times higher with unit multipliers than with the tag values, and higher with red and blue exchanged in all 11 files.

- R-ORF-38: The decoder must take mR and mB from the first source of the list above whose tags are present and give two multipliers from 0.0625 to 16 (only values 1 and 2 of WB_RBLevels count), and must use mG = 1.

Worked example: WB_RBLevels 512, 384, 256, 256 and GainBase 256 give mR = 2.0 and mB = 1.5. The E-M10 Mark II file of S16 has 512, 450, 256, 256: mR = 2.0, mB = 1.7578125.

### Normalization

For a pixel with sensor value s, black level b of its CFA position, white level w and multiplier m of its color, the normalized value is v = clip((s - b) / (w - b) * m, 0, 1). This is R-ARW-47.

- R-ORF-39: The decoder must compute v = clip((s - b) / (w - b) * m, 0, 1) for every pixel of the raster before demosaicing.

Worked example (fixture values of section Test vectors): R = 1216, G = 1856, B = 1056, b = 256, w = 4095, mR = 2.0, mB = 1.5.
- R: (1216 - 256) / 3839 * 2.0 = 0.2500651 * 2.0 = 0.5001302.
- G: (1856 - 256) / 3839 = 0.4167752.
- B: (1056 - 256) / 3839 * 1.5 = 0.2083876 * 1.5 = 0.3125814.

## Color conversion

### Matrix source

ORF files carry a 3x3 color matrix in the maker note: ImageProcessing tag 0x0200, RawInfo tag 0x0200 or main maker-note tag 0x1011 (S4 names all three ColorMatrix). The 9 values are signed 16-bit integers, row by row, with 256 meaning 1. In 79 of the 85 measured files every row sums to 256. The other row sums are 250, 246, 248 (E-1), 252, 254, 252 (E-10), 256, 258, 258 (E-410, E-510) and 257, 257, 256 (SP565UZ, SP570UZ) (M18).

The matrix converts white-balanced camera RGB to linear sRGB (S7). Each row is one output channel (R, G, B of sRGB), each column one camera channel. M18 confirms the orientation: in 24 files of all six variants the image after this matrix matches the camera JPEG with a mean chromaticity error of 0.016 to 0.116. The identity matrix gives a higher error in all 24 files, the transposed matrix in 23 of them. The exception (E-PL1) was shot with the picture mode i-Enhance (S4, CameraSettings tag 0x0520 value 5), which raises the saturation of the camera JPEG. The matrix depends on the shot (it changes with the white balance of the camera, for example between the two E-M10 Mark IV files of S10).

Generic matrix for files without a matrix tag: the element-wise mean of the row-normalized matrices of the 63 measured models, rounded to 3 decimals (M18):

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 1.418 | -0.317 | -0.101 |
| sRGB G | -0.200 | 1.407 | -0.207 |
| sRGB B | 0.030 | -0.419 | 1.389 |

### Procedure

- R-ORF-40: The decoder must take the matrix from the first present tag of the list ImageProcessing 0x0200, RawInfo 0x0200, main maker-note 0x1011 that has 9 values, must divide each value by 256, and must use the generic matrix when no tag qualifies.
- R-ORF-41: The decoder must divide each row of the matrix by the sum of that row when all three row sums are greater than 0, and must use the generic matrix otherwise.
- R-ORF-42: The decoder must compute the output (r, g, b) from the normalized, demosaiced camera values (vR, vG, vB) as r = m11 * vR + m12 * vG + m13 * vB, g = m21 * vR + m22 * vG + m23 * vB, b = m31 * vR + m32 * vG + m33 * vB, and must clip r, g and b to 0 to 1.
- R-ORF-43: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point, without the sRGB transfer function (R-ARW-53).

Worked example: the normalized values of the previous section (0.5001302, 0.4167752, 0.3125814) and the matrix 440, -152, -32, -70, 388, -62, 14, -128, 370 (ImageProcessing 0x0200 of the E-M10 Mark II file, stored as 440, 65384, 65504, 65466, 388, 65474, 14, 65408, 370). The rows sum to 256, so the normalized matrix is the matrix divided by 256.
- r = (440 * 0.5001302 - 152 * 0.4167752 - 32 * 0.3125814) / 256 = (220.0573 - 63.3498 - 10.0026) / 256 = 0.5730659.
- g = (-70 * 0.5001302 + 388 * 0.4167752 - 62 * 0.3125814) / 256 = (-35.0091 + 161.7088 - 19.3800) / 256 = 0.4192172.
- b = (14 * 0.5001302 - 128 * 0.4167752 + 370 * 0.3125814) / 256 = (7.0018 - 53.3472 + 115.6551) / 256 = 0.2707411.

A neutral value (0.25, 0.25, 0.25) stays (0.25, 0.25, 0.25) for every normalized matrix.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and read the header (R-ORF-55, R-ORF-4, R-ORF-5).
2. Read IFD0 and the IFD chain. Validate every IFD (R-ORF-6, R-ORF-7, R-ORF-8, R-ORF-9).
3. Read the Exif IFD and the maker note with its sub-IFDs (R-ORF-10 to R-ORF-13).
4. Collect the preview candidates now, so that every later failure can fall back (R-ORF-48 to R-ORF-50).
5. Classify the file (R-ORF-3). For X1 and X2 use the preview path (R-ORF-2). Check the required tags (R-ORF-14, R-ORF-15).
6. Determine B and the white level (R-ORF-33 to R-ORF-35), the black levels (R-ORF-36, R-ORF-37), the white balance (R-ORF-38), the crop (R-ORF-17, R-ORF-18), the CFA pattern (R-ORF-19) and the matrix (R-ORF-40, R-ORF-41).
7. Check all sizes against the file and the limits before allocating memory (R-ORF-56 to R-ORF-59).
8. Decode the sensor data:
   - V1: R-ORF-20, R-ORF-22.
   - V2: R-ORF-20, R-ORF-23.
   - V3: R-ORF-20, R-ORF-24, R-ORF-21.
   - V4: R-ORF-20, R-ORF-25.
   - V5 and V6: R-ORF-26 to R-ORF-32, R-ORF-60, R-ORF-61.
9. Normalize every pixel (R-ORF-39).
10. Demosaic the whole raster (R-ORF-44, R-ORF-45).
11. Convert to linear sRGB (R-ORF-42).
12. Crop to the final image area (R-ORF-17, R-ORF-45).
13. Apply the orientation (R-ORF-46, R-ORF-47).
14. Deliver the image (R-ORF-43, R-ORF-54). If any step from 5 to 13 fails, deliver the preview instead (R-ORF-53, R-ORF-65).

### Demosaicing

The decoder uses the bilinear interpolation of arw.md (section Decoding procedure, subsection Demosaicing, from S8): every missing color of a pixel is the mean of the pixels of that color among its 8 neighbors.

- R-ORF-44: The decoder must demosaic as R-ARW-54 and R-ARW-55 specify, with the whole raster (0, 0, W, H) in the role of the valid area.
- R-ORF-45: The decoder must demosaic with the CFA phase of the whole raster (R-ORF-19) and crop afterwards (R-ARW-56).

### Orientation

The decoder applies the IFD0 Orientation tag with the table of arw.md (R-ARW-58), from the definitions of S1. The measured XZ-10 file has Orientation 6. Its raw raster and its preview are both stored in the sensor orientation (landscape), and they match without rotation (M20), so the decoder rotates both.

- R-ORF-46: The decoder must read Orientation from IFD0 tag 0x0112, use 1 when the tag is absent or outside 1 to 8, and apply it after the crop.
- R-ORF-47: The decoder must map the pixels of the cropped image by the table of R-ARW-58.

## Embedded preview

### Locations

| Location | Tags | Offset origin | Content (M19) |
| --- | --- | --- | --- |
| CameraSettings IFD | 0x0101 PreviewImageStart, 0x0102 PreviewImageLength | maker-note start (OLYMPUS and OM SYSTEM styles), file start (OLYMP style) | the largest preview: 3200x2400 (E-5, E-620 and all later models), 1600x1200 (E-300 to E-510 generation), 1280x960 (E-1) |
| Main maker-note IFD | 0x0100 ThumbnailImage: the JPEG bytes are the tag value, count = length | the value offset of the entry, which follows the note style (R-ORF-12) | 160x120 thumbnail, in every measured file |
| IFD1 (OLYMP style only) | 0x0201 JPEGInterchangeFormat, 0x0202 JPEGInterchangeFormatLength | file start | the same 160x120 thumbnail |

The V3 compact cameras (C5060WZ, C7070WZ, SP510UZ to SP570UZ) and the E-10 carry only the 160x120 thumbnail. For these files the preview path gives a very small image, which is a reason to decode their raw data.

Example (E-3 file of S15): the maker note starts at 3344 and PreviewImageStart is 21232, so the preview starts at 3344 + 21232 = 24576 with the bytes FF D8 FF DB. The value 21232 read as a file offset points to other data. Example (E-1 file of S11, OLYMP style): PreviewImageStart 34156 is a file offset, and the preview there is 1280x960.

All preview streams are baseline JPEG (SOF0, 8-bit, 3 components), which the decoder decodes with S2 (baseline sequential DCT process). They start with FF D8 FF DB and have no APP1 (Exif) segment, so they carry no orientation of their own. The thumbnails of some files have bytes after the EOI marker or end without an EOI marker at the stated length (M19).

The previews are stored in the sensor orientation, like the raw data. The XZ-10 file with Orientation 6 has a landscape preview of 3200x2400 that matches the unrotated raw data (M20). The decoder therefore applies the IFD0 orientation to the preview. This differs from R-ARW-62.

### Requirements

- R-ORF-48: The decoder must collect as preview candidates the CameraSettings pair (0x0101, 0x0102) with the offset origin of the note style, the main maker-note tag 0x0100 (offset = its value offset, length = its count), and the pairs (0x0201, 0x0202) of every IFD of the chain.
- R-ORF-49: The decoder must keep a candidate only if offset + length lies inside the file, length >= 4, and the first two bytes are FF D8.
- R-ORF-50: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment, found by walking the marker segments from SOI by their length fields (S2 B.1.1 and B.2.2), must drop a candidate without such a segment, must choose the candidate with the largest width * height, must take the earlier candidate of the R-ORF-48 order when two are equal, and must not check PreviewImageValid.
- R-ORF-51: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan (R-ARW-61).
- R-ORF-52: The decoder must apply the IFD0 Orientation (R-ORF-46, R-ORF-47) to the delivered preview for all values 1 to 8.
- R-ORF-53: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 or X2, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes (R-ARW-63).
- R-ORF-54: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-ORF-43) or a preview (sRGB-encoded JPEG values, not linear) (R-ARW-64).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-ORF-53).

### Limits

- R-ORF-55: The decoder must fail for a file shorter than 8 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-ORF-56: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728). The largest measured raster is 10400 x 7796 (81 million pixels, PEN-F high-resolution shot).
- R-ORF-57: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values, and must check that every value, maker-note, sub-IFD and strip byte range lies completely inside the file before it reads it.

### Structure checks

- R-ORF-58: The decoder must fail in each of these cases:
  1. StripOffsets and StripByteCounts have different counts.
  2. For V1 to V4: a StripByteCounts value is not a multiple of RB (RB from the table of section Sensor data layout), or all strips together hold fewer than H rows.
  3. For V5 and V6: the strip is shorter than its stream header plus 1 byte.
- R-ORF-59: The decoder must never read more than 2 * W * H bytes (V1), 16 * W * H / 10 bytes (V2) or 3 * W * H / 2 bytes (V3, V4) of sensor data.
- R-ORF-60: For V5 and V6 the decoder must fail when a read of step 2, 3, 4 or 5 of the code of a pixel needs bits beyond the end of the strip (StripOffsets + StripByteCounts), and must not read bytes beyond that end.
- R-ORF-61: For V5 and V6 the decoder must fail on the first decoded value outside 0 to 2^B - 1 (R-ORF-32), and must use at least 32-bit signed arithmetic for M, E, C, D, e and the prediction.
- R-ORF-62: The decoder must clip every normalized value to 0 to 1 (R-ORF-39) and every output value to 0 to 1 (R-ORF-42).
- R-ORF-63: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0 (w - b, the row sums of the matrix, GainBase), and a failed check selects the default of the quantity, or fails (preview path) when the quantity has no default.

### Behaviour on error

- R-ORF-64: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-ORF-65: The decoder must discard a partly decoded raw image on failure and must not deliver it.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). The vectors TV-4 to TV-7, TV-9 and TV-10 are bytes of the sample files. Hex bytes are in file order.

### TV-1: header

- 0x49 0x49 0x52 0x4F 0x08 0x00 0x00 0x00: little-endian ORF, signature 0x4F52, IFD0 at offset 8 (R-ORF-4).
- 0x49 0x49 0x52 0x53 0x08 0x00 0x00 0x00: little-endian ORF, signature 0x5352 (V3 cameras).
- 0x4D 0x4D 0x4F 0x52 0x00 0x00 0x00 0x08: big-endian ORF (E-10), IFD0 at offset 8.
- 0x49 0x49 0x2A 0x00 0x08 0x00 0x00 0x00: a plain TIFF header, not ORF (error, R-ORF-4).

### TV-2: maker-note headers and sub-IFD entries

- 0x4F 0x4C 0x59 0x4D 0x50 0x00 0x02 0x00 (E-1): OLYMP style. The main IFD starts 8 bytes after the maker-note start. Its first bytes are 1B 00: 27 entries. Offsets count from the file start.
- 0x4F 0x4C 0x59 0x4D 0x50 0x55 0x53 0x00 0x49 0x49 0x03 0x00 (E-3, maker note at 3344): OLYMPUS style, little-endian. The main IFD starts at 3344 + 12 = 3356 with 08 00: 8 entries.
- 0x4F 0x4D 0x20 0x53 0x59 0x53 0x54 0x45 0x4D 0x00 0x00 0x00 0x49 0x49 0x04 0x00 (OM-1, maker note at 5512): OM SYSTEM style, little-endian, main IFD at 5512 + 16 = 5528.
- Entry 40 20 0D 00 01 00 00 00 8C 04 00 00 in the E-3 main IFD: tag 0x2040 (ImageProcessing), type 13, count 1, value 0x048C = 1164. The ImageProcessing IFD starts at 3344 + 1164 = 4508.
- Entry 20 20 0D 00 01 00 00 00 8C 01 00 00 in the E-3 main IFD: CameraSettings at 3344 + 396 = 3740.

### TV-3: CFAPattern values

- 0x02 0x00 0x02 0x00 0x00 0x01 0x01 0x02 (little-endian file): repeat 2 x 2, pattern 0 1 1 2 (RGGB).
- 0x00 0x02 0x00 0x02 0x00 0x01 0x01 0x02 (big-endian E-10 file): repeat 2 x 2, pattern 0 1 1 2.
- 0x02 0x00 0x02 0x00 0x01 0x00 0x02 0x01 (E-1): pattern 1 0 2 1 (GRBG): pixel (0, 0) green, (1, 0) red, (0, 1) blue, (1, 1) green.
- 0x02 0x00 0x02 0x00 0x02 0x01 0x01 0x00 (E-M1): pattern 2 1 1 0 (BGGR).

### TV-4: V1 samples

- Big-endian, B = 10 (E-10): 0x0C 0x00 0x0C 0xC0 0x0B 0x80 0x0D 0x80 give the samples 3072, 3264, 2944, 3456 and s = 48, 51, 46, 54.
- Little-endian, B = 12 (E-1): 0x60 0x43 0x30 0x22 0xB0 0x43 0xA0 0x23 give the samples 17248, 8752, 17328, 9120 and s = 1078, 547, 1083, 570.

### TV-5: V2 group

Input (E-500): 0x6B 0x22 0x61 0xBA 0xA2 0x5E 0x97 0x02 0x5E 0xD5 0xF2 0x5D 0xBF 0x72 0x5E 0x00.
Output pixels 0 to 9: 619, 1554, 698, 1514, 663, 1504, 725, 1503, 703, 1511.

### TV-6: V3 pairs and field order

- Input (C5060WZ): 0x2B 0xF5 0x93 0x31 0x15 0x8F. Output: 703, 1427, 785, 1423.
- Field order: H = 4 gives He = 2: stored rows 0, 1, 2, 3 hold image rows 0, 2, 1, 3. H = 1949 (C5060WZ) gives He = 975: stored row 974 holds image row 1948, stored row 975 holds image row 1.

### TV-7: V4 words

Input (XZ-2): 0x79 0xA4 0xCE 0x7A 0x0E 0x79 0xB4 0xCE 0xD8 0x4E 0x7C 0xE4.
Words: 0x7ACEA479, 0xCEB4790E, 0xE47C4ED8. Output pixels: 1964, 3748, 1948, 3764, 1936, 3812, 1988, 3800.

### TV-8: V5 synthetic stream, 10 x 4 pixels

The code bits (without the 7-byte stream header) of a 10 x 4 raster, 51 bytes, the last 6 bits of the last byte unused:

0x00 0x00 0x0F 0xB0 0x03 0x7A 0xBF 0xA4 0xD2 0x29 0x19 0x28 0x68 0x78 0x00 0x74 0x00 0x2D 0xA8 0xD4 0x14 0xB4 0xBC 0x4C 0x8C 0xD1 0x48 0xC5 0xC1 0x43 0x47 0x43 0x80 0x00 0x1E 0xFA 0xE8 0x28 0x04 0xF1 0x52 0x11 0x10 0x72 0x94 0x2C 0xC0 0x09 0xB7 0x8D 0x00

Expected values:

| | x=0 | x=1 | x=2 | x=3 | x=4 | x=5 | x=6 | x=7 | x=8 | x=9 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| y=0 | 4000 | 300 | 3990 | 310 | 3980 | 320 | 3985 | 322 | 3983 | 321 |
| y=1 | 500 | 600 | 510 | 590 | 520 | 580 | 515 | 585 | 512 | 583 |
| y=2 | 4010 | 305 | 4003 | 312 | 3985 | 318 | 2000 | 330 | 3990 | 325 |
| y=3 | 495 | 610 | 502 | 600 | 518 | 570 | 540 | 900 | 511 | 588 |

The vector exercises every row of the prediction table, escapes with k = 4 and k = 2, negative values of E, C and D, and both branches of the code length rule. Trace of every pixel. N is the run count before the pixel and C the carry used in step 7. Bits is the code split into sign bit, low bits, unary part (or the 12 escape zeros, the 15 - k bits of Q and the ignored bit), and remainder. P (case) gives the prediction and the row of the prediction table that produced it: 0, Ra, Rb, avg (floor((Ra + Rb) / 2)) or Ra+Rb-Rc.

| Pixel | k | N | Bits | S | L | Q | R | M | E | C | D | e | P (case) | Value |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 4 | 0 | 0 00 000000000000 00000111110 1 1000 | 0 | 0 | 62 | 8 | 1000 | 1000 | 0 | 1000 | 4000 | 0 (0) | 4000 |
| (1, 0) | 4 | 0 | 0 00 00001 1011 | 0 | 0 | 4 | 11 | 75 | 75 | 0 | 75 | 300 | 0 (0) | 300 |
| (2, 0) | 8 | 0 | 1 10 1 01011111 | 1 | 2 | 0 | 95 | 95 | -96 | 93 | -3 | -10 | 4000 (Ra) | 3990 |
| (3, 0) | 5 | 0 | 1 10 1 00100 | 1 | 2 | 0 | 4 | 4 | -5 | 7 | 2 | 10 | 300 (Ra) | 310 |
| (4, 0) | 5 | 0 | 1 10 1 00100 | 1 | 2 | 0 | 4 | 4 | -5 | 2 | -3 | -10 | 3990 (Ra) | 3980 |
| (5, 0) | 4 | 1 | 0 10 1 0010 | 0 | 2 | 0 | 2 | 2 | 2 | 0 | 2 | 10 | 310 (Ra) | 320 |
| (6, 0) | 4 | 1 | 0 01 1 0010 | 0 | 1 | 0 | 2 | 2 | 2 | -1 | 1 | 5 | 3980 (Ra) | 3985 |
| (7, 0) | 4 | 2 | 0 10 1 0000 | 0 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 2 | 320 (Ra) | 322 |
| (8, 0) | 4 | 2 | 1 10 1 0000 | 1 | 2 | 0 | 0 | 0 | -1 | 0 | -1 | -2 | 3985 (Ra) | 3983 |
| (9, 0) | 2 | 3 | 1 11 1 00 | 1 | 3 | 0 | 0 | 0 | -1 | 0 | -1 | -1 | 322 (Ra) | 321 |
| (0, 1) | 4 | 0 | 0 00 00000001 1101 | 0 | 0 | 7 | 13 | 125 | 125 | 0 | 125 | 500 | 0 (0) | 500 |
| (1, 1) | 4 | 0 | 0 00 0000000001 0110 | 0 | 0 | 9 | 6 | 150 | 150 | 0 | 150 | 600 | 0 (0) | 600 |
| (2, 1) | 5 | 0 | 1 10 1 01000 | 1 | 2 | 0 | 8 | 8 | -9 | 11 | 2 | 10 | 500 (Ra) | 510 |
| (3, 1) | 6 | 0 | 1 10 1 010000 | 1 | 2 | 0 | 16 | 16 | -17 | 14 | -3 | -10 | 600 (Ra) | 590 |
| (4, 1) | 4 | 1 | 0 10 1 0010 | 0 | 2 | 0 | 2 | 2 | 2 | 0 | 2 | 10 | 510 (Ra) | 520 |
| (5, 1) | 4 | 1 | 1 10 1 0010 | 1 | 2 | 0 | 2 | 2 | -3 | 0 | -3 | -10 | 590 (Ra) | 580 |
| (6, 1) | 4 | 2 | 1 11 1 0001 | 1 | 3 | 0 | 1 | 1 | -2 | 0 | -2 | -5 | 520 (Ra) | 515 |
| (7, 1) | 4 | 2 | 0 01 1 0010 | 0 | 1 | 0 | 2 | 2 | 2 | -1 | 1 | 5 | 580 (Ra) | 585 |
| (8, 1) | 2 | 3 | 0 01 1 00 | 0 | 1 | 0 | 0 | 0 | 0 | -1 | -1 | -3 | 515 (Ra) | 512 |
| (9, 1) | 2 | 3 | 1 10 1 00 | 1 | 2 | 0 | 0 | 0 | -1 | 0 | -1 | -2 | 585 (Ra) | 583 |
| (0, 2) | 4 | 0 | 0 10 1 0010 | 0 | 2 | 0 | 2 | 2 | 2 | 0 | 2 | 10 | 4000 (Rb) | 4010 |
| (1, 2) | 4 | 0 | 0 01 1 0001 | 0 | 1 | 0 | 1 | 1 | 1 | 0 | 1 | 5 | 300 (Rb) | 305 |
| (2, 2) | 4 | 1 | 0 11 1 0000 | 0 | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 3 | 4000 (avg) | 4003 |
| (3, 2) | 4 | 1 | 0 10 1 0000 | 0 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 2 | 310 (Rb) | 312 |
| (4, 2) | 4 | 2 | 1 10 1 0001 | 1 | 2 | 0 | 1 | 1 | -2 | 0 | -2 | -6 | 3991 (avg) | 3985 |
| (5, 2) | 4 | 2 | 1 10 1 0000 | 1 | 2 | 0 | 0 | 0 | -1 | 0 | -1 | -2 | 320 (Rb) | 318 |
| (6, 2) | 2 | 3 | 1 11 000000000000 0000001111011 1 11 | 1 | 3 | 123 | 3 | 495 | -496 | -1 | -497 | -1985 | 3985 (Rb) | 2000 |
| (7, 2) | 2 | 3 | 0 10 1 11 | 0 | 2 | 0 | 3 | 3 | 3 | -1 | 2 | 10 | 320 (avg) | 330 |
| (8, 2) | 7 | 0 | 0 10 00001 0100000 | 0 | 2 | 4 | 32 | 544 | 544 | -47 | 497 | 1990 | 2000 (Ra) | 3990 |
| (9, 2) | 2 | 4 | 0 00 1 00 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 325 (avg) | 325 |
| (0, 3) | 4 | 0 | 1 11 1 0001 | 1 | 3 | 0 | 1 | 1 | -2 | 0 | -2 | -5 | 500 (Rb) | 495 |
| (1, 3) | 4 | 0 | 0 10 1 0010 | 0 | 2 | 0 | 2 | 2 | 2 | 0 | 2 | 10 | 600 (Rb) | 610 |
| (2, 3) | 4 | 1 | 0 00 1 0001 | 0 | 0 | 0 | 1 | 1 | 1 | -1 | 0 | 0 | 502 (avg) | 502 |
| (3, 3) | 4 | 1 | 0 00 1 0000 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 600 (avg) | 600 |
| (4, 3) | 4 | 2 | 0 11 1 0010 | 0 | 3 | 0 | 2 | 2 | 2 | -1 | 1 | 7 | 511 (avg) | 518 |
| (5, 3) | 4 | 2 | 1 00 1 0100 | 1 | 0 | 0 | 4 | 4 | -5 | 0 | -5 | -20 | 590 (avg) | 570 |
| (6, 3) | 2 | 3 | 0 01 01 10 | 0 | 1 | 1 | 2 | 6 | 6 | 0 | 6 | 25 | 515 (Rb) | 540 |
| (7, 3) | 3 | 3 | 0 11 00000000001 001 | 0 | 3 | 10 | 1 | 81 | 81 | -1 | 80 | 323 | 577 (avg) | 900 |
| (8, 3) | 3 | 4 | 1 01 1 011 | 1 | 1 | 0 | 3 | 3 | -4 | 0 | -4 | -15 | 526 (avg) | 511 |
| (9, 3) | 5 | 0 | 1 10 001 10100 | 1 | 2 | 2 | 20 | 84 | -85 | 7 | -78 | -310 | 898 (Ra+Rb-Rc) | 588 |


### TV-9: V5 stream of a camera file

From the E-3 file of S15, strip at file offset 655360: 0x00 0x00 0x00 0x00 0x01 0x00 0x00 (stream header), then 0x4B 0x32 0x86 0x2A 0x8A 0x22 0xA2 0x67 0xA9.
Expected first values of row 0: 90, 147, 103, 177, 109, 197, 121, 250 (worked example in section Compression). The whole strip of 9009482 bytes decodes to 3720 x 2800 values from 56 to 4025 and leaves 4 bits of the last byte unused (M12).

### TV-10: V6 stream of a camera file

From the OM-1 Mark II 14-bit file of S17, strip at file offset 1817600: 0x00 0x00 0x00 0x00 0x00 0x01 0x00 0x00 (stream header), then 0x00 0x00 0x04 0x78 0x00 0x00 0x0A 0xC2 0xCC 0x7A 0x03 0x21 0x4B.
- Pixel (0, 0): k = 4. Bits 0, 00, then 12 zero bits (escape), then the 15 - 4 = 11 bits 00000010001 (Q = 17), then the ignored bit 1, then R = 1100 (12). M = 17 * 16 + 12 = 284, E = 284, D = 284, e = 1136, P = 0: s(0, 0) = 1136.
- Pixel (1, 0): escape with Q = 21, R = 0: M = 336, s(1, 0) = 1344.
- Pixel (2, 0): parity 0 with A = 284 (bitlength 9), N = 0, C = floor(852 / 32) = 26: k = 7. Bits 1, 01, 1, 0011000: S = 1, L = 1, Q = 0, R = 24. M = 24, E = -25, D = -25 + 26 = 1, e = 5, P = 1136: s(2, 0) = 1141.
- Expected first values of row 0: 1136, 1344, 1141, 1339, 1150, 1355.

### TV-11: levels, white balance and matrix

CFA cell R = 1216, G = 1856, G = 1856, B = 1056 (B = 12). Black 256. White 4095. WB_RBLevels 512, 384, 256, 256 with GainBase 256. Matrix 440, -152, -32, -70, 388, -62, 14, -128, 370. Normalized and balanced: 0.5001302, 0.4167752, 0.3125814. Output linear sRGB for a flat area of this cell (every pixel of a flat area gets the same three values after bilinear demosaicing): 0.5730659, 0.4192172, 0.2707411.

The same cell in a V6 file (values 4864, 7424, 7424, 4224, BlackLevel2 tag 256 scaled to 1024, white 16383): normalized 0.5000326, 0.4166938, 0.3125203, output 0.5729540, 0.4191354, 0.2706882.

### TV-12: orientation

Stored 3 x 2 image with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a. These are the vectors of arw.md TV-13. They apply to the raw image and to the preview (R-ORF-52).

### TV-13: preview location

E-3 file of S15: maker note at 3344 (OLYMPUS style), CameraSettings PreviewImageStart 21232 and PreviewImageLength 419789. The preview occupies bytes 24576 to 444364 and starts with 0xFF 0xD8 0xFF 0xDB. Its SOF0 segment gives 1600 x 1200. The maker-note tag 0x0100 has count 5135 and value 10480, so the thumbnail occupies bytes 3344 + 10480 = 13824 to 18958 (160 x 120). The decoder chooses the 1600 x 1200 preview.

### TV-14: CFA phase, demosaicing and crop

A 4 x 4 raster with CFAPattern 1 0 2 1 (GRBG) and these normalized values v (R-ORF-39), rows y = 0 to 3:

| | x=0 | x=1 | x=2 | x=3 |
| --- | --- | --- | --- | --- |
| y=0 | 0.10 (G) | 0.20 (R) | 0.30 (G) | 0.40 (R) |
| y=1 | 0.50 (B) | 0.60 (G) | 0.70 (B) | 0.80 (G) |
| y=2 | 0.15 (G) | 0.25 (R) | 0.35 (G) | 0.45 (R) |
| y=3 | 0.55 (B) | 0.65 (G) | 0.75 (B) | 0.85 (G) |

Bilinear demosaicing of the whole raster (R-ORF-44), then the crop rectangle (1, 1, 2, 2) (R-ORF-17, R-ORF-45), gives these camera RGB values before the matrix:
- Output (0, 0) = raster (1, 1), green: R = (0.20 + 0.25) / 2 = 0.225, G = 0.60, B = (0.50 + 0.70) / 2 = 0.60.
- Output (1, 0) = raster (2, 1), blue: R = (0.20 + 0.40 + 0.25 + 0.45) / 4 = 0.325, G = (0.30 + 0.60 + 0.80 + 0.35) / 4 = 0.5125, B = 0.70.
- Output (0, 1) = raster (1, 2), red: R = 0.25, G = (0.60 + 0.15 + 0.35 + 0.65) / 4 = 0.4375, B = (0.50 + 0.70 + 0.55 + 0.75) / 4 = 0.625.
- Output (1, 1) = raster (2, 2), green: R = (0.25 + 0.45) / 2 = 0.35, G = 0.35, B = (0.70 + 0.75) / 2 = 0.725.
Corner pixels use only neighbours inside the raster: raster (0, 0) gives R 0.20, G 0.10, B 0.50.

### Fixture recipes

The fixtures F1 to F7 are complete small ORF files, one per variant (two for V1). They were built by the measurement tools and decode to the expected values with the procedure of this document. Every fixture shows one flat CFA cell, so after bilinear demosaicing every output pixel has the same value. All fixtures have Orientation 1 and a crop rectangle equal to the whole raster.

Common content of F1, F3, F5, F6 and F7 (OLYMPUS note style, little-endian):
- Header 49 49 52 4F 08 00 00 00. IFD0 at 8 with 13 entries in ascending tag order: 0x0100 ImageWidth LONG, 0x0101 ImageLength LONG, 0x0102 BitsPerSample SHORT, 0x0103 Compression SHORT 1, 0x0106 PhotometricInterpretation SHORT 1, 0x010F Make ASCII 8 "OLYMPUS" at 0xAA, 0x0110 Model ASCII 8 "TESTCAM" at 0xB2, 0x0111 StripOffsets LONG 1 = 0x1A0, 0x0112 Orientation SHORT 1, 0x0115 SamplesPerPixel SHORT 1, 0x0116 RowsPerStrip LONG = H, 0x0117 StripByteCounts LONG 1, 0x8769 ExifIFD LONG 1 = 0xBA. Next-IFD offset 0.
- Exif IFD at 0xBA with 2 entries: 0x927C MakerNote UNDEFINED count 178 at 0xD8, 0xA302 CFAPattern UNDEFINED count 8 at 0x18A (bytes 02 00 02 00 00 01 01 02). Next-IFD offset 0.
- Maker note at 0xD8: header 4F 4C 59 4D 50 55 53 00 49 49 03 00, main IFD with 1 entry: 0x2040 type 13 count 1 value 0x1E (ImageProcessing IFD at 0xD8 + 0x1E = 0xF6). ImageProcessing IFD with 9 entries: 0x0100 SHORT 4 = 512 384 256 256, 0x0200 SHORT 9 = 440 65384 65504 65466 388 65474 14 65408 370, 0x0600 SHORT 4 = 256 256 256 256, 0x0610 SHORT 1 = 256, 0x0611 SHORT 2 = 12 0, 0x0612 SHORT 2 = 0 0, 0x0613 SHORT 2 = 0 0, 0x0614 LONG 1 = W, 0x0615 LONG 1 = H. Value offsets inside the maker note count from 0xD8.
- Sensor strip at 0x1A0.

F6 (V5, 4 x 4 pixels), complete file of 452 bytes:

0000: 49 49 52 4F 08 00 00 00 0D 00 00 01 04 00 01 00
0010: 00 00 04 00 00 00 01 01 04 00 01 00 00 00 04 00
0020: 00 00 02 01 03 00 01 00 00 00 10 00 00 00 03 01
0030: 03 00 01 00 00 00 01 00 00 00 06 01 03 00 01 00
0040: 00 00 01 00 00 00 0F 01 02 00 08 00 00 00 AA 00
0050: 00 00 10 01 02 00 08 00 00 00 B2 00 00 00 11 01
0060: 04 00 01 00 00 00 A0 01 00 00 12 01 03 00 01 00
0070: 00 00 01 00 00 00 15 01 03 00 01 00 00 00 01 00
0080: 00 00 16 01 04 00 01 00 00 00 04 00 00 00 17 01
0090: 04 00 01 00 00 00 24 00 00 00 69 87 04 00 01 00
00A0: 00 00 BA 00 00 00 00 00 00 00 4F 4C 59 4D 50 55
00B0: 53 00 54 45 53 54 43 41 4D 00 02 00 7C 92 07 00
00C0: B2 00 00 00 D8 00 00 00 02 A3 07 00 08 00 00 00
00D0: 8A 01 00 00 00 00 00 00 4F 4C 59 4D 50 55 53 00
00E0: 49 49 03 00 01 00 40 20 0D 00 01 00 00 00 1E 00
00F0: 00 00 00 00 00 00 09 00 00 01 03 00 04 00 00 00
0100: 90 00 00 00 00 02 03 00 09 00 00 00 98 00 00 00
0110: 00 06 03 00 04 00 00 00 AA 00 00 00 10 06 03 00
0120: 01 00 00 00 00 01 00 00 11 06 03 00 02 00 00 00
0130: 0C 00 00 00 12 06 03 00 02 00 00 00 00 00 00 00
0140: 13 06 03 00 02 00 00 00 00 00 00 00 14 06 04 00
0150: 01 00 00 00 04 00 00 00 15 06 04 00 01 00 00 00
0160: 04 00 00 00 00 00 00 00 00 02 80 01 00 01 00 01
0170: B8 01 68 FF E0 FF BA FF 84 01 C2 FF 0E 00 80 FF
0180: 72 01 00 01 00 01 00 01 00 01 02 00 02 00 00 01
0190: 01 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00
01A0: 00 00 00 00 01 00 00 00 00 04 E0 00 00 0E C2 4D
01B0: CA A0 00 00 76 00 00 00 86 25 54 97 10 10 10 10
01C0: 10 10 10 10

Expected sensor values: every row 1216 1856 1216 1856 (even rows) and 1856 1056 1856 1056 (odd rows). The code bits after the stream header fill exactly 29 bytes (232 bits). Expected output for every pixel: 0.5730659, 0.4192172, 0.2707411 (TV-11).

The other fixtures of this layout differ from F6 only in the bytes listed below and in the strip, which starts at 0x1A0 in all of them. In the table, offsets and byte values are hexadecimal ("0x96 = 20" means that the byte at offset 0x96 is 0x20):

| Fixture | Variant | Size | Bytes that differ from F6 before 0x1A0 | Strip bytes from 0x1A0 | Expected output |
| --- | --- | --- | --- | --- | --- |
| F1 | V1, 4 x 4, B = 12 | 448 | 0x96 = 20 (StripByteCounts 32) | 00 4C 00 74 00 4C 00 74 00 74 00 42 00 74 00 42, then the same 16 bytes again | as F6 |
| F3 | V2, 10 x 2 | 448 | 0x12 = 0A (W = 10), 0x1E = 02 (H = 2), 0x8A = 02 (RowsPerStrip), 0x96 = 20 (32 bytes), 0x154 = 0A (CropWidth), 0x160 = 02 (CropHeight) | C0 04 74 C0 04 74 C0 04 74 C0 04 74 C0 04 74 00 40 07 42 40 07 42 40 07 42 40 07 42 40 07 42 00 | as F6 |
| F5 | V4, 8 x 2 | 440 | 0x12 = 08, 0x1E = 02, 0x2A = 0C (BitsPerSample 12), 0x8A = 02, 0x96 = 18 (24 bytes), 0x154 = 08, 0x160 = 02 | 4C 40 07 4C 07 4C 40 07 40 07 4C 40 74 20 04 74 04 74 20 04 20 04 74 20 | as F6 |
| F7 | V6, 4 x 4, B = 14 | 454 | 0x96 = 26 (38 bytes), 0x130 = 0E (ValidBits 14) | 00 00 00 00 00 01 00 00 00 00 13 20 00 00 3A 42 4E 32 AD 00 00 1D 20 00 00 21 42 55 B2 62 10 10 10 10 10 10 10 10 | 0.5729540, 0.4191354, 0.2706882 |

F1 sensor values: 1216 1856 1216 1856 / 1856 1056 1856 1056 / repeated. F3: 1216 1856 (5 times) / 1856 1056 (5 times). F5: 1216 1856 (4 times) / 1856 1056 (4 times). F7: 4864 7424 4864 7424 / 7424 4224 7424 4224 / repeated.

F2 (V1, big-endian, B = 10, OLYMP note style with the levels in the main maker-note IFD: ColorMatrix, BlackLevel 64 64 64 64, RedBalance 512 64, BlueBalance 384 64, ValidBits 10 0), 4 x 2 pixels, complete file of 352 bytes:

0000: 4D 4D 4F 52 00 00 00 08 00 0D 01 00 00 04 00 00
0010: 00 01 00 00 00 04 01 01 00 04 00 00 00 01 00 00
0020: 00 02 01 02 00 03 00 00 00 01 00 10 00 00 01 03
0030: 00 03 00 00 00 01 00 01 00 00 01 06 00 03 00 00
0040: 00 01 00 01 00 00 01 0F 00 02 00 00 00 08 00 00
0050: 00 AA 01 10 00 02 00 00 00 08 00 00 00 B2 01 11
0060: 00 04 00 00 00 01 00 00 01 50 01 12 00 03 00 00
0070: 00 01 00 01 00 00 01 15 00 03 00 00 00 01 00 01
0080: 00 00 01 16 00 04 00 00 00 01 00 00 00 02 01 17
0090: 00 04 00 00 00 01 00 00 00 10 87 69 00 04 00 00
00A0: 00 01 00 00 00 BA 00 00 00 00 4F 4C 59 4D 50 55
00B0: 53 00 54 45 53 54 43 41 4D 00 00 02 92 7C 00 07
00C0: 00 00 00 64 00 00 00 D8 A3 02 00 07 00 00 00 08
00D0: 00 00 01 3C 00 00 00 00 4F 4C 59 4D 50 00 00 02
00E0: 00 05 10 11 00 03 00 00 00 09 00 00 01 22 10 12
00F0: 00 03 00 00 00 04 00 00 01 34 10 17 00 03 00 00
0100: 00 02 02 00 00 40 10 18 00 03 00 00 00 02 01 80
0110: 00 40 10 2C 00 03 00 00 00 02 00 0A 00 00 00 00
0120: 00 00 01 B8 FF 68 FF E0 FF BA 01 84 FF C2 00 0E
0130: FF 80 01 72 00 40 00 40 00 40 00 40 00 02 00 02
0140: 00 01 01 02 00 00 00 00 00 00 00 00 00 00 00 00
0150: 4C 00 74 00 4C 00 74 00 74 00 42 00 74 00 42 00

Expected sensor values: 304 464 304 464 / 464 264 464 264. Normalized and balanced: 0.5005214, 0.4171011, 0.3128259 (black 64, white 1023). Expected output for every pixel: 0.5735141, 0.4195451, 0.2709528.

F4 (V3, signature 52 53, BitsPerSample 12, PhotometricInterpretation 2, two strips of 2 stored rows with 4 unused bytes between them, OLYMP note style as in F2 but little-endian with BlackLevel 256 and ValidBits 12), 4 x 4 pixels, complete file of 380 bytes:

0000: 49 49 52 53 08 00 00 00 0D 00 00 01 04 00 01 00
0010: 00 00 04 00 00 00 01 01 04 00 01 00 00 00 04 00
0020: 00 00 02 01 03 00 01 00 00 00 0C 00 00 00 03 01
0030: 03 00 01 00 00 00 01 00 00 00 06 01 03 00 01 00
0040: 00 00 02 00 00 00 0F 01 02 00 08 00 00 00 AA 00
0050: 00 00 10 01 02 00 08 00 00 00 B2 00 00 00 11 01
0060: 04 00 02 00 00 00 BA 00 00 00 12 01 03 00 01 00
0070: 00 00 01 00 00 00 15 01 03 00 01 00 00 00 01 00
0080: 00 00 16 01 04 00 01 00 00 00 02 00 00 00 17 01
0090: 04 00 02 00 00 00 C2 00 00 00 69 87 04 00 01 00
00A0: 00 00 CA 00 00 00 00 00 00 00 4F 4C 59 4D 50 55
00B0: 53 00 54 45 53 54 43 41 4D 00 60 01 00 00 70 01
00C0: 00 00 0C 00 00 00 0C 00 00 00 02 00 7C 92 07 00
00D0: 64 00 00 00 E8 00 00 00 02 A3 07 00 08 00 00 00
00E0: 4C 01 00 00 00 00 00 00 4F 4C 59 4D 50 00 02 00
00F0: 05 00 11 10 03 00 09 00 00 00 32 01 00 00 12 10
0100: 03 00 04 00 00 00 44 01 00 00 17 10 03 00 02 00
0110: 00 00 00 02 40 00 18 10 03 00 02 00 00 00 80 01
0120: 40 00 2C 10 03 00 02 00 00 00 0C 00 00 00 00 00
0130: 00 00 B8 01 68 FF E0 FF BA FF 84 01 C2 FF 0E 00
0140: 80 FF 72 01 00 01 00 01 00 01 00 01 02 00 02 00
0150: 00 01 01 02 00 00 00 00 00 00 00 00 00 00 00 00
0160: 4C 07 40 4C 07 40 4C 07 40 4C 07 40 00 00 00 00
0170: 74 04 20 74 04 20 74 04 20 74 04 20

The first strip (offset 0x160) holds stored rows 0 and 1 (image rows 0 and 2): 4C 07 40 4C 07 40 = 1216 1856 1216 1856 twice. The second strip (offset 0x170) holds stored rows 2 and 3 (image rows 1 and 3): 74 04 20 = 1856 1056. Expected output as F6.

## Open questions

Each item states what no permitted source or measurement confirms, and the behavior the decoder must take.

- OQ-1 Width of the escape field. In all measured escapes Q was below 2^(14 - k) (largest Q: 86 in E-3, 103 in OM-1, 230 in the V6 file), so the first bit of the 15 - k bit field was always 0. M10 found six readings of the escape that decode the E-3 and OM-1 files equally, and five of them decode the V6 file (M13), because they differ only for codes that the files do not contain (an escaped Q of 2^(14 - k) or more, or a code with exactly 11 zeros and no escape). The reading of R-ORF-29 is the one that keeps Q < 2^(15 - k) and M < 2^15. Fallback: R-ORF-29 as written.
- OQ-2 The bit after an escaped Q. It was 1 in all 752593 measured escapes. Its purpose is unknown. Fallback: read it and ignore its value (step 4).
- OQ-3 Meaning of the stream header bytes (00 00 00 00 01 00 00 in V5, 00 00 00 00 00 01 00 00 in V6). They are constant in all 68 compressed files of S10. A strip with other first bytes is X1. Fallback: R-ORF-3.
- OQ-4 Order of the four black levels. R-ORF-37 assigns value i to CFA position i. A file whose values differ between positions has differences of at most 2 (for example 252 254 254 252 in E-M1X), so another order changes the result by less than 0.06 percent of the range. Fallback: R-ORF-37.
- OQ-5 Clip level below 2^B - 1. In some files the highest values pile up below 4095: near 3791 to 3796 in E-M5, 3952 to 3958 in E-PL1, at 4092 in XZ-2 (whose top values are multiples of 4) and at 4094 in SP510UZ and SP565UZ (M17). With w = 4095 the clipped highlights of these files stay slightly below 1 and can show a tint after white balance. No tag predicts the clip level: SensorCalibration (0x0805) value 1 is 4044 in E-M5 and 3645 in E-P1, whose values reach 4095. Fallback: R-ORF-35 (w = 2^B - 1) and the clipping of R-ORF-39.
- OQ-6 Black level already subtracted. E-M5, E-P1 and E-PM2 have BlackLevel2 0 0 0 0, and their darkest values are 0 to 6 (M15), while E-PL1 and E-P2 of the same years carry 64 and 65. The tag and the data agree in every measured file, so the decoder needs no rule by model. Fallback: R-ORF-36.
- OQ-7 Values 3 and 4 of WB_RBLevels (256 to 260, "presumably RBGG levels" in S4) and value 2 of RedBalance and BlueBalance (62 or 64). Not used. Values 3 and 4 as green gains change green by at most 1.6 percent. Fallback: mG = 1 (R-ORF-38).
- OQ-8 Target space of the color matrix. The orientation and the effect of the matrix are measured against the camera JPEG (M18), which is sRGB. No source states the target space. Fallback: linear sRGB with D65 (R-ORF-43).
- OQ-9 AspectFrame (ImageProcessing 0x1113). When the user chose 3:2, 16:9 or another ratio, the raw data and the preview still cover the full 4:3 area, and AspectFrame gives the chosen frame inside the crop rectangle (for example 0, 164, 3999, 2835 in TG-6). Fallback: not applied (R-ORF-18).
- OQ-10 Lens corrections. The raw data of the compact cameras show the lens distortion and the vignetting that the camera JPEG corrects (SH-2, M18). The maker note holds lens data that no permitted source describes. Fallback: no lens correction.
- OQ-11 Big-endian files. Only E-10 (V1) was measured. A big-endian file of another layout is X2. Fallback: preview path.
- OQ-12 Models without a CC0 sample. The archive has samples of C5050Z, C8080WZ, E-20, E-30, E-520, E-600, SP350 and SP500UZ only under other licenses, and the author did not measure them. Their files are decoded when their structure matches a rule of R-ORF-3 and use the preview path otherwise.
- OQ-13 V1 bit depth. Measured B = 10 and B = 12. A few samples per file have nonzero low bits (793 of 10.7 million in E-400, 3 in E-10, M5). R-ORF-22 discards these bits. Fallback: R-ORF-22 and R-ORF-34.
- OQ-14 ORI files. The companion files of high-resolution shots (extension .ORI) were not measured. Fallback: the decoder classifies any file by its content (R-ORF-3).
- OQ-15 Compressed data in several strips, with odd W, or with another header. Not observed. Fallback: X1, preview path.
- OQ-16 Highlights. Values above the white level do not occur. White-balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-ORF-39.
- OQ-17 Files without a black level tag. All 85 measured files carry one of the three tags. Fallback: preview path (R-ORF-36).
- OQ-18 Generic white balance and matrix. They are means over the measured files and apply only to files without the tags, of which none was measured. Fallback: R-ORF-38 source 4 and the generic matrix of R-ORF-40.
- OQ-19 Orientation of the previews. One measured file has an orientation other than 1 (XZ-10, value 6), and its preview is stored unrotated. Fallback: R-ORF-52 for all values.
- OQ-20 14-bit data outside high-resolution shots. V6 was measured only in high-resolution shots of OM-1 Mark II and OM-3. Fallback: R-ORF-3 classifies by the stream header.
