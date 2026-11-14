# Pentax PEF raw format: decoder specification

Document: specs/raw/pef.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite PEF decoder (clean-room step 1 of 2).

This document describes the Pentax PEF raw file format as facts. It is written for an implementer who has only this document, TIFF 6.0 (S1) and ITU-T T.81 (S2). Every normative statement is traceable to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas. Where PEF shares a mechanism with Sony ARW, this document restates the rule and names the matching part of specs/raw/arw.md for comparison. The rules in this document are complete without arw.md.

### Conventions

- Requirements have the form R-PEF-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x4D 0x4D or "4D 4D".
- Bit 0 is the least significant bit of a value. In a bit stream, "the first bit" of a byte is its most significant bit (bit 7).
- a / b is exact division (a real number, no rounding). floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0).
- Pixel coordinates are (x, y): x is the column, y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (tags 0x0100 and 0x0101 of IFD0). BPS is the value of tag 0x0102 of IFD0.
- clip(v, a, b) is a when v < a, b when v > b, and v otherwise.

### Glossary

- PEF: the raw file format of Pentax digital SLR and medium-format cameras (Asahi Pentax, Pentax Imaging, Ricoh Imaging). PEF is a TIFF file (S1, S7).
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2).
- IFD0, IFD1, IFD2: the first, second and third IFD of the main IFD chain.
- Exif IFD: the IFD at the offset in IFD0 tag 0x8769 (S4).
- Maker note: the value of Exif tag 0x927C (S4). In PEF it holds a Pentax IFD (S6).
- M: the file offset of the first byte of the maker note.
- AOC form, PENTAX form: the two maker-note layouts of section Container layout.
- CFA: color filter array. All measured color PEF files use a 2x2 Bayer pattern (S13).
- CFA position: the index 2 * (y mod 2) + (x mod 2) of a pixel inside its 2x2 repeat cell (0, 1, 2 or 3).
- Variant: one storage method of the sensor data. This document defines four variants, P1 to P4, in section Scope and variants.
- Pentax Huffman coding: the compression of variants P3 and P4 (TIFF Compression 65535, which S5 names "Pentax PEF Compressed").
- SSSS: the difference category of a coded pixel (the term of S2 Annex F, Table F.1): a difference D with SSSS = s lies in the range 2^(s-1) <= |D| <= 2^s - 1, and SSSS = 0 means D = 0.
- Crop rectangle: the part of the stored raster that forms the final image.
- Black level: the sensor value that represents zero light. White level: the sensor value treated as saturation.
- WB multipliers: the per-channel gains that make a neutral surface give equal red, green and blue values.
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S14). The decoder output is linear sRGB.
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Container layout, Metadata tags, Sensor data layout, Decoding procedure (orientation) |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Compression (difference categories, Table F.1, EXTEND procedure of F.2.2.1), Embedded preview |
| S3 | Digital Negative (DNG) Specification 1.6.0.0, Adobe, December 2021 (copy read at the URL, official landing page https://helpx.adobe.com/camera-raw/digital-negative.html) | https://paulbourke.net/dataformats/dng/dng_spec_1_6_0_0.pdf | official | 2026-11-14 | Metadata tags (PhotometricInterpretation 32803 and 34892, ActiveArea, DefaultCropOrigin, DefaultCropSize, BlackLevel, WhiteLevel: used to read the camera DNG files of S26) |
| S4 | Exif Version 2.32, CIPA DC-X008-Translation-2019 | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-14 | Container layout (Exif IFD, MakerNote), Sensor data layout (CFAPattern 0xA302 structure and color codes) |
| S5 | ExifTool Tag Names: EXIF tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/EXIF.html | community documentation | 2026-11-14 | Scope and variants (Compression 65535 "Pentax PEF Compressed", 32773 "PackBits"), Embedded preview (0x0201/0x0202 in IFD2 of PEF) |
| S6 | ExifTool Tag Names: Pentax tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/Pentax.html | community documentation | 2026-11-14 | Metadata tags (maker-note tag names 0x0008, 0x0038, 0x0039, 0x007E, 0x0200, 0x0201, 0x0220, 0x0243), Open questions |
| S7 | PRONOM format registry entry fmt/1781, Pentax PEF Image File, The National Archives (UK) | https://www.nationalarchives.gov.uk/PRONOM/fmt/1781 | community documentation | 2026-11-14 | Container layout (TIFF base, big-endian signature, "AOC" maker note) |
| S8 | Ricoh Imaging, PENTAX K-3 Mark III Specifications | https://www.ricoh-imaging.co.jp/english/products/k-3-3/spec/ | vendor | 2026-11-14 | Scope and variants (RAW 14bit PEF/DNG), Sensor data layout (RAW 6192x4128) |
| S9 | Ricoh Imaging, PENTAX K-1 Mark II Specifications | https://www.ricoh-imaging.co.jp/english/products/k-1-2/spec/ | vendor | 2026-11-14 | Scope and variants (RAW 14bit), Sensor data layout (RAW 7360x4912 full frame, 4800x3200 APS-C) |
| S10 | Ricoh Imaging, PENTAX K-3 Mark III Features: Pixel Shift Resolution System | https://www.ricoh-imaging.co.jp/english/products/k-3-3/feature/02.html | vendor | 2026-11-14 | Scope and variants (Pixel Shift captures four images and synthesizes them) |
| S11 | DXOMARK camera sensor Measurements pages, tab Color response (one page per model, URL pattern https://www.dxomark.com/Cameras/Pentax/<name>---Measurements, names in the Color conversion table) | https://www.dxomark.com/Cameras/Pentax/K-5---Measurements | community documentation | 2026-11-14 | Color conversion (matrices, D50 white balance scales) |
| S12 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear demosaicing) |
| S13 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-14 | Sensor data layout (Bayer arrangement) |
| S14 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (output primaries and D65 white) |
| S15 | Measurement: *ist D sample pentax.PEF, public domain (CC0) | https://raw.pixls.us/data/Pentax/ist%20D/pentax.PEF | measurement | 2026-11-14 | P1, records M1, M7, M8, M17 |
| S16 | Measurement: *ist DS sample IMGP2333.pef (CC0) | https://raw.pixls.us/data/Pentax/ist%20DS/IMGP2333.pef | measurement | 2026-11-14 | P2, records M1, M6, M17 |
| S17 | Measurement: K10D sample _IGP7284.PEF (CC0) | https://raw.pixls.us/data/Pentax/K10D/_IGP7284.PEF | measurement | 2026-11-14 | P3, records M1 to M5, M9, M12, M13, M17 |
| S18 | Measurement: K-5 sample _MAR0544.PEF (CC0) | https://raw.pixls.us/data/Pentax/K-5/_MAR0544.PEF | measurement | 2026-11-14 | P3 (14-bit, BGGR), records M5, M8, M11, M12, M17 |
| S19 | Measurement: K-S2 sample IMGP3369.PEF (CC0) | https://raw.pixls.us/data/Pentax/K-S2/IMGP3369.PEF | measurement | 2026-11-14 | P3 (12-bit, little-endian), records M5, M10, M17 |
| S20 | Measurement: K-70 sample IMGP6854.PEF (CC0) | https://raw.pixls.us/data/Pentax/K-70/IMGP6854.PEF | measurement | 2026-11-14 | records M8, M11, M17, Open questions OQ-3 |
| S21 | Measurement: K-1 Mark II sample _K1I9785.PEF, APS-C crop mode (CC0) | https://raw.pixls.us/data/Pentax/K-1%20Mark%20II/_K1I9785.PEF | measurement | 2026-11-14 | records M5, M9, M17 |
| S22 | Measurement: K-3 Mark III samples IMGP9854.PEF (single frame) and IMGP9855.PEF (Pixel Shift) (CC0) | https://raw.pixls.us/data/Pentax/K-3%20Mark%20III/ | measurement | 2026-11-14 | records M4, M5, M9, M15, M17 |
| S23 | Measurement: K-3 Mark III Monochrome sample K3III.PEF (CC0) | https://raw.pixls.us/data/Pentax/K-3%20Mark%20III%20Monochrome/K3III.PEF | measurement | 2026-11-14 | P4, records M16, M17 |
| S24 | Measurement: full files of *ist DL, K100D, K100D Super, K110D, K20D, K200D, K2000, K-7, K-x, K-r, K-5 II, K-5 II s, 645D, K-S1, K-3, K-3 II, KF, KP, K-1, K-1 Mark II, 645Z (one CC0 file each, measured and not kept) | https://raw.pixls.us/data/Pentax/ | measurement | 2026-11-14 | records M3, M5, M6, M8 to M13 |
| S25 | Measurement: container survey, first 1 MiB of 38 CC0 PEF files of 28 models, including the Pixel Shift files of K-1, K-1 Mark II and K-3 Mark III and one K-3 Mark III "HDR Advanced" file, plus the SubIFD headers of the K-3 Mark III Pixel Shift file | https://raw.pixls.us/data/Pentax/ | measurement | 2026-11-14 | records M1 to M3, M8, M9, M14, M15 |
| S26 | Measurement: first 256 KiB of 25 CC0 DNG files written by Pentax cameras (in-camera DNG of K10D, K20D, K200D, K-7, K-x, K-5, K-5 II, K-5 II s, 645D, 645Z, K-01, K-30, K-50, K-500, K-3, K-3 II, K-S1, K-S2, K-70, KP, K-1, K-1 Mark II in both sensor modes, K-3 Mark III, K-3 Mark III Monochrome) | https://raw.pixls.us/data/Pentax/ | measurement | 2026-11-14 | records M9 to M11, Color conversion (Model strings of K-01, K-30, K-50, K-500) |

Sample files are from the public raw sample archive raw.pixls.us. The archive marks each file with its license. The author used only files marked CC0 (public domain) and checked each downloaded file by its SHA-256 checksum against the archive index. The author inspected the files with measurement tools that the author wrote from S1 and S2 for this purpose. The tools are not part of the deliverable. One comparison decoded the embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S15 to S25): every PEF file is a TIFF file with IFD0 at offset 8. Files of the *ist D to K-5 II s generations and the 645D start with 4D 4D 00 2A 00 00 00 08 (big-endian). Files of the K-S1 and later and the 645Z start with 49 49 2A 00 08 00 00 00 (little-endian). IFD0 is the raw image: it holds ImageWidth, ImageLength, BitsPerSample, Compression, PhotometricInterpretation, one strip (StripOffsets and StripByteCounts with one value each, RowsPerStrip = H), Make, Model, the Exif IFD pointer 0x8769 and, in all files except *ist D, Orientation. IFD1 holds a 160x120 JPEG thumbnail. IFD2 holds a full-size JPEG. No IFD has a NewSubfileType tag. IFD0 has a SubIFDs tag (0x014A) only in the Pixel Shift files (3 offsets) and in the "HDR Advanced" file (2 offsets). The Exif IFD holds the MakerNote (0x927C, type UNDEFINED) and, in all color files, CFAPattern (0xA302).
- M2 (maker note, S25): 16 files (all big-endian files) have a maker note that starts with 41 4F 43 00 4D 4D ("AOC", a zero byte, "MM"). 22 files (all little-endian files) have a maker note that starts with 50 45 4E 54 41 58 20 00 49 49 ("PENTAX ", a zero byte, "II"). The IFD entry count follows these 6 or 10 bytes. For each file, the smallest out-of-line value offset stored in the maker-note IFD was compared with the end of that IFD (IFD start + 2 + 12 * N + 4). In all 16 AOC files the stored offset equals the end of the IFD with origin 0 (file start). In all 22 PENTAX files it equals the end of the IFD only when M is added (origin M). Example: K10D, M = 684, N = 91, IFD end 1788, smallest stored offset 1788. K-3 Mark III, M = 902, N = 99, IFD end 2106, smallest stored offset 1204 = 2106 - 902.
- M3 (Huffman table, S24, S25): maker-note tag 0x0220 is present in every file with Compression 65535 and absent in every other file. Its type is UNDEFINED. Its count is 53 when BPS = 12 and 59 when BPS = 14, which is 14 + 3 * N with N = BPS + 1. The first 16-bit value (maker-note byte order) is 1 when N = 13 and 3 when N = 15. Bytes 14 to 14 + 2N - 1 read as N 16-bit values hold codes, left-aligned in 12 bits (every value is below 4096 and its low 12 - L bits are zero). The last N bytes hold the code lengths L, from 2 to 12. In every table the codes are prefix-free and the sum of 2^(-L) over all entries is exactly 1. Five distinct tables occur: K10D, K20D, K200D, K-7, K-x, K-r share one 13-entry table. K2000 has another 13-entry table. K-S1 and K-S2 share a third. K-5, K-5 II, K-5 II s and 645D share a 15-entry table. K-3, K-3 II, K-70, KF, KP, K-1, K-1 Mark II, K-3 Mark III, K-3 Mark III Monochrome and 645Z share another 15-entry table.
- M4 (Huffman stream hypotheses, S17, S22): the K10D strip (8126786 bytes) and the K-3 Mark III strip (28893388 bytes) were decoded under alternative hypotheses. Only the rules of this document (R-PEF-29 to R-PEF-34) give values inside 0 to 2^BPS - 1 for every pixel:

| Hypothesis | K10D value range | K-3 Mark III value range |
| --- | --- | --- |
| this document (predict from x - 2, row start from y - 2, start value 0) | 0 to 4095 | 0 to 16383 |
| predict from x - 1 | -390 to 8495 | -252 to 32803 |
| row start predicted from y - 1 | 0 to 5829 | 0 to 16679 |
| start value 2048 at the four first pixels | 2048 to 6143 | not tested |
| row start predicted from 0 (no vertical prediction) | -1743 to 4095 | not tested |
| T.81 byte stuffing (FF 00 read as FF) | -13676 to 12867 | -17449 to 55768 |
| every row starts at a byte boundary | -4493 to 8018 | -628678 to 86975 |

  With the rules of this document the K10D stream uses all 65014288 bits (8 * 8126786) of its strip, and the K-3 Mark III stream leaves the last 6 bits of its last byte unused.
- M5 (Huffman decoding of 25 files, S17 to S24): every file with Compression 65535 decodes with the rules of this document. The last coded bit lies in the last byte of the strip in every file: StripByteCounts equals ceil(coded bits / 8), and 0 to 7 bits of the last byte are unused. No decoded value lies outside 0 to 2^BPS - 1. Per-file results (unused bits = 8 * strip bytes minus coded bits):

| Model | Byte order | BPS | W x H | Strip bytes | Unused bits | Min | Max |
| --- | --- | --- | --- | --- | --- | --- | --- |
| K10D | MM | 12 | 3936 x 2624 | 8126786 | 0 | 0 | 4095 |
| K20D | MM | 12 | 4864 x 3136 | 13686775 | 3 | 0 | 4095 |
| K200D | MM | 12 | 3936 x 2624 | 9384916 | 1 | 0 | 4095 |
| K2000 | MM | 12 | 3936 x 2624 | 11124197 | 5 | 0 | 4095 |
| K-7 | MM | 12 | 4736 x 3136 | 19054974 | 7 | 0 | 4081 |
| K-x | MM | 12 | 4352 x 2868 | 12797827 | 3 | 0 | 3872 |
| K-r | MM | 12 | 4352 x 2868 | 11808401 | 4 | 35 | 4094 |
| K-5 | MM | 14 | 4992 x 3284 | 17745780 | 7 | 0 | 16383 |
| K-5 II | MM | 14 | 4992 x 3284 | 20775686 | 3 | 0 | 16383 |
| K-5 II s | MM | 14 | 4992 x 3284 | 14384192 | 3 | 0 | 16383 |
| 645D | MM | 14 | 7424 x 5552 | 57578397 | 1 | 0 | 15862 |
| K-S1 | II | 12 | 5504 x 3664 | 18842426 | 5 | 10 | 4094 |
| K-S2 | II | 12 | 5504 x 3664 | 19350788 | 5 | 195 | 4075 |
| K-3 | II | 14 | 6080 x 4032 | 28193043 | 6 | 0 | 5186 |
| K-3 II | II | 14 | 6080 x 4032 | 34890185 | 6 | 0 | 16317 |
| K-70 | II | 14 | 6080 x 4064 | 25582599 | 4 | 0 | 16315 |
| KF | II | 14 | 6080 x 4064 | 27514736 | 1 | 0 | 16378 |
| KP | II | 14 | 6112 x 4060 | 29109927 | 6 | 93 | 16383 |
| K-1 | II | 14 | 7392 x 4950 | 42304484 | 3 | 41 | 16381 |
| K-1 Mark II | II | 14 | 7392 x 4950 | 41151297 | 7 | 47 | 16382 |
| K-1 Mark II (APS-C) | II | 14 | 4832 x 3238 | 19143154 | 0 | 53 | 7308 |
| K-3 Mark III | II | 14 | 6304 x 4224 | 28893388 | 6 | 0 | 16383 |
| K-3 Mark III (Pixel Shift, IFD0) | II | 14 | 6304 x 4224 | 29266537 | 0 | 0 | 16383 |
| K-3 Mark III Monochrome | II | 14 | 6304 x 4224 | 30916224 | 4 | 0 | 16382 |
| 645Z | II | 14 | 8384 x 6208 | 63428349 | 6 | 0 | 16317 |

- M6 (P2 bit order, S16, S24): the *ist DS, *ist DL, K100D, K100D Super and K110D files have Compression 32773 and StripByteCounts 9229440 = 3 * 3040 * 2024 / 2. Read as 12-bit values with the first pixel in the high 12 bits of each 3-byte group (R-PEF-26), the *ist DS raster correlates with the embedded preview with 0.981, 0.985, 0.984 for R, G, B. The reading with the first pixel in the low 12 bits gives a mean channel correlation of at most 0.41 for every CFA phase. The decoded values range from 123 to 4095 and the darkest values are near the BlackPoint 128. The bytes are not PackBits data: the stored length equals the unpacked size exactly.
- M7 (P1, S15): the *ist D file has Compression 1, BPS 12 and StripByteCounts 12305920 = 2 * 3040 * 2024. Read as big-endian 16-bit values, the samples range from 129 to 2929. Read as little-endian, 93 percent of the samples exceed 4095.
- M8 (CFA, S15 to S25): CFAPattern (Exif 0xA302) has type UNDEFINED, count 8. Its first four bytes are 00 02 00 02 in all files, also in the little-endian files. The last four bytes are 02 01 01 00 (BGGR) for K20D, K-7, K-x, K-r, K-5, K-5 II, K-5 II s and 645D, and 00 01 01 02 (RGGB) for all other color models. For 28 of 29 color files, this pattern gives the highest mean channel correlation with the preview of the four Bayer phases. For the K-70 file (a nearly neutral scene) the correlations do not separate the phases. There the pattern 00 01 01 02 gives a chromaticity error of 0.034 against the preview, the other phases give 0.30 or more, and the mean values of CFA positions 1 and 2 are equal (748.8 and 749.3). The *ist D crop rectangle starts at the odd position (19, 11). The pattern applied from stored pixel (0, 0) gives correlations 0.996, 0.998, 0.998 there.
- M9 (crop rectangle, S17 to S26): AOC-form files carry maker-note tags 0x0038 (2 SHORT) and 0x0039 (2 SHORT). PENTAX-form files carry neither and carry tag 0x023E (UNDEFINED, 8 bytes, four 16-bit values). For every model with a camera DNG in S26, the crop of this document equals the DNG crop: for AOC files 0x0038 equals DefaultCropOrigin and 0x0039 equals DefaultCropSize (no ActiveArea tag). For PENTAX files the first two values of 0x023E equal ActiveArea left plus DefaultCropOrigin x and ActiveArea top plus DefaultCropOrigin y, and the last two equal DefaultCropSize. The position of the IFD2 preview inside the raster, found by correlation of the image content, agrees within 2 pixels in all Huffman files. In the P1 and P2 files it differs from tag 0x0038 by up to 6 columns and 10 rows, so those previews are not exact crops of the raster. The crop size equals the IFD2 preview size in all files except the *ist DS, *ist DL, K100D, K100D Super and K110D, whose preview is 3008 x 2000 and whose crop is 3008 x 2008. The crop sizes 6192 x 4128 (K-3 Mark III) and 7360 x 4912 and 4800 x 3200 (K-1 Mark II) equal the RAW pixel counts that S8 and S9 state. Pixels outside the crop are not always image data: the K10D columns 3880 to 3895 continue the image, columns 3896 to 3899 hold values from 0 to 7, and columns 3900 to 3935 alternate between 0 and 4095 from row to row, and the K-3 Mark III and 645Z rasters have columns of 0 or of 16316.
- M10 (white level, S19, S24, S26): tag 0x007E equals the DNG WhiteLevel of the same model in every model that has both (for example 16316 K-5 and 645Z, 16317 K-1, 16254 KP, 3839 K-S2, 4095 K-7). Models without tag 0x007E (K10D, K20D, K200D, K2000, K-x and all P1 and P2 models) have BPS 12. Their DNG WhiteLevel is 4095 (K10D, K20D, K200D) or 4094 (K-x). The largest stored values form a plateau at or above tag 0x007E: K-5 16382 (tag 16316), KF 16378 (16319), KP 16383 (16254), 645D near 15860 (15767), K-3 II 16317 (16317), 645Z 16316 (16316).
- M11 (black level, S18 to S24, S26): tag 0x0200 holds four values that equal the DNG BlackLevel of the same model in most models (for example 65 65 66 66 K-5, 64 64 64 64 K-1, 128 128 128 128 KP, 16 16 16 16 K-S1, 256 256 256 256 K-S2, 0 0 0 0 K10D and 645D). The means of masked border pixels (outside the crop), taken per CFA position, lie within 3.5 of tag 0x0200 in 12 of 13 files measured: K-5 65.7 to 66.3 (tag 65 to 66), K-5 II 256.4 to 257.5 (256 to 258), K-5 II s 67.1 (67), K-1 64.3 to 65.1 (64), K-1 Mark II 65.0 (64), K-S1 16.1 (16), K-S2 255.8 to 256.3 (256), KF 66.9 to 67.2 (64), KP 127.1 to 128.3 (128), K-3 Mark III 254.2 to 254.8 (256), K-r 63.7 to 64.4 (63 to 64), K-1 Mark II APS-C 66.1 to 66.3 (64). The exception is the K-70 file: masked pixels 129.3 to 130.5, darkest image pixel 133, tag 0x0200 = 64 (Open questions OQ-3). The masked means do not show which of the four values belongs to which CFA position (OQ-2). In the K10D file the ratio of green to red in the six gray patches of a color chart stays between 1.96 and 2.01 from the white patch (raw red 1801) to the black patch (raw red 66), which agrees with a black level near 0 (tag 0).
- M12 (white balance, S17, S18, S24): tag 0x0201 holds four values. In the K10D file (RGGB) the six gray chart patches have raw ratios G/R 1.96 to 2.01 and G/B 1.06 to 1.13. Tag 0x0201 is 13760 8192 8192 9440, which gives mR = 1.680 and mB = 1.152. With these multipliers the gray patches have r/g near 0.84 and b/g near 1.05. The camera preview shows the same patches with linear r/g 0.82 to 0.86 and b/g 1.04 to 1.08 (the four middle patches). In 8 BGGR files the four values were assigned once in color order (red, green, green, blue) and once in CFA-position order. After the S11 matrix, the color order gives these chromaticity errors against the preview: 0.123 K-5, 0.128 K-5 II, 0.128 K-5 II s, 0.063 K-x, 0.092 K-7, 0.068 K-r, 0.074 K20D, 0.047 645D. The position order gives 0.22 to 0.64 in the same files.
- M13 (color matrix orientation, S17, S18, S24): white-balanced camera RGB was converted with four matrices and compared with the preview (mean chromaticity error over 32x32 blocks). Identity: 0.053 to 0.319. S11 matrix with rows as output channels: 0.024 to 0.130. S11 matrix transposed: 0.040 to 0.313. Constant matrix of maker-note tag 0x021D: 0.031 to 0.123. The S11 matrix with rows as output channels gives a smaller error than its transpose in 13 of 14 files (not in K-r: 0.068 and 0.063). Tags 0x021C and 0x021D hold the same 18 bytes each (byte order aside) in every color file from K10D to K-3 Mark III. The *ist DL, *ist DS, K100D, K100D Super and K110D hold one other 0x021D value. File by file (identity, S11, transposed): K10D 0.083, 0.073, 0.075. K-5 0.211, 0.124, 0.179. K-x 0.179, 0.067, 0.164. 645D 0.079, 0.047, 0.313. K-3 II 0.103, 0.024, 0.099. 645Z 0.053, 0.032, 0.118. K-S1 0.319, 0.130, 0.216.
- M14 (previews, S15 to S25): in all files IFD1 holds a 160 x 120 JPEG and IFD2 holds the largest JPEG, a baseline JPEG (SOF0, marker FF C0) with no APP segment. All streams start with FF D8 FF DB and end with FF D9. In the *ist D, *ist DS, *ist DL, K100D, K100D Super and K110D files, IFD1 and IFD2 have no ImageWidth and ImageLength tags. The maker note holds a third, smaller JPEG (640 x 480 or 720 x 480) at tags 0x0004 and 0x0003. It is not used.
- M15 (multi-frame files, S22, S25): in the K-3 Mark III Pixel Shift file IMGP9855, IFD0 is a complete raw image and tag 0x014A holds three offsets. Each of these SubIFDs is a complete raw IFD of the same structure (6304 x 4224, BPS 14, Compression 65535, its own strip of 29.2 MB and its own Exif IFD). IFD0 alone decodes with the rules of this document (M5) and shows the same scene as the single-frame file IMGP9854 of the same session. S6 lists maker-note tag 0x0008 value 7 as "RAW (pixel shift enabled)". The Pixel Shift files measured have tag 0x0008 = 7. The "HDR Advanced" file has tag 0x0008 = 6 and two SubIFD offsets.
- M16 (monochrome, S23): the K-3 Mark III Monochrome file has PhotometricInterpretation 34892, no CFAPattern tag and no tag 0x0201. It decodes with the same Huffman rules (M5). The mean absolute difference between horizontally adjacent pixels (68.1) is smaller than between pixels two columns apart (83.6), so no color filter pattern is present. The preview JPEG has three equal channels. Tag 0x0200 is 64 64 64 64 and tag 0x007E is 16378.
- M17 (complete procedure, S15 to S23): the ten kept sample files were developed with the whole procedure of this document (section Decoding procedure) and compared with their largest preview over 32x32 blocks. Results (correlation of log luminance, mean chromaticity error): *ist D 0.992, 0.035. *ist DS 0.969, 0.058. K10D 0.986, 0.070. K-5 0.993, 0.124. K-S2 0.986, 0.084. K-70 0.964, 0.033. K-1 Mark II APS-C 0.994, 0.063. K-3 Mark III 0.983, 0.184. K-3 Mark III Pixel Shift (IFD0) 0.983, 0.184. K-3 Mark III Monochrome 0.997, 0.000. The 21 further files of S24 give correlations from 0.970 to 0.999 and chromaticity errors from 0.027 to 0.135. The larger chromaticity errors come from the higher saturation of the camera JPEG rendering in green foliage and blue fabric. Visual inspection shows the same scene and plausible colors in every file.

### Sources consulted and not used

The page https://help.accusoft.com/ImageGear-Net/v25.3/Windows/HTML/PEF.html (a commercial imaging SDK reference) was read. It states that PEF stores the raw image in the root IFD, uncompressed or Huffman compressed, and it refers to "DCRAW Filter Control Parameters" of the product. Because the product embeds a raw decoder, nothing from the page is used. M1 and M5 establish the same facts. Web search results that pointed to decoder or metadata-library source code (a file pentaxmn_int.cpp of the exiv2 project on git.stg.centos.org, an issue of the metadata-extractor-dotnet project) were seen as titles and short summaries only. They were not opened and nothing from them is used. The ExifTool page https://exiftool.org/idiosyncracies.html was read for maker-note offset rules. Its Pentax remarks concern preview offsets of older Optio models, and nothing from it is used: M2 establishes the offset origins. See section Open questions for everything that no permitted source settles.

## Scope and variants

### Generations

The table lists the measured models (M1 to M3, M8, S15 to S25), their container form and the variant of their sensor data. S8 and S9 state 14-bit RAW for the K-3 Mark III and the K-1 Mark II.

| Models (measured) | Byte order | Maker note form | Variant | BPS | CFA |
| --- | --- | --- | --- | --- | --- |
| *ist D | MM | AOC | P1 | 12 | RGGB |
| *ist DS, *ist DL, K100D, K100D Super, K110D | MM | AOC | P2 | 12 | RGGB |
| K10D, K200D, K2000 | MM | AOC | P3 | 12 | RGGB |
| K20D, K-7, K-x, K-r | MM | AOC | P3 | 12 | BGGR |
| K-5, K-5 II, K-5 II s, 645D | MM | AOC | P3 | 14 | BGGR |
| K-S1, K-S2 | II | PENTAX | P3 | 12 | RGGB |
| K-3, K-3 II, K-70, KF, KP, K-1, K-1 Mark II, K-3 Mark III, 645Z | II | PENTAX | P3 | 14 | RGGB |
| K-3 Mark III Monochrome | II | PENTAX | P4 | 14 | none |

Models that write PEF but were not measured (for example K-m, K-01, K-30, K-50, K-500, *ist DL2) are classified by structure (R-PEF-3), not by model name.

### Storage variants

| Variant | IFD0 Compression (0x0103) | IFD0 PhotometricInterpretation (0x0106) | Sensor data |
| --- | --- | --- | --- |
| P1 | 1 | 32803 (CFA) | one strip, 16-bit samples in the byte order of the file, 12-bit values |
| P2 | 32773 | 32803 (CFA) | one strip, 12-bit values packed into 3 bytes per 2 pixels, StripByteCounts = 3 * W * H / 2 |
| P3 | 65535 | 32803 (CFA) | one strip, Pentax Huffman coding with the table of maker-note tag 0x0220, 12-bit or 14-bit values |
| P4 | 65535 | 34892 (LinearRaw, S3) | as P3, one monochrome channel |

S5 names Compression 32773 "PackBits" (S1 section 9) and 65535 "Pentax PEF Compressed". In PEF, Compression 32773 marks plain 12-bit packed data, not PackBits (M6).

### Out of scope

| Id | Case | Detection from file contents | Behavior |
| --- | --- | --- | --- |
| X1 | Pixel Shift composite and HDR composite: the combination of several frames into one image (S10) | IFD0 has tag 0x014A | the decoder decodes IFD0 alone as a normal frame (R-PEF-52). It does not combine the frames. |
| X2 | Compression 65535 with a missing or unusable table, BPS other than 12 or 14, or another PhotometricInterpretation | rules of R-PEF-3 fail | preview path |
| X3 | Any other storage: other Compression values, several strips, P1 with BPS other than 12, a packed strip of another length, a missing or invalid CFA pattern, an unknown maker-note form | no rule of R-PEF-3 matches | preview path |
| X4 | Pentax DNG files and the Optio .RAW format | not PEF: other specification | not handled by this document |

### Requirements

- R-PEF-1: The decoder must decode the sensor data of variants P1, P2, P3 and P4 as this document specifies.
- R-PEF-2: The decoder must use the preview path (section Embedded preview) for X2 and X3 files and must not interpret their sensor data.
- R-PEF-3: The decoder must classify the file by these rules, in this order, and must treat a file that matches no rule as X3:
  1. The maker note cannot be parsed by R-PEF-14 to R-PEF-17 and R-PEF-71: X3.
  2. Compression 65535, PhotometricInterpretation 32803, SamplesPerPixel 1, BPS 12 or 14, and a valid table in maker-note tag 0x0220 (R-PEF-27, R-PEF-28): P3.
  3. Compression 65535, PhotometricInterpretation 34892, SamplesPerPixel 1, BPS 12 or 14, and a valid table in tag 0x0220: P4.
  4. Compression 65535 in any other case: X2.
  5. Compression 1, PhotometricInterpretation 32803, SamplesPerPixel 1, BPS 12, StripByteCounts >= 2 * W * H: P1.
  6. Compression 32773, PhotometricInterpretation 32803, SamplesPerPixel 1, BPS 12, W even, StripByteCounts = 3 * W * H / 2: P2.

SamplesPerPixel is 1 when the tag 0x0115 is absent.

## Container layout

### Byte order and header

A PEF file is a TIFF file (S1 section 2, S7). Both byte orders occur (M1).

| Offset | Size | Field | Value in PEF |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 4D 4D ("MM", big-endian) or 49 49 ("II", little-endian) |
| 2 | 2 | TIFF magic number | 42 (bytes 00 2A or 2A 00) |
| 4 | 4 | offset of IFD0 | 8 in all measured files |

- R-PEF-4: The decoder must accept a file only if bytes 0 to 3 are 4D 4D 00 2A or 49 49 2A 00, and must report an error for any other start.
- R-PEF-5: The decoder must read every multi-byte TIFF field and every P1 sample in the byte order of bytes 0 and 1, and every maker-note field in the byte order of the maker note (R-PEF-15).

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

Field types and their sizes in bytes (S1 section 2): 1 BYTE (1), 2 ASCII (1), 3 SHORT (2), 4 LONG (4), 5 RATIONAL (8), 6 SBYTE (1), 7 UNDEFINED (1), 8 SSHORT (2), 9 SLONG (4), 10 SRATIONAL (8), 11 FLOAT (4), 12 DOUBLE (8). The value bytes of an entry occupy count * size bytes. If that total is 4 or less, the value is in the 4-byte value field, starting at its first byte. If the total is more than 4, the value field holds the offset of the value bytes. This is the same rule as arw.md R-ARW-6.

- R-PEF-6: The decoder must read a value from the value field when count * size <= 4 and from the offset in the value field when count * size > 4, with the offset origin of R-PEF-16 for maker-note entries and origin 0 for all other entries.
- R-PEF-7: The decoder must find tags by tag number in any entry order, and must use the first entry when a tag occurs more than once in one IFD.
- R-PEF-8: The decoder must accept both SHORT and LONG for every integer tag of this document (also where a table lists only one of them), and both BYTE and UNDEFINED for the byte-array tags 0x0220, 0x023E, 0x927C and 0xA302.
- R-PEF-9: The decoder must skip entries with a field type outside 1 to 12 and must not use their values.
- R-PEF-10: The decoder must compare ASCII values after it removes all trailing zero bytes and spaces.

### IFD chain

Measured layout (M1, values from S17, K10D):

| Structure | Where | Content in PEF |
| --- | --- | --- |
| IFD0 | offset 8 | the raw image: 0x0100 to 0x0117, Make, Model, Orientation, Exif IFD pointer 0x8769 |
| IFD1 | next-IFD offset of IFD0 | 160x120 thumbnail JPEG (0x0201, 0x0202) |
| IFD2 | next-IFD offset of IFD1 | full-size JPEG (0x0201, 0x0202) |
| Exif IFD | IFD0 tag 0x8769 | Exif data, MakerNote 0x927C, CFAPattern 0xA302 |
| Maker note | value offset of Exif tag 0x927C | Pentax IFD (AOC or PENTAX form) |
| SubIFDs | IFD0 tag 0x014A, Pixel Shift and HDR files only | further raw frames, not used (R-PEF-52) |

Example (S17, K10D): IFD0 at 8 with 19 entries. Exif IFD at 342. Maker note at 684 (77824 bytes). IFD1 at 78516, IFD2 at 78634. Sensor strip at 85472 (8126786 bytes). IFD2 JPEG at 8212260 (1348879 bytes).

- R-PEF-11: The decoder must read IFD0 at the offset in header bytes 4 to 7, must follow the next-IFD offsets to collect IFD1, IFD2 and later IFDs, and must stop at offset 0, at an invalid IFD, at a repeated offset or after 16 IFDs. When IFD0 itself is invalid, the decoder must report an error.
- R-PEF-12: The decoder must treat an IFD as invalid in each of these cases:
  1. The offset lies outside the file.
  2. N is 0 or greater than 1024.
  3. The 2 + 12 * N + 4 bytes of the IFD do not lie completely inside the file.
- R-PEF-13: The decoder must take the sensor data and the tags 0x0100 to 0x0117 from IFD0, and must not take sensor data from IFD1, IFD2 or the SubIFDs of tag 0x014A.

### Maker note

The Exif IFD (S4) is at the offset in IFD0 tag 0x8769. Its tag 0x927C (MakerNote, type UNDEFINED) has a count greater than 4, so its value field holds an offset. That offset is M, the start of the maker note. S7 states that PEF maker notes start with "AOC" and "MM". M2 shows a second form in newer files.

| Form | Bytes at M | Byte order bytes | IFD starts at | Origin of value offsets | Measured in |
| --- | --- | --- | --- | --- | --- |
| AOC | 41 4F 43 00 | M + 4 and M + 5 | M + 6 | 0 (file start) | all big-endian files |
| PENTAX | 50 45 4E 54 41 58 20 00 | M + 8 and M + 9 | M + 10 | M | all little-endian files |

The byte order bytes are 4D 4D (big-endian) or 49 49 (little-endian). The maker-note IFD has the structure of section IFD structure. Its next-IFD offset is not used.

- R-PEF-14: The decoder must find the Exif IFD at the offset in IFD0 tag 0x8769, must apply R-PEF-12 to it, and must take M as the offset stored in the value field of Exif tag 0x927C.
- R-PEF-71: The decoder must fail the maker note (R-PEF-3 rule 1) when the Exif IFD or tag 0x927C is absent, when the count of tag 0x927C is less than 14, or when M + 14 exceeds the file size, and must require the maker-note IFD and its values to lie inside the file (R-PEF-62) but not inside the 0x927C byte count.
- R-PEF-15: The decoder must identify the maker-note form by the bytes at M as in the table above, must read the byte order from the two byte-order bytes of that form, and must fail the maker note (R-PEF-3 rule 1) when the bytes match neither form or the byte-order bytes are neither 4D 4D nor 49 49.
- R-PEF-16: The decoder must add the origin of the table above (0 for the AOC form, M for the PENTAX form) to every value offset stored in a maker-note entry.
- R-PEF-17: The decoder must apply R-PEF-12 to the maker-note IFD, at offset M + 6 (AOC form) or M + 10 (PENTAX form).

## Metadata tags

"Required" means the variant cannot be decoded without the tag. The decoder then uses the preview path. "Default" is the value to use when an optional tag is absent or invalid.

### IFD0 and Exif IFD

| Tag | Name | Type | Count | IFD | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0x0100 | ImageWidth | SHORT or LONG | 1 | IFD0 | stored width W in pixels | required | none |
| 0x0101 | ImageLength | SHORT or LONG | 1 | IFD0 | stored height H in pixels | required | none |
| 0x0102 | BitsPerSample | SHORT | 1 | IFD0 | BPS: 12 or 14 (M1) | required | none |
| 0x0103 | Compression | SHORT | 1 | IFD0 | 1, 32773 or 65535 (section Scope and variants) | required | none |
| 0x0106 | PhotometricInterpretation | SHORT | 1 | IFD0 | 32803 = CFA, 34892 = LinearRaw (S3) | required | none |
| 0x010F | Make | ASCII | any | IFD0 | for example "PENTAX Corporation", "RICOH IMAGING COMPANY, LTD." | ignored | none |
| 0x0110 | Model | ASCII | any | IFD0 | for example "PENTAX K10D" (space padded) | optional | empty |
| 0x0111 | StripOffsets | SHORT or LONG | 1 | IFD0 | file offset of the single strip | required | none |
| 0x0112 | Orientation | SHORT | 1 | IFD0 | 1 to 8 (S1); absent in *ist D | optional | 1 |
| 0x0115 | SamplesPerPixel | SHORT | 1 | IFD0 | 1 | optional | 1 |
| 0x0116 | RowsPerStrip | SHORT or LONG | 1 | IFD0 | H in all measured files | ignored | none |
| 0x0117 | StripByteCounts | SHORT or LONG | 1 | IFD0 | byte count of the single strip | required | none |
| 0x014A | SubIFDs | LONG | 2 or 3 | IFD0 | further frames (M15) | ignored | none |
| 0x8769 | ExifIFD | LONG | 1 | IFD0 | offset of the Exif IFD | required | none |
| 0x0201 | JPEGInterchangeFormat | LONG | 1 | IFD1, IFD2 | offset of a JPEG stream (S5 calls the IFD2 one JpgFromRawStart) | optional | none |
| 0x0202 | JPEGInterchangeFormatLength | LONG | 1 | IFD1, IFD2 | length of that JPEG stream | optional | none |
| 0x927C | MakerNote | UNDEFINED | any | Exif | the maker note (R-PEF-14) | required | none |
| 0xA302 | CFAPattern | UNDEFINED | 8 | Exif | CFA pattern (S4, R-PEF-19) | P1, P2, P3 | none |

### Maker note

Tag names in the column Name are the names of S6. A question mark is part of the S6 name. Tags 0x023D and 0x023E have no name in S6. Their meaning comes from M9.

| Tag | Name | Type | Count | Forms | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0x0008 | Quality | SHORT | 1 | both | 4 = RAW, 7 = RAW with Pixel Shift, 9 = monochrome (S6, M15, M16); diagnostic only | ignored | none |
| 0x0038 | ImageAreaOffset | SHORT | 2 | AOC | x, y of the crop rectangle (M9) | optional | R-PEF-22 |
| 0x0039 | RawImageSize | SHORT | 2 | AOC | width, height of the crop rectangle (M9) | optional | R-PEF-22 |
| 0x007E | WhiteLevel | LONG | 1 | both, not in K10D, K20D, K200D, K2000, K-x, P1, P2 | white level (M10) | optional | 2^BPS - 1 |
| 0x0200 | BlackPoint | SHORT | 4 | both | black levels (M11, R-PEF-38) | optional | 0 0 0 0 |
| 0x0201 | WhitePoint | SHORT | 4 | both, not in P4 | as-shot WB levels in the order red, green, green, blue (M12) | optional | R-PEF-40 |
| 0x0220 | HuffmanTable? | UNDEFINED | 14 + 3 * N | both, P3 and P4 only | Huffman table (M3, R-PEF-27) | P3, P4 | none |
| 0x023D | (no S6 name) | UNDEFINED | 20 | PENTAX | first four 16-bit values: x, y, width, height of the active area (M9); not used | ignored | none |
| 0x023E | (no S6 name) | UNDEFINED | 8 | PENTAX | four 16-bit values: x, y, width, height of the crop rectangle (M9) | optional | R-PEF-22 |

All 16-bit values inside UNDEFINED maker-note tags are in the byte order of the maker note.

- R-PEF-18: The decoder must use the preview path when a tag marked "required" for the classified variant is absent, has a count of 0, or has a value range outside the file.
- R-PEF-19: The decoder must accept CFAPattern only if it has 8 bytes, its first four bytes are 00 02 00 02 or 02 00 02 00, and its last four bytes contain exactly one 0, two 1 and one 2 with the two 1 values at positions 1 and 2 or at positions 0 and 3 (a Bayer arrangement), and must otherwise use the preview path for P1, P2 and P3. The last four bytes are the colors of CFA positions 0, 1, 2, 3, with 0 = red, 1 = green, 2 = blue (S4).
- R-PEF-20: The decoder must use the preview path when StripOffsets or StripByteCounts has a count other than 1, or when the strip does not lie completely inside the file.
- R-PEF-70: The decoder must use the first value of every other tag that this document lists with count 1 when the tag has more values.

## Sensor data layout

### Raster

The raw raster has W columns and H rows, stored in one strip at StripOffsets. Rows follow each other without padding. The CFA pattern repeats every 2 rows and 2 columns, with its origin at stored pixel (0, 0) (M8).

- R-PEF-21: The decoder must give pixel (x, y) of a P1, P2 or P3 raster the color CFA[2 * (y mod 2) + (x mod 2)], where CFA[0] to CFA[3] are the last four bytes of CFAPattern, with the origin of the pattern at stored pixel (0, 0) and independent of the crop rectangle.

### Crop rectangle

The final image is the crop rectangle (x0, y0, CW, CH): CW columns and CH rows starting at stored pixel (x0, y0). Pixels outside it can be masked pixels or invalid data (M9).

Measured crop rectangles (M9):

| Models | W x H | Source tag | x0, y0 | CW x CH | IFD2 preview |
| --- | --- | --- | --- | --- | --- |
| *ist D | 3040 x 2024 | 0x0038, 0x0039 | 19, 11 | 3008 x 2008 | 3008 x 2008 |
| *ist DS, *ist DL, K100D, K100D Super, K110D | 3040 x 2024 | 0x0038, 0x0039 | 16, 8 | 3008 x 2008 | 3008 x 2000 |
| K10D, K200D, K2000 | 3936 x 2624 | 0x0038, 0x0039 | 8, 8 | 3872 x 2592 | same as crop |
| K20D | 4864 x 3136 | 0x0038, 0x0039 | 8, 8 | 4672 x 3104 | same as crop |
| K-7 | 4736 x 3136 | 0x0038, 0x0039 | 10, 10 | 4672 x 3104 | same as crop |
| K-x, K-r | 4352 x 2868 | 0x0038, 0x0039 | 10, 10 | 4288 x 2848 | same as crop |
| K-5, K-5 II, K-5 II s | 4992 x 3284 | 0x0038, 0x0039 | 22, 10 | 4928 x 3264 | same as crop |
| 645D | 7424 x 5552 | 0x0038, 0x0039 | 80, 59 | 7264 x 5440 | same as crop |
| K-S1, K-S2 | 5504 x 3664 | 0x023E | 10, 4 | 5472 x 3648 | same as crop |
| K-3, K-3 II | 6080 x 4032 | 0x023E | 16, 16 | 6016 x 4000 | same as crop |
| K-70, KF | 6080 x 4064 | 0x023E | 66, 32 | 6000 x 4000 | same as crop |
| KP | 6112 x 4060 | 0x023E | 60, 44 | 6016 x 4000 | same as crop |
| K-1, K-1 Mark II | 7392 x 4950 | 0x023E | 14, 28 | 7360 x 4912 | same as crop |
| K-1 Mark II (APS-C mode) | 4832 x 3238 | 0x023E | 14, 28 | 4800 x 3200 | same as crop |
| K-3 Mark III, K-3 Mark III Monochrome | 6304 x 4224 | 0x023E | 54, 58 | 6192 x 4128 | same as crop |
| 645Z | 8384 x 6208 | 0x023E | 58, 8 | 8256 x 6192 | same as crop |

- R-PEF-22: The decoder must take the crop rectangle from maker-note tag 0x023E when it is present with type BYTE or UNDEFINED and count 8 (x0, y0, CW, CH as four 16-bit values), otherwise from tags 0x0038 (x0, y0) and 0x0039 (CW, CH) when both are present with 2 values each, and otherwise must use (0, 0, W, H).
- R-PEF-23: The decoder must use (0, 0, W, H) when the selected rectangle has CW < 1, CH < 1, x0 + CW > W or y0 + CH > H.

### Strip layout of P1 and P2

Row y of P1 starts at byte StripOffsets + 2 * W * y. Row y of P2 starts at byte StripOffsets + (3 * W / 2) * y. P3 and P4 have no row positions: the whole raster is one bit stream (section Compression).

- R-PEF-24: The decoder must read row y of P1 at StripOffsets + 2 * W * y and row y of P2 at StripOffsets + 3 * W * y / 2, with no padding between rows.

## Compression

This section specifies, for each variant, how the stored bytes become sensor values. All variants produce one value per stored pixel, in the range 0 to 2^BPS - 1.

### P1: 16-bit samples

Each pixel is a 16-bit unsigned integer in the byte order of the file (M7). The *ist D file is big-endian.

- R-PEF-25: The decoder must read P1 pixel (x, y) as 256 * b0 + b1 for a big-endian file and b0 + 256 * b1 for a little-endian file, where b0 and b1 are the bytes at StripOffsets + 2 * (W * y + x) and the next byte.

Worked example (first bytes of the S15 strip at offset 65536): 00 E6 01 1C 00 DD 01 13 give 230, 284, 221, 275.

### P2: 12-bit packed

Each row holds 3 * W / 2 bytes. Pixels 2k and 2k + 1 of a row come from the three bytes b0, b1, b2 at row offset 3k. The first pixel is in the high 12 bits of the 24-bit big-endian group (M6). This is the opposite order of the Sony V2 packing in arw.md.

- R-PEF-26: The decoder must compute pixel 2k of a P2 row as 16 * b0 + floor(b1 / 16) and pixel 2k + 1 as 256 * (b1 mod 16) + b2.

Worked example (first bytes of the S16 strip at offset 73728): 89 D0 92 09 40 97.
- Group 1: b0 = 0x89, b1 = 0xD0, b2 = 0x92. Pixel 0 = 16 * 137 + 13 = 2205. Pixel 1 = 256 * 0 + 146 = 146.
- Group 2: b0 = 0x09, b1 = 0x40, b2 = 0x97. Pixel 2 = 16 * 9 + 4 = 148. Pixel 3 = 256 * 0 + 151 = 151.

### P3: Pentax Huffman coding

Sources: S6 names maker-note tag 0x0220 "HuffmanTable?" with a question mark. S5 names Compression 65535 "Pentax PEF Compressed". S2 Annex F defines difference categories (Table F.1) and the mapping of extra bits to a signed difference (procedure EXTEND, F.2.2.1). The table layout, the bit order, the predictor and the stream boundaries come from measurements M3 to M5.

#### The table (maker-note tag 0x0220)

Let N = BPS + 1 (13 for 12-bit data, 15 for 14-bit data). All 16-bit values are in the byte order of the maker note.

| Byte offset in the tag value | Size | Content |
| --- | --- | --- |
| 0 | 2 | V: N - 12 (1 or 3 in all measured files) |
| 2 | 12 | not used (Open questions OQ-1) |
| 14 + 2 * s, s = 0 to N - 1 | 2 | C(s): the code of SSSS value s, left-aligned in 12 bits |
| 14 + 2 * N + s, s = 0 to N - 1 | 1 | L(s): the length of that code in bits |

The code of SSSS value s is the L(s)-bit number floor(C(s) / 2^(12 - L(s))). The low 12 - L(s) bits of C(s) are zero. The codes form a prefix code: no code is the beginning of another code (M3).

- R-PEF-27: The decoder must accept tag 0x0220 only if its count is 14 + 3 * N and its first 16-bit value V equals N - 12, with N = BPS + 1.
- R-PEF-28: The decoder must accept the table only if, for every s from 0 to N - 1, 1 <= L(s) <= 12, C(s) <= 4095, C(s) mod 2^(12 - L(s)) = 0, and no two codes are equal or one the beginning of the other.

Worked example (vector TV-5, K10D table, 53 bytes, big-endian): V = 1, N = 13. C(0) = 0x0F00 and L(0) = 5, so the code of SSSS 0 is floor(3840 / 128) = 30 = 11110 in 5 bits. The 13 codes are, for SSSS 0 to 12: 11110, 110, 100, 00, 01, 101, 1110, 111110, 1111110, 11111110, 111111110, 1111111110, 1111111111.

#### The bit stream

The sensor data are one bit stream that starts at byte StripOffsets. Bits are read from the most significant bit (bit 7) to the least significant bit (bit 0) of each byte, and bytes in increasing file order. The stream does not use byte stuffing: a 0xFF byte is followed by any byte, and all its bits are data. Rows do not start at byte boundaries: the first code of a row follows the last bit of the previous row (M4).

The pixels are coded in raster order: row 0 from x = 0 to W - 1, then row 1, and so on. Each pixel is coded as one code word of the table, followed by s extra bits, where s is the SSSS value of that code word.

- R-PEF-29: The decoder must read the stream from the first bit of byte StripOffsets, most significant bit first, bytes in increasing order, with no byte stuffing and no alignment at row starts.
- R-PEF-30: The decoder must decode the pixels in raster order, and for each pixel must read one code word and then s extra bits.
- R-PEF-31: The decoder must select the code word as the table entry s whose L(s)-bit code equals the next L(s) bits of the stream, and must fail when no entry matches.

#### Difference and prediction

The s extra bits form an unsigned number E (first bit most significant). The difference D follows S2 F.2.2.1:
- s = 0: D = 0 (no extra bits).
- s > 0 and E >= 2^(s - 1): D = E.
- s > 0 and E < 2^(s - 1): D = E - 2^s + 1.

The prediction P of pixel (x, y) uses the pixel two columns to the left, which has the same CFA color (M4):
- x >= 2: P = value(x - 2, y).
- x < 2 and y >= 2: P = value(x, y - 2).
- x < 2 and y < 2: P = 0.

The pixel value is P + D. In all measured files P + D lies in 0 to 2^BPS - 1 (M5).

- R-PEF-32: The decoder must compute D from s and E by the three cases above.
- R-PEF-33: The decoder must compute P by the three cases above, from the stored values of earlier pixels of the same raster.
- R-PEF-34: The decoder must store clip(P + D, 0, 2^BPS - 1) as the value of the pixel, and must use the stored value in later predictions.

Worked example (vector TV-7). Table of TV-5. Stream bytes FB 27 EC 89 AB FB 4B B2 14 A2 59 87, raster 4 x 3:
- (0, 0): bits 111110 are the code of SSSS 7. The next 7 bits 1100100 give E = 100. 100 >= 64, so D = 100. P = 0 (x < 2, y < 2). Value 100.
- (3, 0): bits 01 are the code of SSSS 4. Extra bits 0101 give E = 5. 5 < 8, so D = 5 - 16 + 1 = -10. P = value(1, 0) = 200. Value 190.
- (0, 2): bits 100 are the code of SSSS 2. Extra bits 01 give E = 1. 1 < 2, so D = 1 - 4 + 1 = -2. P = value(0, 0) = 100 (x < 2, y >= 2). Value 98.
- The complete trace is in section Test vectors.

#### End of the stream

The coded data end in the last byte of the strip. In the measured files 0 to 7 bits of that byte remain unused after the last coded pixel (M5). The decoder may read ahead of the current position, for example 12 bits at a time to find a code word. Near the end of the strip such a look-ahead extends beyond the last byte although the code word itself does not.

- R-PEF-35: The decoder must fail when a code word or its extra bits extend beyond the last byte of the strip, must not fail when only bits that it looks ahead extend beyond it (it treats those bits as 0), and must ignore the bits after the last coded pixel.

### P4: monochrome Huffman coding

P4 uses the table, the bit stream, the difference and the prediction of P3 without change (M16). The prediction from x - 2 also applies, although the raster has no CFA.

- R-PEF-36: The decoder must decode P4 by R-PEF-27 to R-PEF-35 and must treat the result as one gray channel without a CFA.

## Levels and white balance

The decoder determines the white level first and the black levels second. arw.md applies the same normalization formula (R-ARW-47). The sources of the values differ.

### White level

- R-PEF-37: The decoder must use the value of maker-note tag 0x007E as the white level w when the tag is present and the value is from 256 to 2^BPS - 1, and 2^BPS - 1 otherwise.

Values above w occur (M10). R-PEF-41 clips them.

### Black level

Tag 0x0200 holds four values B0, B1, B2, B3. M11 shows that they are the black levels. M11 does not show whether the order is the color order of tag 0x0201 or the CFA-position order (OQ-2). In all measured files B0 and B3 differ by at most 2 and B1 and B2 by at most 1. This document uses the color order of tag 0x0201 (M12) with the mean of the two green values.

- R-PEF-38: The decoder must use the black level bR = B0 for red pixels, bG = (B1 + B2) / 2 for green pixels and bB = B3 for blue pixels when tag 0x0200 has 4 values and each value is less than w - 255.
- R-PEF-39: The decoder must use the black level 0 for every color when tag 0x0200 is absent or invalid by R-PEF-38.

### White balance multipliers

The decoder computes three multipliers mR, mG, mB. Sources in priority order:
1. Tag 0x0201 (W0, W1, W2, W3 in the order red, green, green, blue, M12) with all four values greater than 0: G = (W1 + W2) / 2, mR = W0 / G, mG = 1, mB = W3 / G.
2. The D50 white balance scales of the Model in the table of section Color conversion (S11): mR and mB from the table, mG = 1.
3. The generic scales mR = 2.162, mG = 1, mB = 1.483 (mean of the table).

- R-PEF-40: The decoder must take the WB multipliers of P1, P2 and P3 from the first available source of the list above.

Worked example (S17, K10D): tag 0x0201 = 13760 8192 8192 9440. G = 8192, mR = 13760 / 8192 = 1.6796875, mB = 9440 / 8192 = 1.15234375.

### Normalization

For a P1, P2 or P3 pixel with value s, black level b of its color, white level w and multiplier m of its color:
- n = (s - b) / (w - b)
- v = n * m, clipped to the range 0 to 1.

- R-PEF-41: The decoder must compute v = clip((s - b) / (w - b) * m, 0, 1) for every P1, P2 and P3 pixel inside the crop rectangle before demosaicing.
- R-PEF-42: The decoder must compute for every P4 pixel v = clip((s - b) / (w - b), 0, 1), with b the mean of the four values of tag 0x0200 (0 when the tag is absent or invalid by R-PEF-38), and must not apply WB multipliers to P4.

Worked example (fixture F3): a CFA cell with R = 1128, G = 2128, B = 1028, black 128, white 4095, multipliers of the K10D example:
- R: (1128 - 128) / 3967 * 1.6796875 = 0.2520797 * 1.6796875 = 0.4234150.
- G: (2128 - 128) / 3967 = 0.5041593.
- B: (1028 - 128) / 3967 * 1.15234375 = 0.2268717 * 1.15234375 = 0.2614342.

## Color conversion

### Matrix sources

PEF files carry no camera-specific color matrix that a permitted source describes. The maker-note tags 0x021C and 0x021D (S6 names ColorMatrixA2 and ColorMatrixB2) hold the same values in every color model from K10D to K-3 Mark III (M13), so they are not a per-model calibration. The *ist D carries tags 0x0203 and 0x0204 (ColorMatrixA and ColorMatrixB in S6). Their meaning is not documented. The decoder therefore uses a per-model table, as arw.md does for Sony. S11 (DXOMARK) publishes for each tested camera a "color matrix as defined in ISO standard 17321" for the CIE D50 and CIE A illuminants, with white balance scales. The matrix converts white-balanced camera RGB to linear sRGB. Each row is one output channel (R, G, B of sRGB) and each column is one camera channel. The rows sum to 1, so a white-balanced neutral stays neutral, and M13 confirms the orientation against the camera JPEG.

How to read S11: open the page https://www.dxomark.com/Cameras/Pentax/<name>---Measurements, tab "Color response". The page shows a CIE D50 block and a CIE A block. The D50 block is the block with the larger ratio of the R scale to the B scale. The table below gives the D50 block of each listed model, read on 2026-11-14. S11 rounds the values to 2 decimals. The Model column is the IFD0 Model string, measured in S15 to S24 or, for K-01, K-30, K-50 and K-500, in the camera DNG files of S26.

| Model (IFD0 0x0110) | S11 name | WB R | WB B | m11 | m12 | m13 | m21 | m22 | m23 | m31 | m32 | m33 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| PENTAX K10D | K10D | 2.08 | 1.03 | 1.70 | -0.59 | -0.10 | -0.08 | 1.33 | -0.25 | 0.01 | -0.23 | 1.23 |
| PENTAX K20D | K20D | 2.12 | 1.17 | 1.82 | -0.66 | -0.15 | -0.14 | 1.34 | -0.20 | 0.02 | -0.45 | 1.43 |
| PENTAX K200D | K200D | 2.15 | 1.20 | 1.70 | -0.62 | -0.08 | -0.08 | 1.35 | -0.26 | 0.02 | -0.26 | 1.24 |
| PENTAX K-7 | K-7 | 2.18 | 1.07 | 1.75 | -0.55 | -0.20 | -0.10 | 1.30 | -0.20 | 0.03 | -0.38 | 1.35 |
| PENTAX K-x | K-x | 2.22 | 1.20 | 1.91 | -0.75 | -0.15 | -0.13 | 1.48 | -0.36 | 0.01 | -0.34 | 1.33 |
| PENTAX K-r | K-r | 1.87 | 1.47 | 1.76 | -0.62 | -0.14 | -0.11 | 1.42 | -0.32 | 0.05 | -0.42 | 1.37 |
| PENTAX K-5 | K-5 | 2.24 | 1.64 | 1.82 | -0.73 | -0.09 | -0.15 | 1.54 | -0.39 | 0.06 | -0.46 | 1.40 |
| PENTAX K-5 II | K-5-II | 2.16 | 1.61 | 1.68 | -0.57 | -0.11 | -0.14 | 1.52 | -0.38 | 0.06 | -0.47 | 1.41 |
| PENTAX K-5 II s | K-5-IIs | 2.10 | 1.72 | 1.69 | -0.57 | -0.12 | -0.14 | 1.49 | -0.35 | 0.07 | -0.49 | 1.43 |
| PENTAX K-01 | K-01 | 2.22 | 1.68 | 1.77 | -0.69 | -0.08 | -0.14 | 1.51 | -0.37 | 0.08 | -0.50 | 1.42 |
| PENTAX K-30 | K-30 | 2.19 | 1.71 | 1.67 | -0.55 | -0.13 | -0.18 | 1.59 | -0.41 | 0.05 | -0.46 | 1.40 |
| PENTAX K-50 | K-50 | 2.19 | 1.65 | 1.78 | -0.70 | -0.08 | -0.14 | 1.52 | -0.38 | 0.08 | -0.49 | 1.41 |
| PENTAX K-500 | K-500 | 2.26 | 1.66 | 1.77 | -0.70 | -0.07 | -0.14 | 1.52 | -0.38 | 0.07 | -0.48 | 1.41 |
| PENTAX K-3 | K-3 | 2.32 | 1.58 | 1.82 | -0.73 | -0.09 | -0.15 | 1.55 | -0.40 | 0.05 | -0.47 | 1.42 |
| PENTAX K-3 II | K-3-II | 2.22 | 1.58 | 1.82 | -0.74 | -0.08 | -0.16 | 1.58 | -0.42 | 0.06 | -0.51 | 1.45 |
| PENTAX K-S1 | K-S1 | 2.24 | 1.72 | 1.70 | -0.61 | -0.09 | -0.16 | 1.53 | -0.37 | 0.07 | -0.46 | 1.39 |
| PENTAX K-1 | K-1 | 2.07 | 1.67 | 1.80 | -0.67 | -0.13 | -0.15 | 1.54 | -0.39 | 0.07 | -0.47 | 1.40 |
| PENTAX 645D | 645D | 2.18 | 1.20 | 1.99 | -0.88 | -0.11 | -0.19 | 1.57 | -0.38 | 0.11 | -0.75 | 1.64 |
| PENTAX 645Z | 645Z | 2.06 | 1.61 | 1.87 | -0.81 | -0.06 | -0.16 | 1.56 | -0.40 | 0.09 | -0.59 | 1.50 |

Generic matrix for models that are not in the table (for example *ist D, *ist DS, K100D, K2000, K-S2, K-70, KF, KP, K-1 Mark II, K-3 Mark III): the element-wise mean of the 19 matrices above, rounded to 3 decimals:

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 1.780 | -0.671 | -0.108 |
| sRGB G | -0.139 | 1.486 | -0.348 |
| sRGB B | 0.056 | -0.457 | 1.402 |

The generic WB scales are the means of the table: R 2.162, B 1.483.

### Procedure

- R-PEF-43: The decoder must select the table row whose Model column equals the IFD0 Model string (compared by R-PEF-10), and must use the generic matrix when no row matches.
- R-PEF-44: The decoder must divide each row of the selected matrix by the sum of that row before use, so that every row sums to 1.
- R-PEF-45: The decoder must compute the output (r, g, b) of a P1, P2 or P3 pixel from its demosaiced values (vR, vG, vB) with the normalized matrix as follows, and must clip r, g and b to the range 0 to 1:
  - r = m11 * vR + m12 * vG + m13 * vB
  - g = m21 * vR + m22 * vG + m23 * vB
  - b = m31 * vR + m32 * vG + m33 * vB
- R-PEF-46: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S14), without the sRGB transfer function.
- R-PEF-47: The decoder must deliver a P4 pixel as r = g = b = v (R-PEF-42) and must not apply a matrix to it.

Worked example (fixture F3, Model "PENTAX K10D"): the row sums are 1.01, 1.00 and 1.01. The normalized rows are (1.6831683, -0.5841584, -0.0990099), (-0.08, 1.33, -0.25) and (0.0099010, -0.2277228, 1.2178218). With (vR, vG, vB) = (0.4234150, 0.5041593, 0.2614342):
- r = 0.7126788 - 0.2945089 - 0.0258846 = 0.3922853.
- g = -0.0338732 + 0.6705319 - 0.0653585 = 0.5713001.
- b = 0.0041922 - 0.1148086 + 0.3183802 = 0.2077639.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and read the header (R-PEF-59, R-PEF-4, R-PEF-5).
2. Read IFD0 and the IFD chain. Validate every IFD (R-PEF-11, R-PEF-12, R-PEF-6 to R-PEF-9).
3. Collect the preview candidates now, so that every later failure can fall back (R-PEF-53, R-PEF-54).
4. Read the Exif IFD and the maker note (R-PEF-14 to R-PEF-17, R-PEF-71).
5. Classify the file (R-PEF-3, R-PEF-13). For X2 and X3 use the preview path. Check the required tags, the CFA pattern and the strip (R-PEF-18 to R-PEF-20, R-PEF-70).
6. Determine the crop rectangle (R-PEF-22, R-PEF-23), the CFA colors (R-PEF-21), the white and black levels (R-PEF-37 to R-PEF-39) and the WB multipliers (R-PEF-40).
7. Check all sizes against the file and the limits before allocating memory (section Error handling).
8. Decode the sensor data of the whole raster:
   - P1: R-PEF-24, R-PEF-25.
   - P2: R-PEF-24, R-PEF-26.
   - P3: R-PEF-27 to R-PEF-35.
   - P4: R-PEF-36.
9. Normalize the pixels of the crop rectangle (R-PEF-41 for P1 to P3, R-PEF-42 for P4).
10. Demosaic P1 to P3 inside the crop rectangle (R-PEF-48, R-PEF-49). P4 needs no demosaicing.
11. Convert to linear sRGB (R-PEF-43 to R-PEF-45 for P1 to P3, R-PEF-47 for P4).
12. Apply the orientation (R-PEF-50, R-PEF-51).
13. Deliver the image (R-PEF-46, R-PEF-58). If any step from 4 to 12 fails, deliver the preview instead (R-PEF-57).

### Demosaicing

The decoder uses bilinear interpolation, the basic non-adaptive method described in S12, the same method as arw.md R-ARW-54. The crop rectangle takes the place of the valid area of arw.md: pixels outside it can hold invalid data (M9), so they never enter the interpolation. For each pixel and each color that the pixel lacks, the missing value is the mean of the pixels of that color among its 8 neighbors inside the crop rectangle. With a 2x2 Bayer pattern:
- Green at a red or blue pixel: the mean of the 4 orthogonal neighbors.
- Red or blue at a green pixel: the mean of the 2 row neighbors or of the 2 column neighbors, whichever pair has that color.
- Red at a blue pixel, and blue at a red pixel: the mean of the 4 diagonal neighbors.

- R-PEF-48: For each pixel (x, y) of the crop rectangle, the decoder must set its own color to its normalized value v, and each missing color to the arithmetic mean of the normalized values of the pixels of that color among the 8 neighbors (x + dx, y + dy), with dx and dy from -1 to 1 and not both 0.
- R-PEF-49: The decoder must ignore neighbors outside the crop rectangle in that mean, and must use 0 when no neighbor of the missing color lies inside the crop rectangle.

Worked example: a crop rectangle that starts at the odd column x0 = 1 of an RGGB raster (fixture F3). Its first pixel (1, 0) is green. Its neighbors inside the rectangle are (2, 0) red, (1, 1) blue and (2, 1) green. Red = value of (2, 0). Blue = value of (1, 1). The stored pixel (0, 0), which is red, lies outside the rectangle and is ignored.

### Orientation

- R-PEF-50: The decoder must read Orientation from IFD0 tag 0x0112, use 1 when the tag is absent or outside 1 to 8, and apply it after the crop.
- R-PEF-51: The decoder must map pixel (x, y) of the cropped W' x H' image to the output pixel of the table below, with an output of W' x H' pixels for values 1 to 4 and H' x W' pixels for values 5 to 8.

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

These mappings follow the definitions of S1 and equal the table of arw.md. For example, S1 defines value 6 as "the 0th row represents the visual right-hand side of the image, and the 0th column represents the visual top".

### Multi-frame files

- R-PEF-52: The decoder must decode only the IFD0 frame of a file whose IFD0 has tag 0x014A, must deliver it as a single image by this procedure, and must not read the frames of the SubIFDs.

## Embedded preview

### Locations

| Location | Tags | Content (M14) |
| --- | --- | --- |
| IFD1 | 0x0201 offset, 0x0202 length | 160 x 120 JPEG thumbnail |
| IFD2 | 0x0201, 0x0202 | full-size JPEG: the crop rectangle size, or 3008 x 2000 in the *ist DS family |
| maker note | 0x0004 offset, 0x0003 length | 640 x 480 or 720 x 480 JPEG, not used |

All preview streams are baseline JPEG (SOF0, 8-bit, 3 components) and are decoded with S2 (baseline sequential DCT process). They contain no APP1 (Exif) segment, so they carry no orientation of their own (M14). The monochrome preview has three equal channels.

### Requirements

- R-PEF-53: The decoder must collect as preview candidates the pairs (0x0201, 0x0202) of every IFD of the chain, and must keep a candidate only if offset + length lies inside the file, length >= 4, and the first two bytes are FF D8.
- R-PEF-54: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment, not from ImageWidth and ImageLength, and must choose the candidate with the largest width * height.
- R-PEF-55: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan.
- R-PEF-56: The decoder must apply the IFD0 Orientation (R-PEF-51) to the preview, except that it must leave the preview unchanged (no rotation and no mirroring) for values 5 to 8 when the preview is taller than wide.
- R-PEF-57: The decoder must deliver the chosen preview when raw decoding fails or the file is X2 or X3, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-PEF-58: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-PEF-46) or a preview (sRGB-encoded JPEG values, not linear).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-PEF-57). The limits are those of arw.md, with the changes for the Huffman stream.

### Limits

- R-PEF-59: The decoder must fail for a file shorter than 8 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-PEF-60: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728). The largest measured raster is 8384 x 6208 = 52047872 pixels (645Z).
- R-PEF-61: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values.
- R-PEF-62: The decoder must check, before reading, that every value, the maker note IFD, the table and the strip lie completely inside the file.

### Structure checks

- R-PEF-63: The decoder must fail for P1 when StripByteCounts is less than 2 * W * H, and for P2 when StripByteCounts differs from 3 * W * H / 2, and must never read more than these byte counts.
- R-PEF-64: The decoder must fail for P3 and P4 when the strip has fewer than ceil(W * H / 8) bytes, because every pixel needs at least one bit (the shortest code has at least one bit).
- R-PEF-65: The decoder must allocate for P3 and P4 at most one value per stored pixel, must keep the table in a structure of fixed size (at most 4096 entries), and must not allocate memory in proportion to the number of decoded bits.
- R-PEF-66: The decoder must clip every decoded sensor value to 0 to 2^BPS - 1 (R-PEF-34) and every normalized value to 0 to 1 (R-PEF-41, R-PEF-42).
- R-PEF-67: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check selects the default of the quantity (w - b > 0 holds whenever R-PEF-37 to R-PEF-39 are followed).

### Behaviour on error

- R-PEF-68: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-PEF-69: The decoder must discard a partly decoded raw image on failure and must not deliver it.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order.

### TV-1: TIFF headers

Bytes 0x4D 0x4D 0x00 0x2A 0x00 0x00 0x00 0x08: big-endian TIFF, IFD0 at offset 8 (all big-endian PEF files). Bytes 0x49 0x49 0x2A 0x00 0x08 0x00 0x00 0x00: little-endian TIFF, IFD0 at offset 8 (R-PEF-4).

### TV-2: maker-note forms and offset origins

- AOC form (S17, K10D): at M = 684 the bytes 41 4F 43 00 4D 4D 00 5B. Form AOC, big-endian, N = 0x005B = 91 entries, IFD at 690. The entry 02 00 00 03 00 00 00 04 00 00 06 FC is tag 0x0200, SHORT, count 4 (8 bytes), stored offset 0x06FC = 1788, origin 0, so the values are at file offset 1788: bytes 00 00 00 00 00 00 00 00, values 0 0 0 0. 1788 = 690 + 2 + 12 * 91 + 4, the end of the IFD (M2).
- PENTAX form (S22, K-3 Mark III IMGP9854): at M = 902 the bytes 50 45 4E 54 41 58 20 00 49 49 63 00. Form PENTAX, little-endian, N = 0x0063 = 99 entries, IFD at 912. The entry 00 02 03 00 04 00 00 00 B4 04 00 00 is tag 0x0200, SHORT, count 4, stored offset 0x04B4 = 1204, origin M, so the values are at file offset 902 + 1204 = 2106: bytes 00 01 00 01 00 01 00 01, values 256 256 256 256. 2106 = 912 + 2 + 12 * 99 + 4.

### TV-3: CFAPattern

- 0x00 0x02 0x00 0x02 0x00 0x01 0x01 0x02: CFA = 0 1 1 2 (RGGB). Pixel (0, 0) red, (1, 0) green, (0, 1) green, (1, 1) blue.
- 0x00 0x02 0x00 0x02 0x02 0x01 0x01 0x00: CFA = 2 1 1 0 (BGGR, K-5). Pixel (0, 0) blue, (1, 1) red.
- 0x00 0x02 0x00 0x02 0x00 0x01 0x01 0x01: invalid (three greens), preview path (R-PEF-19).

### TV-4: crop tags

- Tag 0x023E, little-endian maker note (S22): 0x36 0x00 0x3A 0x00 0x30 0x18 0x20 0x10 give x0 = 54, y0 = 58, CW = 6192, CH = 4128.
- Tags 0x0038 and 0x0039, big-endian maker note (S17): entry 00 38 00 03 00 00 00 02 00 08 00 08 gives x0 = 8, y0 = 8 (inline value). Tag 0x0039 values 0x0F20 0x0A20 give CW = 3872, CH = 2592.

### TV-5: Huffman table, 12-bit, big-endian (K10D, S17)

Tag 0x0220, 53 bytes:

0x00 0x01 0x00 0x00 0x00 0x16 0x00 0x28 0x00 0x49 0x00 0x23 0x00 0x4D 0x0F 0x00 0x0C 0x00 0x08 0x00 0x00 0x00 0x04 0x00 0x0A 0x00 0x0E 0x00 0x0F 0x80 0x0F 0xC0 0x0F 0xE0 0x0F 0xF0 0x0F 0xF8 0x0F 0xFC 0x05 0x03 0x03 0x02 0x02 0x03 0x04 0x06 0x07 0x08 0x09 0x0A 0x0A

V = 1, N = 13. Bytes 2 to 13 are not used.

| SSSS | C | L | Code |
| --- | --- | --- | --- |
| 0 | 0x0F00 | 5 | 11110 |
| 1 | 0x0C00 | 3 | 110 |
| 2 | 0x0800 | 3 | 100 |
| 3 | 0x0000 | 2 | 00 |
| 4 | 0x0400 | 2 | 01 |
| 5 | 0x0A00 | 3 | 101 |
| 6 | 0x0E00 | 4 | 1110 |
| 7 | 0x0F80 | 6 | 111110 |
| 8 | 0x0FC0 | 7 | 1111110 |
| 9 | 0x0FE0 | 8 | 11111110 |
| 10 | 0x0FF0 | 9 | 111111110 |
| 11 | 0x0FF8 | 10 | 1111111110 |
| 12 | 0x0FFC | 10 | 1111111111 |

### TV-6: Huffman table, 14-bit, little-endian (K-3, K-1, K-3 Mark III and others)

Tag 0x0220, 59 bytes:

0x03 0x00 0x00 0x00 0x0C 0x00 0x00 0x00 0x00 0x00 0x00 0x00 0x00 0x00 0xE0 0x0F 0xC0 0x0F 0x80 0x0F 0x00 0x0F 0x00 0x0E 0x00 0x0A 0x00 0x0C 0x00 0x08 0x00 0x00 0x00 0x04 0xF0 0x0F 0xF8 0x0F 0xFC 0x0F 0xFE 0x0F 0xFF 0x0F 0x08 0x07 0x06 0x05 0x04 0x03 0x03 0x03 0x02 0x02 0x09 0x0A 0x0B 0x0C 0x0C

V = 3, N = 15. Codes for SSSS 0 to 14: 11111110, 1111110, 111110, 11110, 1110, 101, 110, 100, 00, 01, 111111110, 1111111110, 11111111110, 111111111110, 111111111111.

### TV-7: Huffman stream, 4 x 3 pixels, table of TV-5

Input stream: 0xFB 0x27 0xEC 0x89 0xAB 0xFB 0x4B 0xB2 0x14 0xA2 0x59 0x87 (94 coded bits and 2 padding bits).

| Pixel (x, y) | Code bits | SSSS | Extra bits | E | D | P | Value |
| --- | --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 111110 | 7 | 1100100 | 100 | 100 | 0 | 100 |
| (1, 0) | 1111110 | 8 | 11001000 | 200 | 200 | 0 | 200 |
| (2, 0) | 100 | 2 | 11 | 3 | 3 | 100 | 103 |
| (3, 0) | 01 | 4 | 0101 | 5 | -10 | 200 | 190 |
| (0, 1) | 1111110 | 8 | 11010010 | 210 | 210 | 0 | 210 |
| (1, 1) | 1110 | 6 | 110010 | 50 | 50 | 0 | 50 |
| (2, 1) | 00 | 3 | 010 | 2 | -5 | 210 | 205 |
| (3, 1) | 100 | 2 | 10 | 2 | 2 | 50 | 52 |
| (0, 2) | 100 | 2 | 01 | 1 | -2 | 100 | 98 |
| (1, 2) | 00 | 3 | 101 | 5 | 5 | 200 | 205 |
| (2, 2) | 100 | 2 | 11 | 3 | 3 | 98 | 101 |
| (3, 2) | 00 | 3 | 001 | 1 | -6 | 205 | 199 |

Expected raster: 100 200 103 190 / 210 50 205 52 / 98 205 101 199. The last two bits (11) are padding. A decoder that predicts row 1 from row 0 or row 2 from row 1 fails this vector.

### TV-8: Huffman stream of a camera file, 12-bit (K10D, S17)

The strip at offset 85472 starts with 0xEA 0x3E 0x93 0x8C 0x90 0x20 0xF4 0x89 0x18 0x4F. With the table of TV-5, row 0 starts:

| x | Code bits | SSSS | Extra bits | D | P | Value |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 1110 | 6 | 101000 | 40 | 0 | 40 |
| 1 | 111110 | 7 | 1001001 | 73 | 0 | 73 |
| 2 | 110 | 1 | 0 | -1 | 40 | 39 |
| 3 | 01 | 4 | 1001 | 9 | 73 | 82 |
| 4 | 00 | 3 | 100 | 4 | 39 | 43 |
| 5 | 00 | 3 | 001 | -6 | 82 | 76 |
| 6 | 00 | 3 | 000 | -7 | 43 | 36 |
| 7 | 11110 | 0 | none | 0 | 76 | 76 |
| 8 | 100 | 2 | 10 | 2 | 36 | 38 |
| 9 | 00 | 3 | 100 | 4 | 76 | 80 |

The whole strip decodes to values 0 to 4095. The last code word of the last pixel ends on the last bit of the strip: 65014288 coded bits = 8 * 8126786 (M5). A decoder that needs bytes after the strip for its look-ahead must supply zero bits (R-PEF-35).

### TV-9: Huffman stream of a camera file, 14-bit, little-endian (K-3 Mark III, S22)

The strip at offset 204800 starts with 0x60 0xCC 0x03 0xE7 0xEB 0xFB 0xFB 0xC3. With the table of TV-6, row 0 starts: (code 01, SSSS 9, extra 100000110, D 262, value 262), (01, 9, 100000000, 256, value 256), (111110, 2, 01, D -2, P 262, value 260), (111110, 2, 10, D 2, P 256, value 258), (11111110, 0, value 260), (11111110, 0, value 258), (11110, 3, 000, D -7, P 260, value 253), (11111110, 0, value 258). These masked pixels lie near the BlackPoint 256 of the file.

### TV-10: P1 and P2 samples

- P1, big-endian: 0x00 0xE6 0x01 0x1C 0x00 0xDD 0x01 0x13 give 230, 284, 221, 275 (S15). 0x04 0x68 gives 1128.
- P1, little-endian (rule R-PEF-25, no measured file): 0x68 0x04 gives 1128.
- P2: 0x89 0xD0 0x92 0x09 0x40 0x97 give 2205, 146, 148, 151 (S16). 0x46 0x88 0x50 gives 1128, 2128. 0x85 0x04 0x04 gives 2128, 1028.

### TV-11: levels, white balance and matrix

CFA cell R 1128, G 2128, G 2128, B 1028. BlackPoint 128 128 128 128. No tag 0x007E, BPS 12, so w = 4095. WhitePoint 13760 8192 8192 9440. Model "PENTAX K10D". Normalized and balanced: 0.4234150, 0.5041593, 0.2614342. Output linear sRGB for a flat area of this cell: 0.3922853, 0.5713001, 0.2077639.

Second case (fixture F4): cell R 4256, G 8256, G 8256, B 3856. BlackPoint 256 256 256 256. Tag 0x007E 16376. WhitePoint 20352 8192 8192 14912. Model "PENTAX K-3 Mark III" (generic matrix). Normalized and balanced: (4000 / 16120) * 2.484375 = 0.6164702, 8000 / 16120 = 0.4962779, (3600 / 16120) * 1.8203125 = 0.4065214. Normalized generic rows: (1.7782218, -0.6703297, -0.1078921), (-0.1391391, 1.4874875, -0.3483483), (0.0559441, -0.4565435, 1.4005994). Output: 0.7196905, 0.5108210, 0.3772890.

### TV-12: P4 normalization

BlackPoint 64 64 64 64, tag 0x007E 16378: s = 64 gives 0. s = 1064 gives 1000 / 16314 = 0.0612970. s = 8221 gives 8157 / 16314 = 0.5. s = 16378 gives 1. Output r = g = b = v.

### TV-13: orientation

Stored 3 x 2 image with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a.

### Fixture recipes

The fixtures are complete small PEF files for the decoder tests. They have no preview, so a decoding failure is an error. Tags are written in ascending order. Values longer than 4 bytes follow their IFD at even offsets.

Common layout of all fixtures:
- Offset 0x00: the TIFF header of TV-1 (big-endian for F1 to F3, little-endian for F4 and F5).
- Offset 0x08: IFD0 with 12 entries: 0x0100 LONG W, 0x0101 LONG H, 0x0102 SHORT BPS, 0x0103 SHORT Compression, 0x0106 SHORT PhotometricInterpretation, 0x0110 ASCII Model, 0x0111 LONG StripOffsets, 0x0112 SHORT 1, 0x0115 SHORT 1, 0x0116 LONG H, 0x0117 LONG StripByteCounts, 0x8769 LONG Exif IFD offset. Next-IFD offset 0. The Model string follows the IFD.
- Then the Exif IFD with 2 entries (1 entry in F5): 0x927C UNDEFINED (maker note) and 0xA302 UNDEFINED count 8 (CFA pattern 00 02 00 02 00 01 01 02). Next-IFD offset 0. The CFA bytes follow it, then the maker note.
- The sensor strip starts at the next multiple of 16 after the maker note.

F1 (P1, 4 x 2 pixels, big-endian, AOC maker note with tags 0x0200 = 128 128 128 128 and 0x0201 = 13760 8192 8192 9440, no crop tags), complete file of 288 bytes:

0000: 4D 4D 00 2A 00 00 00 08 00 0C 01 00 00 04 00 00
0010: 00 01 00 00 00 04 01 01 00 04 00 00 00 01 00 00
0020: 00 02 01 02 00 03 00 00 00 01 00 0C 00 00 01 03
0030: 00 03 00 00 00 01 00 01 00 00 01 06 00 03 00 00
0040: 00 01 80 23 00 00 01 10 00 02 00 00 00 0C 00 00
0050: 00 9E 01 11 00 04 00 00 00 01 00 00 01 10 01 12
0060: 00 03 00 00 00 01 00 01 00 00 01 15 00 03 00 00
0070: 00 01 00 01 00 00 01 16 00 04 00 00 00 01 00 00
0080: 00 02 01 17 00 04 00 00 00 01 00 00 00 10 87 69
0090: 00 04 00 00 00 01 00 00 00 AA 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 31 30 44 00 00 02 92 7C 00 07
00B0: 00 00 00 34 00 00 00 D0 A3 02 00 07 00 00 00 08
00C0: 00 00 00 C8 00 00 00 00 00 02 00 02 00 01 01 02
00D0: 41 4F 43 00 4D 4D 00 02 02 00 00 03 00 00 00 04
00E0: 00 00 00 F4 02 01 00 03 00 00 00 04 00 00 00 FC
00F0: 00 00 00 00 00 80 00 80 00 80 00 80 35 C0 20 00
0100: 20 00 24 E0 00 00 00 00 00 00 00 00 00 00 00 00
0110: 04 68 08 50 04 68 08 50 08 50 04 04 08 50 04 04

Annotations: IFD0 at 0x08 (W 4, H 2, BPS 12, Compression 1, PhotometricInterpretation 32803, Model at 0x9E "PENTAX K10D", StripOffsets 0x110, StripByteCounts 16, Exif IFD at 0xAA). Exif IFD at 0xAA: MakerNote count 0x34 at 0xD0, CFAPattern at 0xC8. Maker note at 0xD0: AOC form, 2 entries, 0x0200 at 0xF4 and 0x0201 at 0xFC (origin 0). Sensor data at 0x110: rows 1128 2128 1128 2128 and 2128 1028 2128 1028. Expected output: 4 x 2 pixels, every pixel 0.3922853, 0.5713001, 0.2077639 (TV-11).

F2 (P2, 4 x 2 pixels): F1 with these changes: Compression = 32773 (bytes 0x36 to 0x37 = 80 05), StripByteCounts = 12 (byte 0x8D = 0C), and the 12 data bytes at 0x110: 46 88 50 46 88 50 85 04 04 85 04 04. File length 284 bytes. Expected output as F1.

F3 (P3, 8 x 4 pixels, big-endian, AOC maker note, 12-bit, crop rectangle (1, 0, 4, 4) by tags 0x0038 and 0x0039, table of TV-5), complete file of 406 bytes:

0000: 4D 4D 00 2A 00 00 00 08 00 0C 01 00 00 04 00 00
0010: 00 01 00 00 00 08 01 01 00 04 00 00 00 01 00 00
0020: 00 04 01 02 00 03 00 00 00 01 00 0C 00 00 01 03
0030: 00 03 00 00 00 01 FF FF 00 00 01 06 00 03 00 00
0040: 00 01 80 23 00 00 01 10 00 02 00 00 00 0C 00 00
0050: 00 9E 01 11 00 04 00 00 00 01 00 00 01 60 01 12
0060: 00 03 00 00 00 01 00 01 00 00 01 15 00 03 00 00
0070: 00 01 00 01 00 00 01 16 00 04 00 00 00 01 00 00
0080: 00 04 01 17 00 04 00 00 00 01 00 00 00 36 87 69
0090: 00 04 00 00 00 01 00 00 00 AA 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 31 30 44 00 00 02 92 7C 00 07
00B0: 00 00 00 8D 00 00 00 D0 A3 02 00 07 00 00 00 08
00C0: 00 00 00 C8 00 00 00 00 00 02 00 02 00 01 01 02
00D0: 41 4F 43 00 4D 4D 00 05 00 38 00 03 00 00 00 02
00E0: 00 01 00 00 00 39 00 03 00 00 00 02 00 04 00 04
00F0: 02 00 00 03 00 00 00 04 00 00 01 18 02 01 00 03
0100: 00 00 00 04 00 00 01 20 02 20 00 07 00 00 00 35
0110: 00 00 01 28 00 00 00 00 00 80 00 80 00 80 00 80
0120: 35 C0 20 00 20 00 24 E0 00 01 00 00 00 16 00 28
0130: 00 49 00 23 00 4D 0F 00 0C 00 08 00 00 00 04 00
0140: 0A 00 0E 00 0F 80 0F C0 0F E0 0F F0 0F F8 0F FC
0150: 05 03 03 02 02 03 04 06 07 08 09 0A 0A 00 00 00
0160: FF FF FF FF 85 0F FD 1A 3D EF FB D7 FF F7 2F EF
0170: FF FF FF E8 09 FF 05 0F 7B FF BF BF FB D7 FB DE
0180: FF D1 A3 DE FF BD 7F FF 72 FE F7 BF E0 A1 EF 7F
0190: F7 F7 FF 7A FF 7F

Annotations: IFD0 (W 8, H 4, BPS 12, Compression 65535, StripOffsets 0x160, StripByteCounts 0x36 = 54). Maker note at 0xD0: AOC form, 5 entries: 0x0038 = 1, 0 and 0x0039 = 4, 4 (inline), 0x0200 at 0x118, 0x0201 at 0x120, 0x0220 (53 bytes) at 0x128. The strip codes 425 bits and ends with 7 padding 1-bits. Expected decoded raster (R-PEF-29 to R-PEF-34):

| | x=0 | x=1 | x=2 | x=3 | x=4 | x=5 | x=6 | x=7 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| y=0 | 4095 | 2128 | 1128 | 2128 | 1128 | 4095 | 4095 | 4095 |
| y=1 | 4095 | 1028 | 2128 | 1028 | 2128 | 4095 | 4095 | 4095 |
| y=2 | 4095 | 2128 | 1128 | 2128 | 1128 | 4095 | 4095 | 4095 |
| y=3 | 4095 | 1028 | 2128 | 1028 | 2128 | 4095 | 4095 | 4095 |

Columns 0, 5, 6 and 7 lie outside the crop rectangle. Expected output: 4 x 4 pixels, every pixel 0.3922853, 0.5713001, 0.2077639. A decoder that lets the outside value 4095 enter the demosaicing, or that crops before applying the CFA phase of stored pixel (0, 0), fails this fixture. The first 4 bytes FF FF FF FF are the code 1111111111 of SSSS 12 and its 12 extra bits for the difference 4095, followed by more 1-bits: a decoder that removes a byte after FF fails this fixture.

F4 (P3, 8 x 4 pixels, little-endian, PENTAX maker note, 14-bit, crop rectangle (2, 1, 4, 2) by tag 0x023E, table of TV-6), complete file of 429 bytes:

0000: 49 49 2A 00 08 00 00 00 0C 00 00 01 04 00 01 00
0010: 00 00 08 00 00 00 01 01 04 00 01 00 00 00 04 00
0020: 00 00 02 01 03 00 01 00 00 00 0E 00 00 00 03 01
0030: 03 00 01 00 00 00 FF FF 00 00 06 01 03 00 01 00
0040: 00 00 23 80 00 00 10 01 02 00 14 00 00 00 9E 00
0050: 00 00 11 01 04 00 01 00 00 00 80 01 00 00 12 01
0060: 03 00 01 00 00 00 01 00 00 00 15 01 03 00 01 00
0070: 00 00 01 00 00 00 16 01 04 00 01 00 00 00 04 00
0080: 00 00 17 01 04 00 01 00 00 00 2D 00 00 00 69 87
0090: 04 00 01 00 00 00 B2 00 00 00 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 2D 33 20 4D 61 72 6B 20 49 49
00B0: 49 00 02 00 7C 92 07 00 A0 00 00 00 D8 00 00 00
00C0: 02 A3 07 00 08 00 00 00 D0 00 00 00 00 00 00 00
00D0: 00 02 00 02 00 01 01 02 50 45 4E 54 41 58 20 00
00E0: 49 49 05 00 7E 00 04 00 01 00 00 00 F8 3F 00 00
00F0: 00 02 03 00 04 00 00 00 4C 00 00 00 01 02 03 00
0100: 04 00 00 00 54 00 00 00 20 02 07 00 3B 00 00 00
0110: 5C 00 00 00 3E 02 07 00 08 00 00 00 98 00 00 00
0120: 00 00 00 00 00 01 00 01 00 01 00 01 80 4F 00 20
0130: 00 20 40 3A 03 00 00 00 0C 00 00 00 00 00 00 00
0140: 00 00 E0 0F C0 0F 80 0F 00 0F 00 0E 00 0A 00 0C
0150: 00 08 00 00 00 04 F0 0F F8 0F FC 0F FE 0F FF 0F
0160: 08 07 06 05 04 03 03 03 02 02 09 0A 0B 0C 0C 00
0170: 02 00 01 00 04 00 02 00 00 00 00 00 00 00 00 00
0180: FE FE FE FE FE FE FE FE FF F8 10 3F F7 88 7F 7F
0190: 7F 7F 7F 7F 7F F4 28 3F FE 04 0F EF EF EF EF EF
01A0: EF FE FD FF FF E1 DF FD FD FD FD FD FD

Annotations: IFD0 (W 8, H 4, BPS 14, Compression 65535, Model at 0x9E "PENTAX K-3 Mark III", StripOffsets 0x180, StripByteCounts 0x2D = 45). Exif IFD at 0xB2. Maker note at M = 0xD8: PENTAX form, little-endian, 5 entries at 0xE2. Stored offsets count from M: 0x0200 at 0xD8 + 0x4C = 0x124, 0x0201 at 0x12C, 0x0220 at 0x134, 0x023E at 0x170 (bytes 02 00 01 00 04 00 02 00: x0 2, y0 1, CW 4, CH 2). Tag 0x007E = 0x3FF8 = 16376 (inline). The strip codes 359 bits. Expected decoded raster: row 0 all 0, row 1 = 8256 3856 8256 3856 8256 3856 8256 3856, row 2 = 4256 8256 4256 8256 4256 8256 4256 8256, row 3 all 16383. Expected output: 4 x 2 pixels, every pixel 0.7196905, 0.5108210, 0.3772890 (TV-11, second case). A decoder that reads the maker-note offsets from the file start reads wrong levels and fails this fixture.

F5 (P4, 8 x 2 pixels, little-endian, PENTAX maker note, Model "PENTAX K-3 Mark III Monochrome", PhotometricInterpretation 34892, no CFAPattern, no crop tags, table of TV-6), complete file of 375 bytes:

0000: 49 49 2A 00 08 00 00 00 0C 00 00 01 04 00 01 00
0010: 00 00 08 00 00 00 01 01 04 00 01 00 00 00 02 00
0020: 00 00 02 01 03 00 01 00 00 00 0E 00 00 00 03 01
0030: 03 00 01 00 00 00 FF FF 00 00 06 01 03 00 01 00
0040: 00 00 4C 88 00 00 10 01 02 00 1F 00 00 00 9E 00
0050: 00 00 11 01 04 00 01 00 00 00 50 01 00 00 12 01
0060: 03 00 01 00 00 00 01 00 00 00 15 01 03 00 01 00
0070: 00 00 01 00 00 00 16 01 04 00 01 00 00 00 02 00
0080: 00 00 17 01 04 00 01 00 00 00 27 00 00 00 69 87
0090: 04 00 01 00 00 00 BE 00 00 00 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 2D 33 20 4D 61 72 6B 20 49 49
00B0: 49 20 4D 6F 6E 6F 63 68 72 6F 6D 65 00 00 01 00
00C0: 7C 92 07 00 77 00 00 00 D0 00 00 00 00 00 00 00
00D0: 50 45 4E 54 41 58 20 00 49 49 03 00 7E 00 04 00
00E0: 01 00 00 00 FA 3F 00 00 00 02 03 00 04 00 00 00
00F0: 34 00 00 00 20 02 07 00 3B 00 00 00 3C 00 00 00
0100: 00 00 00 00 40 00 40 00 40 00 40 00 03 00 00 00
0110: 0C 00 00 00 00 00 00 00 00 00 E0 0F C0 0F 80 0F
0120: 00 0F 00 0E 00 0A 00 0C 00 08 00 00 00 04 F0 0F
0130: F8 0F FC 0F FE 0F FF 0F 08 07 06 05 04 03 03 03
0140: 02 02 09 0A 0B 0C 0C 00 00 00 00 00 00 00 00 00
0150: 90 3F E8 51 FF 7D 0F FB E8 7F DF 43 FE FA 1F F7
0160: D0 FF FB 0C BF FF FF AF FF 80 77 FF C0 45 FF E0
0170: 23 11 06 41 90 64 7F

Annotations: IFD0 (W 8, H 2, BPS 14, Compression 65535, PhotometricInterpretation 0x884C = 34892, Model at 0x9E, StripOffsets 0x150, StripByteCounts 0x27 = 39). Exif IFD at 0xBE with the MakerNote entry only. Maker note at M = 0xD0: PENTAX form, 3 entries: 0x007E = 16378 (inline), 0x0200 at 0xD0 + 0x34 = 0x104 (64 64 64 64), 0x0220 at 0x10C. The strip codes 305 bits.

Expected decoded raster: 64 1064 2064 3064 4064 5064 6064 16378 / 16378 8221 64 100 200 300 400 500. Expected gray output v (r = g = b = v): 0, 0.0612970, 0.1225941, 0.1838911, 0.2451882, 0.3064852, 0.3677823, 1 / 1, 0.5, 0, 0.0022067, 0.0083364, 0.0144661, 0.0205958, 0.0267255.

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 Table bytes 2 to 13 of tag 0x0220. AOC files hold 00 00 00 16 and four 16-bit values that differ between files (for example 0x0028 0x0049 0x0023 0x004D in K10D, zeros in K-7, K-x and K-5). PENTAX files hold 00 00 0C 00 and zeros. Their meaning is unknown. Fallback: ignored (R-PEF-27).
- OQ-2 Order of the four BlackPoint values (tag 0x0200): color order (red, green, green, blue) like tag 0x0201, or CFA-position order like the DNG BlackLevel tag (S3). The evidence is mixed. In the BGGR models K-5, K-5 II and K-x, the camera DNG of S26 has the same first and last BlackLevel values as tag 0x0200 of the PEF of S18 and S24 (for example 65 65 66 66 in both K-5 files), which favors the position order if the camera writes one array to both formats. The masked pixels of the K-5 PEF are 0.4 higher at the blue position than at the red position, which favors the color order. The measured values differ by at most 2 between the red and blue entries. Fallback: color order with the mean of the two green values (R-PEF-38).
- OQ-3 K-70 black level. In the K-70 file (S20, ISO 400) the masked pixels and the darkest image pixels lie near 130, while tag 0x0200 is 64. The KF, which has the same raster size, agrees with its tag. No tag with the value 128 or 130 was found in the maker note. Fallback: tag 0x0200 (R-PEF-38). The image is slightly lifted in the shadows.
- OQ-4 Compression 32773. S5 and S1 name it PackBits. All measured files store plain 12-bit packed data with the exact unpacked length. A PackBits-coded strip was not observed. Fallback: P2 only when StripByteCounts = 3 * W * H / 2 (R-PEF-3), preview path otherwise.
- OQ-5 P1 in little-endian files, P1 with BPS other than 12, and P1 saturation. Only one big-endian P1 file was measured, and it has no saturated pixels. Fallback: R-PEF-25 for both byte orders, BPS 12 only, white level 4095.
- OQ-6 Pixel Shift and HDR composites (S10). The files store three or two further raw frames in SubIFDs (M15). The sub-pixel shifts and the combination method are not documented. Fallback: IFD0 frame only (R-PEF-52).
- OQ-7 Color matrices. S11 gives D50 matrices with 2 decimals for 19 models and nothing for others (for example K-S2, K-70, KF, KP, K-1 Mark II, K-3 Mark III, all P1 and P2 models, K2000, K-m). The D50 matrix is used for every scene illuminant. Maker-note tags 0x021C and 0x021D hold one constant matrix pair for all models. Measured against the camera JPEG, 0x021D performs like the S11 matrices (M13), but its meaning is not documented. Fallback: R-PEF-43 with the generic matrix.
- OQ-8 White level semantics. Tag 0x007E lies below the largest stored values in several models (M10). The camera DNG files use the same value. Fallback: R-PEF-37. Values above it clip.
- OQ-9 Maker-note tag 0x023D. Its first four values equal the DNG ActiveArea (left, top, width, height). Values 5 to 8 repeat tag 0x023E. Values 9 and 10 are unknown. Fallback: not used.
- OQ-10 Models that were not measured (for example K-m, K-01, K-30, K-50, K-500, *ist DL2, K-1 Mark II firmware variants). Fallback: classification by structure (R-PEF-3).
- OQ-11 Orientation of rotated shots. All measured files have Orientation 1 or no Orientation tag. Fallback: R-PEF-50, R-PEF-56.
- OQ-12 Several strips. Not observed. Fallback: preview path (R-PEF-20).
- OQ-13 Highlights. Values above the white level and white-balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-PEF-41.
- OQ-14 Default white level of 12-bit models without tag 0x007E. The K-x camera DNG states 4094, this document uses 4095. Fallback: R-PEF-37.
- OQ-15 Monochrome color. P4 is delivered as neutral gray in linear sRGB. No spectral data of the monochrome sensor are documented. Fallback: R-PEF-47.
- OQ-16 Tag 0x0201 in P4 and in files where it is absent. Not present in P4. Not observed absent in color files. Fallback: R-PEF-40 sources 2 and 3.
- OQ-17 Maker-note preview (tags 0x0003 and 0x0004). Its offset origin follows R-PEF-16 in the measured files, but it is smaller than the IFD2 preview. Fallback: not used.
- OQ-18 Data outside the crop rectangle. Some columns hold masked pixels, others hold constant or alternating values (M9). Their purpose is not documented. Fallback: not used.
