# Nikon NEF raw format: decoder specification

Document: specs/raw/nef.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite NEF decoder (clean-room step 1 of 2).

This document describes the Nikon NEF raw file format as facts. It is written for an implementer who has only this document, TIFF 6.0 (S1), ITU-T T.81 (S2) and the Sony specification `specs/raw/arw.md` of this repository, from which it reuses the demosaicing and orientation rules by reference. Every normative statement is traceable to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Conventions

- Requirements have the form R-NEF-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x4D 0x4D or "4D 4D".
- Bit 0 is the least significant bit of a value. A bit string such as 11101 is written with the first bit read on the left.
- floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0). ceil(a / b) rounds toward plus infinity.
- Pixel coordinates are (x, y): x is the column, y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (tags 0x0100 and 0x0101 of the raw IFD). N bits per sample means the value of BitsPerSample (tag 0x0102), written bps.
- "Maker-note byte order" is the byte order of the TIFF header inside the maker note (section Container layout). It can differ from the file byte order in principle. In all measured files it is equal.

### Glossary

- NEF: Nikon Electronic Format, the raw file format of Nikon D-series, Z-series, Nikon 1 and some Coolpix cameras. NEF is a TIFF file.
- IFD, IFD0, SubIFD: as in arw.md. The raw IFD is the SubIFD that holds the sensor data.
- Maker note: the Exif tag 0x927C. In NEF it holds a complete small TIFF structure with Nikon tags.
- CFA, CFA position: as in arw.md. CFA position of pixel (x, y) is 2 * (y mod 2) + (x mod 2).
- NEFCompression: the Nikon storage method code. It is stored in maker-note tag 0x0093, or in maker-note tag 0x0051 on Z-series cameras (S5).
- Linearization tag: maker-note tag 0x0096, called NEFLinearizationTable by S5. It holds the start predictors and, for lossy files, the tone curve.
- Variant: one storage method of the sensor data. This document defines eight variants, V1 to V8, in the section Scope and variants.
- Huffman variant: V6, V7 or V8. The sensor data is a prefix-coded bit stream of pixel differences.
- Code table: one of the four prefix code tables L12, L14, Y12, Y14 of section Compression.
- Category: the symbol decoded with a code table. Category s is the number of extra bits that follow the code, as in the DC coding of S2.
- Code value: the value that a Huffman variant reconstructs for a pixel. In V6 it is the sensor value. In V7 and V8 it is an index into the curve.
- Curve: the table that maps a V7 or V8 code value to a linear sensor value.
- Black level, white level, WB multipliers, camera RGB, linear sRGB: as in arw.md.
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Container layout, Metadata tags, Sensor data layout, Embedded preview |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Compression (Annex C code generation, F.2.2.1 difference extension), Embedded preview |
| S3 | Exif Version 2.32, CIPA DC-X008-Translation-2019 | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-14 | Sensor data layout (CFAPattern meaning and order) |
| S4 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (output primaries and D65 white) |
| S5 | ExifTool Tag Names: Nikon tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/Nikon.html | community documentation | 2026-11-14 | Scope and variants (NEFCompression values), Metadata tags (0x000C, 0x0011, 0x003D, 0x0045, 0x0051, 0x0093, 0x0096, 0x0097), Open questions |
| S6 | ExifTool Tag Names: EXIF tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/EXIF.html | community documentation | 2026-11-14 | Metadata tags (Compression 34713 "Nikon NEF Compressed", JpgFromRaw in the SubIFD of NEF) |
| S7 | B. Claff, NEF Compression, PhotonsToPhotos, prepared 2006-03-19, updated 2021-01-28 (prose and tables) | https://photonstophotos.net/NikonInfo/NEF_Compression.htm | community documentation | 2026-11-14 | Scope and variants (lossy encoding followed by non-adaptive Huffman compression, lossless by skipping the encoding table, encoding table sizes per model), Compression (curves) |
| S8 | B. Claff, NEF encoding value tables 567, 683, 689, 769, 1025, 1025v2, 2753, 3073, 4097, 4097v2 (text files linked from S7, one value per line) | https://photonstophotos.net/NikonInfo/NEF_Compression_689.txt | community documentation | 2026-11-14 | Compression (curve expansion, record M9) |
| S9 | DXOMARK camera sensor Measurements pages, tab Color response (one page per model, URL pattern https://www.dxomark.com/Cameras/Nikon/<name>---Measurements, names in the Color conversion table) | https://www.dxomark.com/Cameras/Nikon/D850---Measurements | community documentation | 2026-11-14 | Color conversion (matrices, D50 white balance scales), Levels and white balance |
| S10 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear demosaicing, as in arw.md) |
| S11 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-14 | Sensor data layout (Bayer arrangement) |
| S12 | S. Okada, Nikon Corporation, Image compressor for generating predicted difference code having fixed bit length by requantizing with coarser quantization steps, US Patent 7,421,132 B2 (priority 2003-12-25) | https://patents.google.com/patent/US7421132B2/en | patent | 2026-11-14 | Compression (prediction of a Bayer pixel from the second pixel to its left, same color, as the starting hypothesis of record M4) |
| S13 | Measurement: D700 sample set, same scene in six storage methods: 12bit uncompressed (2256), 12bit lossless (2254), 12bit lossy type 2 (2255), 14bit uncompressed (2251), 14bit lossless (2249), 14bit lossy type 2 (2250), public domain (CC0) files | https://raw.pixls.us/data/Nikon/D700/ | measurement | 2026-11-14 | all sections, records M3 to M9, M13, M14 |
| S14 | Measurement: D3 sample set, same scene: 12bit lossy (3516), 12bit lossless (3518), 14bit lossy (3519), 14bit lossless (3520), 14bit uncompressed (3521) (CC0) | https://raw.pixls.us/data/Nikon/D3/ | measurement | 2026-11-14 | Compression (tables L14, Y12, Y14), records M5 to M8, M14 |
| S15 | Measurement: Z 6 sample set, same scene: 14bit uncompressed (3580), 14bit lossy (3581), 14bit lossless (3582), 12bit uncompressed (3583) (CC0) | https://raw.pixls.us/data/Nikon/Z%206/ | measurement | 2026-11-14 | Compression (V4, V5, V6, V8 version 0x44 0x40), records M3, M6, M8, M9, M14 |
| S16 | Measurement: D4S samples 12bit uncompressed (2222), 14bit uncompressed (2224), small raw (2227) (CC0) | https://raw.pixls.us/data/Nikon/D4S/ | measurement | 2026-11-14 | Compression (V1 little-endian, V4), Scope and variants (X2), records M3, M14, M17 |
| S17 | Measurement: D850 samples 12bit lossless RAW size S (1835, ImageSizeRAW 0x003E = 3), 12bit lossless RAW size M (1836, ImageSizeRAW = 2), 14bit lossless size L (1840) (CC0) | https://raw.pixls.us/data/Nikon/D850/ | measurement | 2026-11-14 | Compression (V6), Levels, Sensor data layout (CropArea), records M5, M6, M11, M14, M15 |
| S18 | Measurement: D750 samples 12bit lossy (895), 14bit lossless (898) (CC0) | https://raw.pixls.us/data/Nikon/D750/ | measurement | 2026-11-14 | Compression (V6, V8), records M6, M7, M11, M14 |
| S19 | Measurement: D6 sample 12bit lossy, version 0x44 0x40 (4726) (CC0) | https://raw.pixls.us/data/Nikon/D6/ | measurement | 2026-11-14 | Compression (V8 version 0x44 0x40), records M7, M9, M14 |
| S20 | Measurement: D70 sample 20170902_0047 lossy type 1 (2060) (CC0) | https://raw.pixls.us/data/Nikon/D70/ | measurement | 2026-11-14 | Compression (V7), Levels and white balance, records M7, M12, M14 |
| S21 | Measurement: D100 sample DSC_2376 (2996) (CC0) | https://raw.pixls.us/data/Nikon/D100/ | measurement | 2026-11-14 | Compression (V3), records M3, M14 |
| S22 | Measurement: D200 samples uncompressed (824) and lossy type 1 (8079) (CC0) | https://raw.pixls.us/data/Nikon/D200/ | measurement | 2026-11-14 | Compression (V2, V7), records M3, M7, M14 |
| S23 | Measurement: D2H samples uncompressed (5632) and lossy type 1 (5227), D2X samples uncompressed (9064) and lossy type 1 (9065) (CC0) | https://raw.pixls.us/data/Nikon/D2X/ | measurement | 2026-11-14 | Compression (V2, V7), Levels and white balance, records M7, M11, M12, M14 |
| S24 | Measurement: Z 9 sample High Efficiency (5148), 1 V3 sample (2802), D1X samples compressed (1785) and uncompressed (1786), E8400 sample (2082), D810 small raw (2036) (CC0) | https://raw.pixls.us/data/Nikon/Z%209/ | measurement | 2026-11-14 | Scope and variants (X1 to X5), records M11, M17, M18 |
| S25 | Measurement: container survey, first 768 KiB of the smallest CC0 NEF file for each model and storage mode of the archive, 217 files of 82 models, 208 parsed (the maker notes of Nikon 1 AW1, J1, J2, J3, V1, V2, D50, D70 and D70s lay beyond the downloaded range) (CC0 archive) | https://raw.pixls.us/data/Nikon/ | measurement | 2026-11-14 | Scope and variants, Container layout, Metadata tags, Compression (curve steps), records M1, M2, M9, M10, M12 |

Sample files are from the public raw sample archive raw.pixls.us. The archive lists a CC0 public-domain dedication for each file used here (its repository table, column license). The numbers in parentheses are the archive file ids (download URL pattern https://raw.pixls.us/getfile.php/<id>/nice/<name>). The author downloaded the files and inspected them with measurement tools that the author wrote from S1 and S2 for this purpose. The tools are not part of the deliverable.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S13 to S25): every NEF file starts with 4D 4D 00 2A 00 00 00 08 or 49 49 2A 00 08 00 00 00. Big-endian files: D1H, D1X, D100, D2H, D2Hs, D2X, D2Xs, D40, D40X, D50, D60, D70, D70s, D80, D90, D200, D300, D300S, D3, D3S, D3X, D700, D3000, D3100, D3200, D5000, D5100, D7000, D4, D600, D610, D800, D800E, Df, Coolpix A. Little-endian files: D3300 and later D3xxx models, D5200 and later D5xxx models, D7100 and later D7xxx models, D4S, D5, D6, D500, D750, D780, D810, D850, all Z models, Nikon 1 J4, J5, S1, V3, Coolpix E series. IFD0 holds a 160x120 RGB thumbnail (Compression 1, PhotometricInterpretation 2, 8 8 8 bits, one strip of 57600 bytes) or, on the Z 9 and later, a 160x120 JPEG. IFD0 tag 0x014A lists 1 to 6 SubIFDs. The raw IFD is the SubIFD with NewSubfileType 0 and PhotometricInterpretation 32803. The other SubIFDs have NewSubfileType 1 (JPEG previews, and on Z models further reduced images, some with Compression 34713, that are not used). The Exif IFD (0x8769) holds the maker note (0x927C).
- M2 (maker note, S25): the maker note starts with the 10 bytes 4E 69 6B 6F 6E 00 02 10 00 00 ("Nikon", a zero byte, version 02 10, two zero bytes) or 4E 69 6B 6F 6E 00 02 00 00 00 (D100, D1X), followed by a TIFF header (8 bytes, offset 8) at maker-note byte 10. All offsets inside the maker note count from the first byte of that TIFF header. The maker-note byte order equals the file byte order in all 208 parsed files.
- M3 (uncompressed layouts): V1 (S13 2251, S14 3521: big-endian; S16 2224: little-endian): 16-bit samples in the file byte order, values 0 to 16383, mean |x-2 difference| 22 to 264, the other byte order gives 8632 to 16106. V2 (S13 2256, S22 824, S23 5632 and 9064, D1X 1786, E8400 2082): 12-bit packing with the first pixel in the high 12 bits of a 24-bit big-endian group, mean |x-2 difference| 15 to 61, the other order gives 183 to 327. V3 (S21): every 16th byte of a row is 0, 15 bytes carry 10 pixels with the V2 packing, rows hold 304 groups (3040 pixels) for W = 3034. V4 (S16 2222, S15 3583) and V5 (S15 3580): least-significant-bit-first packing (pixel 0 in the low bits of byte 0), mean |x-2 difference| 6.4 to 38, the most-significant-bit-first reading gives 62 to 5248. Row stride = total strip bytes / H: 7404 = 1.5 * 4936 (D4S, no padding), 9104 = 1.5 * 6064 rounded up to a multiple of 16 (Z 6 12-bit), 10624 = 1.75 * 6064 rounded up to a multiple of 16 (Z 6 14-bit). The survey (S25) shows the same strides for D500, D6, D810, D850, Z 6_2, Z 7_2, Z fc.
- M4 (Huffman bit stream structure): the author tested the hypothesis "per pixel one prefix code that gives a category s, then s extra bits, difference by the F.2.2.1 rule of S2, prediction from the pixel two columns to the left (S12) or for columns 0 and 1 from two rows above, start values from tag 0x0096 bytes 2 to 9, most significant bit first, no byte stuffing". A beam search over prefix codes scored each candidate code by the distance between decoded values and the same-scene uncompressed file (S13 2256 for S13 2254). It converged on a canonical code (S2 Annex C) after 1200 pixels. With that code every listed Huffman file of S13 to S24 decodes all H rows. The bits used end in the last 4 bytes of the strip (30 bits at most), except the 1 V3 file (record M18). Decoded values never leave the allowed range. Byte stuffing is absent: the 14874979-byte strip of S13 2249 holds 53586 bytes 0xFF, and the exact bit count would fail if 0xFF 0x00 pairs were stuffed.
- M5 (table L12): found by the search of M4 on S13 2254, completed with the code 1111111 for category 12 at row 163, column 734 (decoded value 3666, uncompressed neighbor shot 3589 * 1.025). Full decodes: S13 2254 uses 94922644 of 94922648 strip bits, S14 3518 uses 80697315 of 80697320, S17 1835 and 1836 end 4 and 2 bits before the end. All values lie in 0 to 4095. Mean absolute difference to the same-scene uncompressed shot (exposure fitted) is 16 to 67, below the shot's own two-row difference of 46 to 92. All 13 codes occur.
- M6 (table L14): S14 3520 with the same-exposure S14 3521 as reference. The codes of the first 17 pixels fit table L12 with every category raised by 2, and the Z 6 file S15 3582 confirmed the codes 00 (7), 011 (8) and 101 (9). A search seeded with these eight codes then fixed 11100 (3), 111101 (2), 111110 (0) and 1111110 (1). Of the six orders of the categories 12, 13 and 14 on the remaining codes, only 12, 13, 14 decodes S14 3520 without a negative value (104129841 of 104129848 bits). The same table decodes S13 2249 (118999828 of 118999832 bits), S18 898, S17 1840 and S15 3582 (204682543 of 204682544 bits), all within 0 to 16383. All 15 codes occur.
- M7 (table Y12): S14 3516 with S14 3518 as reference, through the curve of M9. Nine codes came from the search. Tail layouts were tested by full decodes. Only the layout with 1111110 (9), an unused 8-bit code and 111111110 (10) decodes without negative values and reaches the curve maximum 688 exactly. Category 10 occurs 15 times (S14 3516) and 25 times (S18 895). The codes 11111110 and 111111111 never occur in the 9 decoded files S13 2255, S14 3516, S18 895, S19 4726, S20 2060, S22 8079, S23 5227 and 9065, S24 2802 (lossy type 2 and type 1).
- M8 (table Y14): S14 3519 with S14 3520 as reference. Codes 11111110 (11, 3967 times) and 111111110 (12, 13 times) were fixed by tail tests (no negative values, maximum code value 2752 = curve end). Full decodes of S14 3519 (3 bits left), S13 2250 (1 bit left) and S15 3581 (version 0x44 0x40, 6 bits left). Codes beginning with 111111111 never occur.
- M9 (curves): the expansion rule of section Compression reproduces the S8 tables exactly: 689 values (S13 2255, D700 12-bit), 2753 values (S13 2250, D700 14-bit), 1025 values (S18 895, D750 12-bit), 683 values (S20 2060, the curve is stored directly), 4097v2 values (S15 3581, Z 6 14-bit, step 16) and 1025v2 values (S19 4726, D6 12-bit, step 4). The survey (S25) shows the knot step 2^bps / 256 for version 0x44 0x20 (37 models) and 2^bps / 1024 for version 0x44 0x40 (D6, D780, Z 5, Z 6, Z 6_2, Z 7_2). All measured curves are non-decreasing.
- M10 (lossless linearization tag, S25): version 0x46 0x30 in every lossless file. Bytes 10 and 11 read as a 16-bit number give 32 (12-bit files) or 34 (14-bit files). They are not a curve length. The tag is 46 bytes long.
- M11 (black level): tag 0x003D holds four equal values on a 14-bit scale. 12-bit files reach down to the value / 4: S18 895 minimum code 150 (tag 600), S17 1835 minimum 101 (tag 400), S24 2802 (1 V3) 0.01 percentile 197 (tag 800), S16 2222 minimum 192 (tag 768, 768 / 4 = 192), S15 3583 minimum 249 and 0.01 percentile 253 (tag 1008, 1008 / 4 = 252). 14-bit files reach the tag value itself (S18 898 minimum 600, S17 1840 minimum 400, S16 2224 minimum 764 for tag 768, S15 3582 minimum 993 for tag 1008). Files without tag 0x003D (S13, S14, S20 to S23) have their darkest values at 0 to 46 and the D2H file holds 0.24 percent exact zeros (clipped at 0).
- M12 (white balance): tag 0x000C exists in every parsed file except D100, D2H, D70 (and Coolpix E series). Its first two rationals are the red and blue multipliers: with them the chromaticity error to the camera JPEG is 0.121 (S13 2254, no matrix) against 0.276 without white balance. D70 tag 0x0097 version "0103" holds 555, 256, 388, 256 at bytes 20 to 27 (R, G, B, G). D2H tag 0x0097 version "0102" holds 466, 256, 256, 463 (5632) and 563, 256, 256, 373 (5227) at bytes 10 to 17 (R, G, G, B). With these values the error falls from 0.103 to 0.093 (D70), 0.203 to 0.121 and 0.124 to 0.056 (D2H).
- M13 (matrix orientation, S13 2254): white-balanced camera RGB times the S9 D700 matrix, read with rows as output channels, gives a chromaticity error of 0.062 to the camera JPEG. The transposed matrix gives 0.237 and no matrix gives 0.121.
- M14 (agreement with the embedded JPEG, every in-scope variant): decoded by this document (2x2 cell binning in place of demosaicing), cropped by R-NEF-40, compared with the largest embedded JPEG at a common grid 512 pixels wide. Results in the table below. "Chroma" is the mean absolute difference of the (r, g) chromaticities after exposure normalization. "Luma" is the correlation of sRGB-encoded luminance.
- M15 (crop): tag 0x0045 equals the full-size JPEG area where both exist (S17 1840: 16, 8, 8256, 5504 and JPEG 8256x5504; S15: 8, 8, 6048, 4024). For files without the tag, the best alignment of raw and JPEG (search in steps of 2 pixels) is the centered position: S13 (16, 6) for 4288x2844 and 4256x2832, S22 (16, 12), S23 D2X (16, 10), S16 (4, 4), S18 (8, 8). S20 (D70) aligns at (14, 6), centered is (16, 7).
- M16 (previews): SubIFD JPEGs (Compression 6, tags 0x0201 and 0x0202) of RAW size L files are full size or nearly full size (3008x2000 D70, 4256x2832 D700, 8256x5504 D850). The D850 RAW sizes S and M carry the JPEG of their output size (4128x2752, 6192x4128). Newer models add a 1620x1080 JPEG in another SubIFD. The maker-note tag 0x0011 points to an IFD with a JPEG of 570x375 (564x372 on the D70) or 640x424. D100, D1X and D2H have no SubIFD JPEG. All JPEG streams start with FF D8 FF DB (no APP1 segment) and are baseline (SOF0).
- M17 (out-of-scope structures): Z 9 (S24 5148) and the survey models Z 8, Z f, Z50_2, Z5_2, Z6_3: NEFCompression 13, 0.375 bytes per pixel, no tag 0x0096. D810 (S24 2036) and D4S (S16 2227) small raw: Compression 34713, PhotometricInterpretation 32803, NEFCompression 8, 3 bytes per pixel, the 3-byte groups alternate between pairs of values near the image level and pairs near 2048. D1X compressed (S24 1785): NEFCompression 1, no tag 0x0096. D1X: W = 4028, H = 1324 for a 3:2 picture (non-square pixels). E8400 (S24 2082): the decoded raster shows the picture twice, one above the other (rows stored as two fields). E5700 (S25): CFAPattern 5 3 1 4 (cyan, magenta, yellow, green).
- M18 (stream end, S24 2802): the 1 V3 strip holds 6565 bytes after the last code of row H - 1. They repeat the code 11110 (category 0). The decoder must stop after H rows.

M14 results:

| File | Variant | Table | WB source | Matrix | Crop | Preview | Chroma | Luma |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| D700 14bit uncompressed (S13 2251) | V1 big-endian | - | 0x000C | D700 | centered | 4256x2832 | 0.067 | 0.973 |
| D3 14bit uncompressed (S14 3521) | V1 big-endian | - | 0x000C | D3 | 16, 6 | 4256x2832 | 0.012 | 0.990 |
| D4S 14bit uncompressed (S16 2224) | V1 little-endian | - | 0x000C | D4S | 4, 4 | 4928x3280 | 0.021 | 0.987 |
| D700 12bit uncompressed (S13 2256) | V2 | - | 0x000C | D700 | 16, 6 | 4256x2832 | 0.067 | 0.974 |
| D200 uncompressed (S22 824) | V2 | - | 0x000C | D200 | 16, 12 | 3872x2592 | 0.063 | 0.994 |
| D2X uncompressed (S23 9064) | V2 | - | 0x000C | D2X | 16, 10 | 4288x2848 | 0.040 | 0.988 |
| D2H uncompressed (S23 5632) | V2 | - | 0x0097 | D2H | none | 570x375 | 0.121 | 0.948 |
| D100 (S21 2996) | V3 | - | generic | generic | none | 570x375 | 0.146 | 0.775 |
| D4S 12bit uncompressed (S16 2222) | V4 | - | 0x000C | D4S | 4, 4 | 4928x3280 | 0.020 | 0.985 |
| Z 6 12bit uncompressed (S15 3583) | V4 | - | 0x000C | Z 6 | 0x0045 | 6048x4024 | 0.079 | 0.865 |
| Z 6 14bit uncompressed (S15 3580) | V5 | - | 0x000C | Z 6 | 0x0045 | 6048x4024 | 0.078 | 0.865 |
| D700 12bit lossless (S13 2254) | V6 | L12 | 0x000C | D700 | 16, 6 | 4256x2832 | 0.067 | 0.973 |
| D3 12bit lossless (S14 3518) | V6 | L12 | 0x000C | D3 | 16, 6 | 4256x2832 | 0.012 | 0.989 |
| D850 12bit lossless size S (S17 1835) | V6 | L12 | 0x000C | D850 | 0x0045 | 4128x2752 | 0.062 | 0.996 |
| D850 12bit lossless size M (S17 1836) | V6 | L12 | 0x000C | D850 | 0x0045 | 6192x4128 | 0.068 | 0.995 |
| D700 14bit lossless (S13 2249) | V6 | L14 | 0x000C | D700 | 16, 6 | 4256x2832 | 0.067 | 0.975 |
| D3 14bit lossless (S14 3520) | V6 | L14 | 0x000C | D3 | 16, 6 | 4256x2832 | 0.012 | 0.990 |
| D750 14bit lossless (S18 898) | V6 | L14 | 0x000C | D750 | 8, 8 | 6016x4016 | 0.145 | 0.977 |
| D850 14bit lossless (S17 1840) | V6 | L14 | 0x000C | D850 | 0x0045 | 8256x5504 | 0.061 | 0.996 |
| Z 6 14bit lossless (S15 3582) | V6 | L14 | 0x000C | Z 6 | 0x0045 | 6048x4024 | 0.080 | 0.866 |
| D70 lossy type 1 (S20 2060) | V7 | Y12 | 0x0097 | D70 | centered | 3008x2000 | 0.093 | 0.996 |
| D200 lossy type 1 (S22 8079) | V7 | Y12 | 0x000C | D200 | 16, 12 | 3872x2592 | 0.053 | 0.993 |
| D2X lossy type 1 (S23 9065) | V7 | Y12 | 0x000C | D2X | 16, 10 | 4288x2848 | 0.041 | 0.990 |
| D2H lossy type 1 (S23 5227) | V7 | Y12 | 0x0097 | D2H | none | 570x375 | 0.056 | 0.909 |
| D700 12bit lossy (S13 2255) | V8 0x44 0x20 | Y12 | 0x000C | D700 | 16, 6 | 4256x2832 | 0.067 | 0.973 |
| D3 12bit lossy (S14 3516) | V8 0x44 0x20 | Y12 | 0x000C | D3 | 16, 6 | 4256x2832 | 0.013 | 0.989 |
| D750 12bit lossy (S18 895) | V8 0x44 0x20 | Y12 | 0x000C | D750 | 8, 8 | 6016x4016 | 0.143 | 0.976 |
| D6 12bit lossy (S19 4726) | V8 0x44 0x40 | Y12 | 0x000C | generic | 0x0045 | 5568x3712 | 0.025 | 0.943 |
| 1 V3 12bit lossy (S24 2802) | V8 0x44 0x20 | Y12 | 0x000C | generic | 0x0045 | 5232x3488 | 0.174 | 0.993 |
| D700 14bit lossy (S13 2250) | V8 0x44 0x20 | Y14 | 0x000C | D700 | 16, 6 | 4256x2832 | 0.067 | 0.974 |
| D3 14bit lossy (S14 3519) | V8 0x44 0x20 | Y14 | 0x000C | D3 | 16, 6 | 4256x2832 | 0.013 | 0.989 |
| Z 6 14bit lossy (S15 3581) | V8 0x44 0x40 | Y14 | 0x000C | Z 6 | 0x0045 | 6048x4024 | 0.080 | 0.865 |

All decoded pictures show the scene of the preview. The higher chroma values have causes in the scene, not in the decoding: the D750 files show saturated magenta cloth, the Z 6 files a tungsten-lit table and the D2H and D100 files have no as-shot white balance in a usable form (D100) or in the main tag (D2H). For the D100 the generic white balance leaves a cyan cast against the warm camera JPEG. The low luma correlation of the D100 comes from the small 570x375 preview. The same-scene sets S13, S14 and S15 give equal values for all their variants.

### Sources encountered and not used

The author used a web search to find documentation. The search result list named pages that contain or describe raw decoder source code: a Rust crate source file "bioformats ... src/tiff/nikon.rs" on docs.rs, the libopenraw format pages (libopenraw.pages.freedesktop.org/formats/nef and wiki.freedesktop.org/libopenraw/Nikon_NEF, whose summaries cite decoder functions), the blog page "Decoding Nikon NEF Files" (leon_plickat.srht.site) and the Rust forum thread "Decoding of Nikon raw format (NEF)". The author saw only their titles and short summaries in the search result list and did not open them. Nothing from them is used.

## Scope and variants

### Storage methods

S5 lists the NEFCompression values 1 (Lossy type 1), 2 (Uncompressed), 3 (Lossless), 4 (Lossy type 2), 5 (Striped packed 12 bits), 6 (Uncompressed, reduced to 12 bit), 7 (Unpacked 12 bits), 8 (Small), 9 (Packed 12 bits), 10 (Packed 14 bits), 13 (High Efficiency) and 14 (High Efficiency*). S7 states that compressed NEF data is compressed twice: a lossy encoding with an encoding table, followed by non-adaptive Huffman compression, and that lossless compression skips the encoding table. S7 states that the encoding tables are stored in the NEF file. Uncompressed files of older models carry no NEFCompression tag (M1, S25).

### Storage variants

| Variant | Camera setting | TIFF Compression (0x0103) | NEFCompression | Sensor data | Seen in (M3, M5 to M9, S25) |
| --- | --- | --- | --- | --- | --- |
| V1 | Uncompressed 14-bit | 1 | absent | 16-bit samples in the file byte order | D3, D3S, D3X, D300, D300S, D4, D4S, D5, D500, D700, D800, D800E, D810, D850, D5100, Df |
| V2 | Uncompressed 12-bit (older models) | 1 | absent | 12-bit packed, most significant bits first | D1H, D1X, D2H, D2Hs, D2X, D2Xs, D200, D3, D3S, D300, D300S, D4, D700, D800, Df |
| V3 | Uncompressed (D100) | 34713 | 2 | 12-bit packed in 16-byte groups of 10 pixels | D100 |
| V4 | Uncompressed 12-bit (newer models) | 34713 | 6 | 12-bit packed, least significant bits first | D4S, D500, D6, D810, D850, Z 6, Z 6_2, Z 7_2, Z fc |
| V5 | Uncompressed 14-bit (newer models) | 34713 | 10 | 14-bit packed, least significant bits first | D6, Z 6, Z 6_2, Z 7_2, Z fc |
| V6 | Lossless compressed, 12-bit and 14-bit | 34713 | 3 | Huffman, tables L12 and L14, no curve | 32 surveyed models from D3, D300 and D700 to D850, D6, Z 5, Z 6, Z 7, Z 30, Z 50, Z fc, Z f |
| V7 | Lossy compressed type 1, 12-bit | 34713 | 1 | Huffman, table Y12, curve of 683 values | D2H, D2X, D2Xs, D200, D40, D40X, D60, D70, D80, D3000 (S7 lists the same table size for D50 and D70s) |
| V8 | Lossy compressed type 2, 12-bit and 14-bit | 34713 | 4 | Huffman, tables Y12 and Y14, curve of 257 knots | D3 and all later DSLRs, Nikon 1, Coolpix A, D780, D6, Z 5, Z 6, Z 6_2, Z 7_2 |

V6 includes the RAW sizes M and S of the D850 and similar models: they are ordinary CFA rasters of smaller size (M14, S17). Their CFA raster is larger than the camera output size of that setting: size S stores 6216x4136 (CropArea 6192x4128) with a 4128x2752 JPEG, size M stores 7104x4728 (CropArea 7074x4716) with a 6192x4128 JPEG. Both carry tag 0x0045, so rule 1 of R-NEF-40 applies.

### Out-of-scope relatives

| Id | Relative | Detection from file contents (M17) |
| --- | --- | --- |
| X1 | High Efficiency and High Efficiency* (Z 8, Z 9, Z f, Z50_2, Z5_2, Z6_3 and others) | NEFCompression 13 or 14 |
| X2 | Small raw of the D810, D4S and similar models | NEFCompression 8 |
| X3 | Lossy compressed files of the D1 family | NEFCompression 1 and no tag 0x0096 |
| X4 | D1X (non-square pixels) | IFD0 Model equal to "NIKON D1X" |
| X5 | Coolpix E-series NEF (the archive holds E5400, E5700, E8400, E8800) | IFD0 Model is "E" followed by four digits |
| X6 | NEF files without a CFA raw IFD (for example Coolscan scanner files) | no SubIFD qualifies as raw IFD (R-NEF-15) |
| X7 | any other combination, including NEFCompression 5, 7 and 9, which no measured NEF file uses | no rule of R-NEF-3 matches |

### Requirements

- R-NEF-1: The decoder must decode the sensor data of variants V1 to V8 as this document specifies.
- R-NEF-2: The decoder must use the preview path (section Embedded preview) for X1 to X7 files and must not interpret their sensor data.
- R-NEF-3: The decoder must classify the file by these rules, in this order, and must treat a file that matches no rule as X7:
  1. IFD0 Model (compared by R-NEF-10) equal to "NIKON D1X": X4.
  2. IFD0 Model of five characters, "E" followed by four digits 0 to 9: X5.
  3. No SubIFD qualifies as raw IFD (R-NEF-15): X6.
  4. Compression 1, BitsPerSample 12 or 14, total strip bytes >= 2 * W * H: V1.
  5. Compression 1, BitsPerSample 12, W even, total strip bytes = 3 * W * H / 2: V2.
  6. Compression 34713 and NEFCompression 13 or 14: X1.
  7. Compression 34713 and NEFCompression 8: X2.
  8. Compression 34713, NEFCompression 2, BitsPerSample 12, total strip bytes = 16 * ceil(W / 10) * H: V3.
  9. Compression 34713, NEFCompression 6, BitsPerSample 12, total strip bytes an exact multiple of H, row stride (R-NEF-27) from ceil(12 * W / 8) to ceil(12 * W / 8) + 15: V4.
  10. Compression 34713, NEFCompression 10, BitsPerSample 14, total strip bytes an exact multiple of H, row stride (R-NEF-27) from ceil(14 * W / 8) to ceil(14 * W / 8) + 15: V5.
  11. Compression 34713, linearization tag version 0x46 0x30, NEFCompression 3 or absent, BitsPerSample 12 or 14: V6.
  12. Compression 34713, linearization tag version 0x44 0x10, NEFCompression 1 or absent, BitsPerSample 12: V7.
  13. Compression 34713, linearization tag version 0x44 0x20 or 0x44 0x40, NEFCompression 4 or absent, BitsPerSample 12 or 14: V8.
  14. Compression 34713, NEFCompression 1 and no linearization tag: X3.

"Total strip bytes" is the sum of all values of StripByteCounts (0x0117). "Linearization tag version" is the first two bytes of maker-note tag 0x0096, which must be at least 12 bytes long for rules 11 to 13. NEFCompression is defined in R-NEF-20.

## Container layout

### Byte order and header

A NEF file is a TIFF file (S1 section 2). Both byte orders occur (M1).

| Offset | Size | Field | Value in NEF |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 4D 4D ("MM", big-endian) or 49 49 ("II", little-endian) |
| 2 | 2 | TIFF magic number | 42 (bytes 00 2A or 2A 00) |
| 4 | 4 | offset of IFD0 | 8 in all measured files |

- R-NEF-4: The decoder must accept a file only if bytes 0 to 3 are 4D 4D 00 2A or 49 49 2A 00, and must report "not a NEF file" otherwise.
- R-NEF-5: The decoder must read every multi-byte TIFF field of the main IFDs in the file byte order and every multi-byte field inside the maker note in the maker-note byte order.

### IFD structure

The IFD layout, the field types and their sizes are those of S1 section 2 and of arw.md (section Container layout, IFD structure). Type 13 (IFD) occurs for SubIFD offsets in some TIFF writers. The decoder treats it as LONG.

- R-NEF-6: The decoder must read a value from the value field when count * type size <= 4 and from the offset in the value field when count * type size > 4.
- R-NEF-7: The decoder must find tags by tag number in any entry order, and must use the first entry when a tag number occurs more than once in one IFD.
- R-NEF-8: The decoder must accept the types BYTE, SHORT, LONG and IFD (13) for every tag that this document reads as an integer, whatever type the tables name.
- R-NEF-9: The decoder must skip entries with a type outside 1 to 13 and must not use their values.
- R-NEF-10: The decoder must end an ASCII value at its first zero byte and must compare ASCII values after it removes all trailing spaces.

### IFD chain, SubIFDs, Exif IFD

Measured layout (M1, values from S13 2249):

| Structure | Where | Content in NEF |
| --- | --- | --- |
| IFD0 | offset 8 | Make "NIKON CORPORATION", Model, Orientation, 160x120 RGB thumbnail (strip), SubIFDs (0x014A), Exif IFD (0x8769) |
| SubIFD (preview) | an offset in 0x014A | NewSubfileType 1, Compression 6, JPEG in 0x0201 and 0x0202 |
| raw IFD | an offset in 0x014A | NewSubfileType 0, PhotometricInterpretation 32803, sensor data |
| Exif IFD | IFD0 tag 0x8769 | Exif data and the MakerNote (0x927C) |
| maker note | Exif tag 0x927C | Nikon header and a TIFF structure with the Nikon tags |

Example (S13 2249, big-endian): IFD0 at 8 with 27 entries, SubIFDs 175436 (JPEG 4256x2832 at 175808, 1739983 bytes) and 175556 (raw IFD, 17 entries), Exif IFD at 600, maker note at 1120 (116698 bytes), sensor strip at 1915808 (14874979 bytes).

- R-NEF-11: The decoder must read IFD0 at the offset in header bytes 4 to 7 and must follow the next-IFD offsets, stopping at offset 0, at an invalid IFD, at a repeated offset or after 16 IFDs.
- R-NEF-12: The decoder must treat an IFD as invalid in each of these cases: the offset lies outside the file, the entry count N is 0 or greater than 1024, or the 2 + 12 * N + 4 bytes of the IFD do not lie completely inside the file.
- R-NEF-13: The decoder must find the Exif IFD through IFD0 tag 0x8769 and the maker note through Exif tag 0x927C.

### Maker note

The maker note (M2) has this layout. Offsets count from the first byte of the maker note.

| Offset | Size | Content |
| --- | --- | --- |
| 0 | 6 | 4E 69 6B 6F 6E 00 ("Nikon" and a zero byte) |
| 6 | 2 | version: 02 10, or 02 00 (D100, D1X) |
| 8 | 2 | 00 00 |
| 10 | 8 | TIFF header: byte order, 42, offset of the first maker-note IFD (8) |
| 18 | ... | maker-note IFD and its values |

Every offset inside the maker note (the IFD offset of the header, value offsets, the PreviewIFD offset of tag 0x0011 and the offsets inside that IFD) counts from maker-note byte 10, the first byte of its TIFF header (M2).

- R-NEF-14: The decoder must accept a maker note only if it starts with 4E 69 6B 6F 6E 00 and its bytes 10 to 13 are a valid TIFF header, must read the maker-note IFD at the header offset plus the maker-note start plus 10, and must add the maker-note start plus 10 to every offset inside the maker note. Without a valid maker note, the decoder must use the defaults of section Metadata tags, NEFCompression is absent, and only rules 1 to 5 of R-NEF-3 can match.

### Raw IFD selection

- R-NEF-15: The decoder must use as the raw IFD the first SubIFD (in the order of the offsets in IFD0 tag 0x014A) that is a valid IFD, has NewSubfileType 0 or no NewSubfileType tag, has PhotometricInterpretation 32803, and has the tags 0x0100, 0x0101, 0x0103, 0x0111 and 0x0117.

## Metadata tags

"Required" means the variant cannot be decoded without the tag. The decoder then uses the preview path. "Default" is the value to use when an optional tag is absent or invalid.

### Raw IFD and IFD0 tags

| Tag | Name | Type | Count | IFD | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0x00FE | NewSubfileType | LONG | 1 | raw, SubIFDs | 0 = raw image, 1 = reduced image (S1) | optional | 0 |
| 0x0100 | ImageWidth | SHORT or LONG | 1 | raw | W | required | none |
| 0x0101 | ImageLength | SHORT or LONG | 1 | raw | H | required | none |
| 0x0102 | BitsPerSample | SHORT | 1 | raw | 12 or 14 (bps) | required | none |
| 0x0103 | Compression | SHORT | 1 | raw | 1 or 34713 ("Nikon NEF Compressed", S6) | required | none |
| 0x0106 | PhotometricInterpretation | SHORT | 1 | raw | 32803 = CFA | required | none |
| 0x010F | Make | ASCII | any | IFD0 | "NIKON CORPORATION" | optional | empty |
| 0x0110 | Model | ASCII | any | IFD0 | model, for example "NIKON D850", "NIKON Z 6_2" | optional | empty |
| 0x0111 | StripOffsets | SHORT or LONG | strips | raw | file offset of each strip | required | none |
| 0x0112 | Orientation | SHORT | 1 | IFD0 | 1 to 8, S1 definition | optional | 1 |
| 0x0115 | SamplesPerPixel | SHORT | 1 | raw | 1 | optional | 1 |
| 0x0116 | RowsPerStrip | SHORT or LONG | 1 | raw | rows in each strip (7, 10, 11, 17 or H measured) | optional | H |
| 0x0117 | StripByteCounts | SHORT or LONG | strips | raw | bytes in each strip | required | none |
| 0x014A | SubIFDs | LONG or IFD | 1 to 6 | IFD0 | offsets of SubIFDs | required | none |
| 0x0201 | JPEGInterchangeFormat (S6: JpgFromRawStart in the SubIFD of NEF) | LONG | 1 | SubIFDs, IFD chain, PreviewIFD | offset of a JPEG stream | optional | none |
| 0x0202 | JPEGInterchangeFormatLength | LONG | 1 | as 0x0201 | length of that JPEG stream | optional | none |
| 0x828D | CFARepeatPatternDim | SHORT | 2 | raw | 2, 2 | optional | 2, 2 |
| 0x828E | CFAPattern | BYTE | 4 | raw | colors of CFA positions 0 to 3 (S3): measured 0 1 1 2, 1 0 2 1, 2 1 1 0, 1 2 0 1 | optional | 0, 1, 1, 2 |
| 0x8769 | ExifIFD | LONG or IFD | 1 | IFD0 | offset of the Exif IFD | optional | none |
| 0x927C | MakerNote | UNDEFINED | any | Exif IFD | Nikon maker note | optional | none |

### Maker-note tags

Names from S5. Meanings from S5 and the records cited.

| Tag | Name (S5) | Type | Count | Meaning | Default |
| --- | --- | --- | --- | --- | --- |
| 0x000C | WB_RBLevels | RATIONAL | 4 | as-shot white balance: red multiplier, blue multiplier, then two values equal to 1 (M12) | R-NEF-38 |
| 0x0011 | PreviewIFD | LONG or IFD | 1 | offset (maker-note origin) of an IFD with a JPEG preview in 0x0201 and 0x0202 | none |
| 0x003D | BlackLevel | SHORT | 4 | black level of CFA positions 0 to 3 on a 14-bit scale (M11) | 0 |
| 0x0045 | CropArea | SHORT | 4 | left, top, width, height of the final image area in raw coordinates (S5, M15) | R-NEF-40 |
| 0x0051 | MakerNotes0x51 | UNDEFINED | 24 measured | bytes 10 and 11: NEFCompression on Z models (S5) | none |
| 0x0093 | NEFCompression | SHORT | 1 | storage method (S5) | none |
| 0x0096 | NEFLinearizationTable | UNDEFINED | 46, 624 or 1412 measured | start predictors and curve (layout below) | V6 to V8: required |
| 0x0097 | ColorBalance | UNDEFINED | any | version in bytes 0 to 3 (ASCII digits). Versions "0102" and "0103" carry plain white balance levels (M12). Most later versions are encrypted (S5) and are not used | none |

### Linearization tag 0x0096

All multi-byte numbers in maker-note byte order.

| Bytes | Content | V6 (version 0x46 0x30) | V7 (0x44 0x10) | V8 (0x44 0x20 or 0x44 0x40) |
| --- | --- | --- | --- | --- |
| 0, 1 | version | 0x46 0x30 | 0x44 0x10 | 0x44 0x20 or 0x44 0x40 |
| 2 to 9 | four 16-bit start predictors P0, P1, P2, P3 | 512 (12-bit), 2048 (14-bit) | 328 | 328 (12-bit), 1312 (14-bit) |
| 10, 11 | 16-bit count NC | 32 or 34, not a count (M10), ignored | 683 measured | 257 |
| 12 to 12 + 2 * NC - 1 | NC 16-bit curve values | not present | curve values C(0) to C(NC - 1) | curve knots K(0) to K(256) |
| following bytes | not used (34 or 98 bytes measured) | ignored | ignored | ignored |

- R-NEF-16: The decoder must use the preview path when a tag marked "required" for the classified variant is absent, has a count of 0, or is shorter than the layout above needs.
- R-NEF-17: The decoder must treat a RATIONAL with denominator 0 as invalid and use the default of its tag.
- R-NEF-18: For V1 to V8, the decoder must accept a CFA pattern only if CFARepeatPatternDim is 2, 2 (or absent) and the four CFAPattern values contain exactly one 0, two 1 and one 2, and must otherwise use the preview path.
- R-NEF-19: The decoder must use H as RowsPerStrip when tag 0x0116 is absent, is 0 or is greater than H.
- R-NEF-20: The decoder must take NEFCompression from the first value of maker-note tag 0x0093 when that tag is present and readable as an integer, otherwise from bytes 10 and 11 (maker-note byte order) of maker-note tag 0x0051 when that tag has at least 12 bytes, and must treat it as absent otherwise.

## Sensor data layout

### Raster and CFA arrangement

The raw raster has W columns and H rows. The CFA pattern repeats every 2 rows and 2 columns with its origin at stored pixel (0, 0). CFAPattern lists the colors of CFA positions 0, 1, 2, 3, which are (x even, y even), (x odd, y even), (x even, y odd), (x odd, y odd), in the row-major order of S3 (value 0 red, 1 green, 2 blue). The Bayer arrangement is that of S11. M14 confirms the four measured patterns by color agreement with the camera JPEG.

- R-NEF-21: The decoder must give pixel (x, y) the color CFAPattern[2 * (y mod 2) + (x mod 2)], with the origin of the pattern at stored pixel (0, 0) and independent of the crop origin.

### Strips and row stride

V1 and V2 files have many strips (RowsPerStrip 7 to 17) or one strip (RowsPerStrip = H, S16 2224). All other variants have one strip. Strips are contiguous in all measured files, but the decoder reads each strip at its own offset.

| Variant | Row stride R (bytes) | Pixels per row stored |
| --- | --- | --- |
| V1 | 2 * W | W |
| V2 | 3 * W / 2 | W |
| V3 | 16 * ceil(W / 10) | 10 * ceil(W / 10), the last 10 * ceil(W / 10) - W pixels are not used |
| V4 | total strip bytes / H, from ceil(12 * W / 8) to ceil(12 * W / 8) + 15 | W, the bytes after ceil(12 * W / 8) are padding |
| V5 | total strip bytes / H, from ceil(14 * W / 8) to ceil(14 * W / 8) + 15 | W, the bytes after ceil(14 * W / 8) are padding |
| V6 to V8 | no row stride: one continuous bit stream for all rows | W |

- R-NEF-22: For V1 and V2, the decoder must treat the strips as consecutive groups of RowsPerStrip rows (the last strip can hold fewer rows), each strip starting at its StripOffsets value, with the row stride R of the table.
- R-NEF-23: For V3 to V8, the decoder must require exactly one strip, or treat several strips as one byte sequence only when each strip starts where the previous one ends.

### Final image area

- R-NEF-24: The decoder must decode the whole raster (all W x H pixels) before it crops.

The crop rule is R-NEF-40 in section Levels and white balance, because it depends on preview sizes as well.

## Compression

This section specifies, for each variant, how the stored bytes become sensor values s on the scale 0 to 2^bps - 1.

### V1: 16-bit samples

Pixel (x, y) is the 16-bit unsigned value at byte offset RowStart(y) + 2 * x, where RowStart(y) is the start of row y from R-NEF-22. The byte order is the file byte order (M3: big-endian in the big-endian D700 and D3 files, little-endian in the little-endian D4S file).

- R-NEF-25: The decoder must read V1 pixel (x, y) as b0 * 256 + b1 in big-endian files and b0 + 256 * b1 in little-endian files, where b0 and b1 are the bytes at RowStart(y) + 2 * x and RowStart(y) + 2 * x + 1, and must clip values above 2^bps - 1 to 2^bps - 1.

Worked example: big-endian bytes 0x02 0xFE (S14 3521, first pixel) give 2 * 256 + 254 = 766. Little-endian bytes 0xBB 0x03 (S16 2224, first pixel) give 187 + 3 * 256 = 955.

### V2: 12-bit packed, most significant bits first

Pixels 2k and 2k + 1 of a row come from the bytes b0, b1, b2 at row offset 3k. The 24-bit big-endian group G = 65536 * b0 + 256 * b1 + b2 holds pixel 2k in bits 12 to 23 and pixel 2k + 1 in bits 0 to 11 (M3). All measured V2 files of in-scope models are big-endian files. The little-endian E8400 file (X5) uses the same packing.

- R-NEF-26: The decoder must compute pixel 2k of a V2 or V3 group as 16 * b0 + floor(b1 / 16) and pixel 2k + 1 as 256 * (b1 mod 16) + b2.

Worked example (S13 2256, first bytes of the strip): 0x0C 0x00 0xBF 0x11 0x22 0x7F. Group 1: 16 * 12 + 0 = 192 and 256 * 0 + 191 = 191. Group 2: 16 * 17 + 2 = 274 and 256 * 2 + 127 = 639.

### V3: D100 groups

Each row is cut into groups of 16 bytes. Bytes 0 to 14 of a group hold 10 pixels as five V2 groups of 3 bytes. Byte 15 is a pad byte (0 in all measured rows). Group g of a row holds the pixels 10g to 10g + 9.

Worked example (S21, first group of row 0): 18 20 55 0B 60 3F 0D D0 52 0D 00 4D 0B 30 44 00. Pixels: 386, 85, 182, 63, 221, 82, 208, 77, 179, 68. Byte 15 is 00.

### V4 and V5: packed, least significant bits first

Row y starts at StripStart + y * R (R from the stride table). Read the bytes of the row as one bit sequence in which bit 8i + j is bit j of byte i (bit 0 first). Pixel x of a row with n bits per pixel (n = 12 for V4, n = 14 for V5) is the n-bit number whose bit k is sequence bit n * x + k.

- R-NEF-27: The decoder must compute the V4 and V5 row stride R as total strip bytes / H, must require that this division is exact and that R lies in the range of rule 9 or 10 of R-NEF-3, and must ignore the bytes of each row after the first ceil(n * W / 8).
- R-NEF-28: The decoder must compute V4 and V5 pixel x of a row as the sum over k = 0 to n - 1 of 2^k times bit (n * x + k) of the row bit sequence, where bit 8i + j of the sequence is bit j of row byte i.

Worked examples:
- V4 (S16 2222, first bytes): 0xF2 0xD0 0x13. Pixel 0 = 0xF2 + 256 * (0xD0 mod 16) = 242 + 0 = 242. Pixel 1 = floor(0xD0 / 16) + 16 * 0x13 = 13 + 304 = 317.
- V5 (S15 3580, first bytes): 0xD3 0x4D 0xAC 0x83. The 32-bit little-endian number is 0x83AC4DD3. Pixel 0 = 0x83AC4DD3 mod 2^14 = 0x0DD3 = 3539. Pixel 1 = floor(0x83AC4DD3 / 2^14) mod 2^14 = 0x0EB1 = 3761.

### V6, V7, V8: Huffman-coded differences

Sources: S7 (lossy encoding followed by non-adaptive Huffman compression, encoding table in the file, lossless skips the table), S12 (prediction from the same-color pixel two columns to the left), S2 (canonical codes from code counts, difference extension). The bit order, the predictor structure, the four code tables and the curve rules are measurements (M4 to M9).

#### Bit stream

The strip is one bit stream. Bits are read from each byte most significant bit first, bytes in file order. There is no byte stuffing and there are no markers (M4). Pixels follow in raster order: row 0 from x = 0 to W - 1, then row 1, and so on. Rows start at any bit position (no alignment). After the last pixel of row H - 1 the strip can contain more bytes: padding of up to 30 bits, or in the 1 V3 file 6565 bytes (M18).

- R-NEF-29: The decoder must read the strip as one bit sequence, most significant bit of each byte first, with no byte stuffing, no markers and no alignment between rows, and must stop after the last pixel of row H - 1 without checking the remaining bytes.

#### Code tables

Each code table is given by the numbers of codes of each length (BITS, lengths 1 to 16) and the list of categories in code order (HUFFVAL), as in S2 Annex C. The codes follow from BITS by the procedure of S2 Annex C (figures C.1 and C.2): the first code of the shortest length is all zeros, each next code of the same length is the previous code plus 1, and at each step to a longer length the code is the previous code plus 1, shifted left by one bit. The tables below list the resulting codes too.

Table L12 (V6, bps = 12). BITS = 0, 1, 4, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0. HUFFVAL = 5, 4, 6, 3, 7, 2, 8, 1, 9, 0, 10, 11, 12.

| Code | Category | Code | Category | Code | Category |
| --- | --- | --- | --- | --- | --- |
| 00 | 5 | 1100 | 2 | 11110 | 0 |
| 010 | 4 | 1101 | 8 | 111110 | 10 |
| 011 | 6 | 11100 | 1 | 1111110 | 11 |
| 100 | 3 | 11101 | 9 | 1111111 | 12 |
| 101 | 7 | | | | |

Table L14 (V6, bps = 14). BITS = 0, 1, 4, 2, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0. HUFFVAL = 7, 6, 8, 5, 9, 4, 10, 3, 11, 12, 2, 0, 1, 13, 14.

| Code | Category | Code | Category | Code | Category |
| --- | --- | --- | --- | --- | --- |
| 00 | 7 | 1100 | 4 | 111101 | 2 |
| 010 | 6 | 1101 | 10 | 111110 | 0 |
| 011 | 8 | 11100 | 3 | 1111110 | 1 |
| 100 | 5 | 11101 | 11 | 11111110 | 13 |
| 101 | 9 | 111100 | 12 | 11111111 | 14 |

Table Y12 (V7, and V8 with bps = 12). Established codes (M7): BITS = 0, 1, 5, 1, 1, 1, 1, 1, 2 (lengths 1 to 9, then 0). HUFFVAL in code order = 5, 4, 3, 6, 2, 7, 1, 0, 8, 9, (not established), 10, (not established).

| Code | Category | Code | Category | Code | Category |
| --- | --- | --- | --- | --- | --- |
| 00 | 5 | 101 | 2 | 1111110 | 9 |
| 010 | 4 | 110 | 7 | 11111110 | not established |
| 011 | 3 | 1110 | 1 | 111111110 | 10 |
| 100 | 6 | 11110 | 0 | 111111111 | not established |
| | | 111110 | 8 | | |

Table Y14 (V8 with bps = 14). Established codes (M8): BITS = 0, 1, 4, 3, 1, 1, 1, 1, 1 (lengths 1 to 9, then the subtree 111111111 is not established). HUFFVAL in code order = 5, 6, 4, 7, 8, 3, 9, 2, 1, 0, 10, 11, 12. The established codes use 511/512 of the code space.

| Code | Category | Code | Category | Code | Category |
| --- | --- | --- | --- | --- | --- |
| 00 | 5 | 1100 | 3 | 1111110 | 10 |
| 010 | 6 | 1101 | 9 | 11111110 | 11 |
| 011 | 4 | 1110 | 2 | 111111110 | 12 |
| 100 | 7 | 11110 | 1 | 111111111... | not established |
| 101 | 8 | 111110 | 0 | | |

- R-NEF-30: The decoder must use table L12 for V6 with bps 12, table L14 for V6 with bps 14, table Y12 for V7 and for V8 with bps 12, and table Y14 for V8 with bps 14, with exactly the codes listed above.
- R-NEF-31: The decoder must fail (preview path) when it reads a code marked "not established" or a bit sequence that no listed code matches within 16 bits.

#### Differences and prediction

For each pixel the decoder reads one code and obtains the category s (0 to 14). It then reads the next s bits as an unsigned number E (most significant bit first). The difference is (S2 F.2.2.1, procedure EXTEND):
- d = 0 when s = 0,
- d = E when s >= 1 and E >= 2^(s - 1),
- d = E - 2^s + 1 when s >= 1 and E < 2^(s - 1).

The code value of pixel (x, y) is the prediction plus d (M4, S12):
- For x >= 2: the prediction is the code value of pixel (x - 2, y).
- For x = 0 or x = 1 and y >= 2: the prediction is the code value of pixel (x, y - 2).
- For x = 0 or x = 1 and y = 0 or y = 1: the prediction is the start predictor P(2 * y + x) from bytes 2 to 9 of tag 0x0096 (P0 at bytes 2 and 3, P1 at 4 and 5, P2 at 6 and 7, P3 at 8 and 9).

- R-NEF-32: The decoder must compute each difference with the three rules above and each code value as prediction plus difference, with the prediction rules above, using the code values (not the curve values) of earlier pixels.
- R-NEF-33: The decoder must fail when a V6 code value is less than 0 or greater than 2^bps - 1, or when a V7 or V8 code value is less than 0 or not less than the curve length L of R-NEF-34 or R-NEF-35.

For V6 the code value is the sensor value s. For V7 and V8 the sensor value is s = C(code value), with the curve C of the next subsection.

#### Curves (V7 and V8)

V7 (version 0x44 0x10) stores the curve directly: NC values C(0) to C(NC - 1) at bytes 12 onward of tag 0x0096. All measured V7 files have NC = 683 (M9, S7 lists 683 for D70, D200 and related models).

V8 (versions 0x44 0x20 and 0x44 0x40) stores 257 knots K(0) to K(256). The step S is 2^bps / 256 for version 0x44 0x20 and 2^bps / 1024 for version 0x44 0x40, so S is 16 or 64 (0x44 0x20, 12 or 14 bits) and 4 or 16 (0x44 0x40, 12 or 14 bits). The curve has L = 256 * S + 1 entries:
- C(i) = K(j) + floor((K(j + 1) - K(j)) * f / S) for 0 <= i < 256 * S, where j = floor(i / S) and f = i mod S,
- C(256 * S) = K(256).

This rule reproduces the six published tables of S8 exactly (M9). S7 calls the curve the encoding table and states that its first missing value is where the encoding truly begins: below it the curve is the identity.

- R-NEF-34: For V7, the decoder must require 2 <= NC <= 2^bps, tag 0x0096 length >= 12 + 2 * NC, C non-decreasing and every C value <= 2^bps - 1, must use L = NC, and must otherwise use the preview path.
- R-NEF-35: For V8, the decoder must require NC = 257, tag 0x0096 length >= 526, K non-decreasing and every K value <= 2^bps - 1, must build C with the rule above (L = 256 * S + 1), and must otherwise use the preview path.

Worked example (S13 2255, D700 12-bit, version 0x44 0x20, S = 16): knots K(14) to K(19) are 224, 241, 263, 287, 312, 397. C(240) = K(15) = 241. C(241) = 241 + floor(22 * 1 / 16) = 242. C(247) = 241 + floor(22 * 7 / 16) = 250. C(255) = 241 + floor(22 * 15 / 16) = 261. C(256) = 263. C(300): j = 18, f = 12, 312 + floor(85 * 12 / 16) = 375. K(43) = 4095, so C(688) = 4095, and 689 entries C(0) to C(688) are the 689-value table of S8.

#### Worked example (vector TV-5 of section Test vectors)

First bytes of the strip of S13 2254 (D700 12-bit lossless, table L12, P0 to P3 = 512): EA FF AB EB 33 B9 F7 7B 80 B3 A8.

| Pixel | Bits | Code | s | Extra bits E | d | Prediction | Value |
| --- | --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 0 to 13 | 11101 | 9 | 010111111 = 191 | 191 - 511 = -320 | P0 = 512 | 192 |
| (1, 0) | 14 to 27 | 11101 | 9 | 010111110 = 190 | -321 | P1 = 512 | 191 |
| (2, 0) | 28 to 37 | 101 | 7 | 1001100 = 76 | 76 | 192 | 268 |
| (3, 0) | 38 to 51 | 11101 | 9 | 110011111 = 415 | 415 | 191 | 606 |
| (4, 0) | 52 to 60 | 011 | 6 | 101111 = 47 | 47 | 268 | 315 |
| (5, 0) | 61 to 69 | 011 | 6 | 100000 = 32 | 32 | 606 | 638 |
| (6, 0) | 70 to 76 | 00 | 5 | 10110 = 22 | 22 | 315 | 337 |
| (7, 0) | 77 to 85 | 011 | 6 | 101010 = 42 | 42 | 638 | 680 |

## Levels and white balance

### White level and black level

- R-NEF-36: The decoder must use the white level w = 2^bps - 1 (4095 or 16383) for all variants.
- R-NEF-37: The decoder must use as black level of CFA position i the real number BlackLevel[i] / 2^(14 - bps) of maker-note tag 0x003D (not rounded) when the tag has 4 values and each scaled value is less than w - 255, and 0 for every position otherwise.

The scale 2^(14 - bps) follows from M11: the tag is on a 14-bit scale also in 12-bit files. Files without the tag have their black level subtracted in the camera (M11). Example: BlackLevel 600 in a 12-bit file gives 600 / 4 = 150.

Values above the white level do not occur in decoded data, because every variant is limited to 2^bps - 1 (R-NEF-25, R-NEF-33, curve maximum).

### White balance multipliers

The decoder computes three multipliers mR, mG, mB. Sources in priority order:
1. Maker-note tag 0x000C WB_RBLevels with at least 2 rationals whose first two denominators are not 0: mR = first rational, mB = second rational, mG = 1, when both are from 0.25 to 8.
2. Maker-note tag 0x0097 with version "0102" (bytes 0 to 3 are 0x30 0x31 0x30 0x32) and at least 18 bytes: the 16-bit numbers at bytes 10, 12, 14, 16 are R, G1, G2, B. With G = (G1 + G2) / 2: mR = R / G, mG = 1, mB = B / G.
3. Maker-note tag 0x0097 with version "0103" and at least 28 bytes: the 16-bit numbers at bytes 20, 22, 24, 26 are R, G1, B, G2, and the multipliers follow as in source 2.
4. The D50 white balance scales of the Model in the table of section Color conversion (S9): mR and mB from the table, mG = 1.
5. The generic scales mR = 2.099, mG = 1, mB = 1.443 (mean of the table).

For sources 2 and 3 the 16-bit numbers are in maker-note byte order (big-endian in the measured D2H and D70 files), and the decoder requires G > 0 and each multiplier from 0.25 to 8.

- R-NEF-38: The decoder must take the WB multipliers from the first valid source of the list above.

Worked example: tag 0x000C of S13 2254 holds 461/256, 338/256, 256/256, 256/256: mR = 1.80078125, mB = 1.3203125. Tag 0x0097 of S20 2060 (version "0103") holds 555, 256, 388, 256 at bytes 20 to 27: G = 256, mR = 2.16796875, mB = 1.515625.

### Normalization

For a pixel with sensor value s, black level b of its CFA position, white level w and multiplier m of its color:

- R-NEF-39: The decoder must compute v = clip((s - b) / (w - b) * m, 0, 1) for every pixel before demosaicing.

Worked example (fixture F1, TV-12): s = 2912 (red), 4912 (green), 1912 (blue), b = 400, w = 16383, mR = 3892/2048 = 1.900390625, mB = 2817/2048 = 1.37548828125:
- R: (2912 - 400) / 15983 * 1.900390625 = 0.1571670 * 1.9003906 = 0.2986787.
- G: (4912 - 400) / 15983 = 0.2822999.
- B: (1912 - 400) / 15983 * 1.3754883 = 0.0946005 * 1.3754883 = 0.1301219.

### Final image area (crop)

- R-NEF-40: The decoder must crop the demosaiced image to:
  1. the rectangle (left, top, width, height) of maker-note tag 0x0045 when the tag has 4 values, width >= 16, height >= 16, left + width <= W and top + height <= H;
  2. otherwise, when the largest JPEG candidate of R-NEF-52 with a parsable SOF segment (R-NEF-53, decodability not required) has width PW and height PH with 0.9 * W <= PW <= W and 0.9 * H <= PH <= H, the rectangle with origin (floor((W - PW) / 2), floor((H - PH) / 2)) and size PW x PH;
  3. otherwise the whole raster.

Rule 2 follows M15. Example: D700, 4288x2844 raster, preview 4256x2832: origin (16, 6), size 4256x2832.

## Color conversion

### Matrix sources

NEF files carry no color matrix in a documented tag (S5 lists none). As in arw.md, the decoder uses a per-model table from S9 (DXOMARK), which publishes for each tested camera a "color matrix as defined in ISO standard 17321" for CIE D50 and CIE A with white balance scales. The matrix converts white-balanced camera RGB to linear sRGB, rows are output channels and columns are camera channels (M13 confirms this orientation for NEF). The table gives the D50 block of each model, read on 2026-11-14: on every listed page it is the first block and the block with the larger ratio of the R scale to the B scale. The three spot checks of the author (D700, D850, Z 6) agree with the table.

| Model (IFD0 0x0110) | S9 name | WB R | WB B | m11 | m12 | m13 | m21 | m22 | m23 | m31 | m32 | m33 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| NIKON D2H | D2H | 2.62 | 1.27 | 1.93 | -0.73 | -0.20 | -0.13 | 1.48 | -0.35 | 0.06 | -0.51 | 1.45 |
| NIKON D2X | D2X | 1.85 | 1.56 | 1.62 | -0.64 | 0.02 | -0.09 | 1.36 | -0.27 | 0.02 | -0.26 | 1.24 |
| NIKON D70 | D70 | 2.16 | 1.64 | 1.66 | -0.62 | -0.04 | -0.10 | 1.44 | -0.34 | 0.03 | -0.36 | 1.33 |
| NIKON D70s | D70s | 2.16 | 1.64 | 1.66 | -0.62 | -0.04 | -0.10 | 1.44 | -0.34 | 0.03 | -0.36 | 1.33 |
| NIKON D50 | D50 | 2.08 | 1.50 | 1.61 | -0.56 | -0.05 | -0.10 | 1.46 | -0.36 | 0.03 | -0.37 | 1.34 |
| NIKON D200 | D200 | 2.22 | 1.32 | 1.77 | -0.67 | -0.10 | -0.13 | 1.43 | -0.30 | 0.00 | -0.26 | 1.26 |
| NIKON D80 | D80 | 2.20 | 1.19 | 1.80 | -0.70 | -0.10 | -0.10 | 1.37 | -0.27 | 0.01 | -0.25 | 1.24 |
| NIKON D40 | D40 | 2.11 | 1.52 | 1.64 | -0.61 | -0.02 | -0.11 | 1.45 | -0.35 | 0.03 | -0.34 | 1.32 |
| NIKON D40X | D40X | 1.80 | 1.39 | 1.56 | -0.49 | -0.07 | -0.12 | 1.39 | -0.27 | 0.03 | -0.33 | 1.30 |
| NIKON D60 | D60 | 1.79 | 1.38 | 1.52 | -0.47 | -0.05 | -0.11 | 1.37 | -0.27 | 0.02 | -0.31 | 1.29 |
| NIKON D90 | D90 | 2.10 | 1.21 | 1.98 | -0.87 | -0.11 | -0.13 | 1.48 | -0.35 | 0.03 | -0.43 | 1.40 |
| NIKON D300 | D300 | 1.56 | 1.28 | 1.73 | -0.61 | -0.12 | -0.14 | 1.49 | -0.36 | 0.06 | -0.54 | 1.48 |
| NIKON D300S | D300S | 1.86 | 1.18 | 1.85 | -0.72 | -0.13 | -0.13 | 1.45 | -0.32 | 0.04 | -0.42 | 1.38 |
| NIKON D3 | D3 | 2.24 | 1.20 | 1.79 | -0.61 | -0.19 | -0.13 | 1.42 | -0.29 | -0.01 | -0.25 | 1.26 |
| NIKON D3S | D3S | 2.18 | 1.23 | 1.90 | -0.80 | -0.10 | -0.13 | 1.46 | -0.33 | 0.04 | -0.40 | 1.36 |
| NIKON D3X | D3X | 2.66 | 1.25 | 1.77 | -0.69 | -0.08 | -0.14 | 1.57 | -0.43 | 0.02 | -0.38 | 1.36 |
| NIKON D700 | D700 | 1.84 | 1.41 | 1.65 | -0.52 | -0.13 | -0.12 | 1.43 | -0.31 | 0.03 | -0.38 | 1.35 |
| NIKON D3000 | D3000 | 2.24 | 1.24 | 1.73 | -0.69 | -0.04 | -0.11 | 1.38 | -0.27 | 0.00 | -0.24 | 1.24 |
| NIKON D3100 | D3100 | 2.53 | 1.14 | 1.86 | -0.78 | -0.08 | -0.15 | 1.52 | -0.37 | 0.02 | -0.37 | 1.35 |
| NIKON D3200 | D3200 | 2.10 | 1.36 | 2.02 | -1.01 | -0.01 | -0.16 | 1.54 | -0.38 | 0.03 | -0.56 | 1.54 |
| NIKON D3300 | D3300 | 2.12 | 1.47 | 1.85 | -0.74 | -0.10 | -0.17 | 1.56 | -0.39 | 0.05 | -0.47 | 1.42 |
| NIKON D3400 | D3400 | 2.37 | 1.75 | 1.52 | -0.46 | -0.06 | -0.09 | 1.35 | -0.26 | 0.02 | -0.33 | 1.32 |
| NIKON D5000 | D5000 | 2.18 | 1.15 | 1.93 | -0.86 | -0.07 | -0.18 | 1.52 | -0.34 | -0.01 | -0.43 | 1.44 |
| NIKON D5100 | D5100 | 2.09 | 1.52 | 1.87 | -0.80 | -0.07 | -0.16 | 1.53 | -0.37 | 0.06 | -0.50 | 1.44 |
| NIKON D5200 | D5200 | 2.51 | 1.74 | 1.50 | -0.45 | -0.05 | -0.07 | 1.33 | -0.26 | 0.04 | -0.39 | 1.35 |
| NIKON D5300 | D5300 | 2.12 | 1.49 | 1.84 | -0.73 | -0.11 | -0.16 | 1.56 | -0.40 | 0.05 | -0.49 | 1.44 |
| NIKON D5500 | D5500 | 2.00 | 1.51 | 1.87 | -0.83 | -0.04 | -0.18 | 1.57 | -0.39 | 0.09 | -0.56 | 1.47 |
| NIKON D5600 | D5600 | 2.05 | 1.47 | 1.88 | -0.79 | -0.09 | -0.19 | 1.60 | -0.41 | 0.04 | -0.46 | 1.43 |
| NIKON D7000 | D7000 | 2.11 | 1.50 | 1.87 | -0.81 | -0.06 | -0.16 | 1.55 | -0.39 | 0.05 | -0.47 | 1.42 |
| NIKON D7100 | D7100 | 2.50 | 1.76 | 1.49 | -0.44 | -0.05 | -0.07 | 1.30 | -0.23 | 0.04 | -0.37 | 1.33 |
| NIKON D7200 | D7200 | 2.47 | 1.69 | 1.54 | -0.52 | -0.03 | -0.08 | 1.35 | -0.27 | 0.04 | -0.39 | 1.35 |
| NIKON D7500 | D7500 | 2.29 | 1.63 | 1.74 | -0.74 | 0.00 | -0.15 | 1.46 | -0.32 | 0.07 | -0.59 | 1.52 |
| NIKON D600 | D600 | 1.95 | 1.49 | 1.89 | -0.80 | -0.09 | -0.15 | 1.52 | -0.37 | 0.07 | -0.50 | 1.43 |
| NIKON D610 | D610 | 1.94 | 1.49 | 1.87 | -0.78 | -0.09 | -0.16 | 1.54 | -0.38 | 0.07 | -0.50 | 1.43 |
| NIKON D800 | D800 | 2.04 | 1.48 | 1.94 | -0.86 | -0.08 | -0.16 | 1.54 | -0.38 | 0.06 | -0.49 | 1.43 |
| NIKON D800E | D800E | 2.02 | 1.49 | 1.89 | -0.82 | -0.08 | -0.16 | 1.55 | -0.39 | 0.07 | -0.51 | 1.44 |
| NIKON D810 | D810 | 2.00 | 1.43 | 1.88 | -0.77 | -0.11 | -0.15 | 1.53 | -0.38 | 0.05 | -0.48 | 1.43 |
| NIKON D850 | D850 | 1.93 | 1.49 | 1.64 | -0.58 | -0.06 | -0.10 | 1.36 | -0.26 | 0.04 | -0.48 | 1.44 |
| NIKON D750 | D750 | 1.98 | 1.44 | 1.92 | -0.79 | -0.13 | -0.17 | 1.58 | -0.41 | 0.07 | -0.55 | 1.48 |
| NIKON D4 | D4 | 2.14 | 1.59 | 1.69 | -0.60 | -0.08 | -0.09 | 1.34 | -0.25 | 0.05 | -0.42 | 1.37 |
| NIKON D4S | D4S | 2.12 | 1.53 | 1.71 | -0.58 | -0.12 | -0.11 | 1.40 | -0.29 | 0.04 | -0.42 | 1.37 |
| NIKON D5 | D5 | 2.28 | 1.63 | 1.61 | -0.52 | -0.09 | -0.09 | 1.39 | -0.30 | -0.01 | -0.42 | 1.43 |
| NIKON D500 | D500 | 2.32 | 1.63 | 1.70 | -0.68 | -0.02 | -0.14 | 1.47 | -0.33 | 0.06 | -0.53 | 1.46 |
| NIKON Df | Df | 2.14 | 1.56 | 1.65 | -0.55 | -0.10 | -0.09 | 1.34 | -0.25 | 0.05 | -0.40 | 1.35 |
| NIKON Z 6 | Z6 | 1.75 | 1.34 | 1.92 | -0.82 | -0.10 | -0.15 | 1.48 | -0.33 | 0.05 | -0.55 | 1.50 |
| NIKON Z 7 | Z7 | 1.76 | 1.38 | 1.78 | -0.70 | -0.08 | -0.12 | 1.42 | -0.29 | 0.04 | -0.49 | 1.44 |
| NIKON Z 6_2 | Z6II | 1.73 | 1.33 | 1.94 | -0.84 | -0.10 | -0.15 | 1.50 | -0.35 | 0.04 | -0.52 | 1.48 |
| NIKON Z 7_2 | Z7II | 1.76 | 1.37 | 1.81 | -0.72 | -0.09 | -0.14 | 1.44 | -0.31 | 0.03 | -0.46 | 1.43 |
| NIKON Z 9 | Z9 | 1.86 | 1.44 | 1.74 | -0.64 | -0.10 | -0.13 | 1.43 | -0.30 | 0.02 | -0.43 | 1.41 |

The model names in the first column are IFD0 Model strings measured in the files of S13 to S25. For the D5 page S9 prints only one block. The pages for D1X, D100, D780, D6, Z 5, Z 50 and Z fc returned no page, and the D3500 page has no color data.

Generic matrix for models that are not in the table: the element-wise mean of the 49 matrices above, rounded to 3 decimals:

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 1.766 | -0.686 | -0.079 |
| sRGB G | -0.130 | 1.458 | -0.329 |
| sRGB B | 0.038 | -0.427 | 1.389 |

### Procedure

- R-NEF-41: The decoder must select the table row whose Model column equals the IFD0 Model string (compared by R-NEF-10), and must use the generic matrix when no row matches.
- R-NEF-42: The decoder must divide each row of the selected matrix by the sum of that row before use, so that every row sums to 1, and must use the generic matrix when a row sum of the selected matrix is not greater than 0.
- R-NEF-43: The decoder must compute the output (r, g, b) from the demosaiced, white-balanced, normalized camera values (vR, vG, vB) as r = m11 * vR + m12 * vG + m13 * vB, g = m21 * vR + m22 * vG + m23 * vB, b = m31 * vR + m32 * vG + m33 * vB with the normalized matrix, and must clip r, g and b to the range 0 to 1.
- R-NEF-44: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S4), without the sRGB transfer function.

Worked example (TV-12): camera values (0.2986787, 0.2822999, 0.1301219) and the NIKON D850 matrix, whose rows sum to 1.00, so normalization changes nothing:
- r = 1.64 * 0.2986787 - 0.58 * 0.2822999 - 0.06 * 0.1301219 = 0.4898330 - 0.1637340 - 0.0078073 = 0.3182917.
- g = -0.10 * 0.2986787 + 1.36 * 0.2822999 - 0.26 * 0.1301219 = -0.0298679 + 0.3839279 - 0.0338317 = 0.3202284.
- b = 0.04 * 0.2986787 - 0.48 * 0.2822999 + 1.44 * 0.1301219 = 0.0119471 - 0.1355040 + 0.1873755 = 0.0638187.

The products are computed with full precision and rounded to 7 decimals for display. Sums of the rounded products can differ by 1 in the last decimal.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and read the header (R-NEF-58, R-NEF-4, R-NEF-5).
2. Read IFD0, the IFD chain, the Exif IFD and the maker note. Validate every IFD (R-NEF-11, R-NEF-12, R-NEF-13, R-NEF-14, R-NEF-6 to R-NEF-10).
3. Collect the preview candidates now, so that every later failure can fall back (R-NEF-52 to R-NEF-55).
4. Find the raw IFD (R-NEF-15) and NEFCompression (R-NEF-20).
5. Classify the file (R-NEF-3). For X1 to X7 use the preview path (R-NEF-2). Check the required tags (R-NEF-16, R-NEF-17).
6. Determine the CFA pattern (R-NEF-18, R-NEF-21), the white and black levels (R-NEF-36, R-NEF-37), the WB multipliers (R-NEF-38), the strip layout (R-NEF-19, R-NEF-22, R-NEF-23) and, for V7 and V8, the curve (R-NEF-34, R-NEF-35).
7. Check all sizes against the file and the limits before allocating memory (section Error handling).
8. Decode the sensor data:
   - V1: R-NEF-22, R-NEF-25.
   - V2: R-NEF-22, R-NEF-26.
   - V3: R-NEF-23, R-NEF-26 and the group layout of section Compression (R-NEF-45).
   - V4, V5: R-NEF-23, R-NEF-27, R-NEF-28.
   - V6, V7, V8: R-NEF-23, R-NEF-29 to R-NEF-33, and for V7 and V8 the curve lookup (R-NEF-46).
9. Normalize (R-NEF-39).
10. Demosaic the whole raster (R-NEF-24, R-NEF-47, R-NEF-48).
11. Convert to linear sRGB (R-NEF-41 to R-NEF-43).
12. Crop to the final image area (R-NEF-40, R-NEF-49).
13. Apply the orientation (R-NEF-50, R-NEF-51).
14. Deliver the image (R-NEF-44). If any step from 4 to 13 fails, deliver the preview instead (R-NEF-56).

### Requirements for steps 8 to 13

- R-NEF-45: The decoder must read V3 row y at StripStart + y * 16 * ceil(W / 10), must take pixels 10g to 10g + 9 from bytes 16g to 16g + 14 of the row with R-NEF-26, must ignore byte 16g + 15, and must ignore stored pixels with x >= W.
- R-NEF-46: For V7 and V8, the decoder must replace each code value c by the sensor value C(c) after all code values of the raster are decoded or while decoding, but must use code values, not sensor values, for the prediction (R-NEF-32).
- R-NEF-47: The decoder must demosaic by the bilinear rule of arw.md (R-ARW-54 and R-ARW-55, source S10) applied to the whole raster as valid area: each pixel keeps its own color, and each missing color is the mean of the normalized values of that color among its 8 neighbors inside the raster, or 0 when there is none.
- R-NEF-48: The decoder must demosaic with the CFA phase of the stored raster and crop afterwards, so that the result equals demosaicing the whole raster and then cropping.
- R-NEF-49: The decoder must apply the crop of R-NEF-40 after the color conversion and before the orientation.
- R-NEF-50: The decoder must read Orientation from IFD0 tag 0x0112, use 1 when the tag is absent or outside 1 to 8, and apply it after the crop.
- R-NEF-51: The decoder must map pixels for the eight orientation values exactly as the table of arw.md R-ARW-58 (S1 definitions): values 1 to 4 keep the size W' x H', values 5 to 8 swap it, value 6 sends stored row 0 to the rightmost output column and value 8 sends it to the leftmost.

The raw IFD of some old models (D70) also has an Orientation tag. The decoder uses the IFD0 tag only.

## Embedded preview

### Locations

| Location | Tags | Content (M1, M16) |
| --- | --- | --- |
| IFD0 | 0x0111 strip, Compression 1, PhotometricInterpretation 2, BitsPerSample 8 8 8 | 160x120 RGB thumbnail, 57600 bytes (Z 9 and later: 160x120 JPEG in 0x0201, 0x0202) |
| SubIFDs with NewSubfileType 1 and Compression 6 | 0x0201, 0x0202 | JPEG, full size or nearly full size (for example 4256x2832 of a 4288x2844 raster), and on newer models a second JPEG of 1620x1080 |
| maker-note tag 0x0011 PreviewIFD | 0x0201, 0x0202 inside that IFD, offsets from maker-note byte 10 | JPEG of 570x375 (564x372 on the D70) or 640x424 |

All preview streams are baseline JPEG (SOF0, 8-bit, 3 components). They start with FF D8 FF DB, contain no APP1 (Exif) segment, and carry no orientation of their own (M16). D100, D1X and D2H files have only the maker-note JPEG and the IFD0 thumbnail.

### Requirements

- R-NEF-52: The decoder must collect as JPEG preview candidates the pairs (0x0201, 0x0202) of every IFD of the chain, of every SubIFD and of the maker-note PreviewIFD (offsets converted by R-NEF-14), and must keep a candidate only if offset + length lies inside the file, length >= 4 and the first two bytes are FF D8.
- R-NEF-53: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment and must decode the candidates with the baseline or progressive process of S2.
- R-NEF-54: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan.
- R-NEF-55: The decoder must choose the candidate with the largest width * height, and must try the next-largest candidate when one fails to decode.
- R-NEF-56: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 to X7, and when no JPEG candidate decodes, must deliver the IFD0 thumbnail if IFD0 has Compression 1, PhotometricInterpretation 2, SamplesPerPixel 3, BitsPerSample 8 8 8 and a strip of exactly 3 * width * height bytes (pixels in row order, red, green, blue), and must report an error only when neither exists.
- R-NEF-57: The decoder must apply the IFD0 Orientation (R-NEF-51) to the preview, except that it must not apply values 5 to 8 when the preview is taller than wide, and must report with every delivered image whether it is the decoded raw image (linear sRGB, R-NEF-44) or a preview (sRGB-encoded values, not linear).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-NEF-56).

### Limits

- R-NEF-58: The decoder must fail for a file shorter than 8 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-NEF-59: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728).
- R-NEF-60: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values.
- R-NEF-61: The decoder must check, before reading, that every value, strip, maker-note and preview byte range lies completely inside the file, where a maker-note offset counts as the maker-note origin plus the stored offset.

### Structure checks

- R-NEF-62: The decoder must fail for V1 and V2 in each of these cases: StripOffsets and StripByteCounts have different counts, the strip count is not ceil(H / RowsPerStrip), or a strip holds fewer bytes than the row stride times its number of rows.
- R-NEF-63: The decoder must never read beyond the strip in V3, V4 and V5 (rules 8 to 10 of R-NEF-3 already fix the total strip bytes).
- R-NEF-64: The decoder must fail for V6, V7 and V8 when a code or its extra bits consume bits beyond the end of the strip. Only consumed bits count: a reader that looks ahead must supply zero bits past the strip end without failing, and unread bits after the last pixel must not cause a failure (R-NEF-29). The strip of S13 2255 ends exactly at its last consumed bit.
- R-NEF-65: The decoder must fail when W is less than 2 or H is less than 2 for V6, V7 and V8, because the start predictors need two columns and two rows.
- R-NEF-66: The decoder must keep every intermediate code value in a signed integer of at least 32 bits, so that a sequence of large differences cannot overflow before R-NEF-33 detects it.
- R-NEF-67: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check selects the default of the quantity.

### Behaviour on error

- R-NEF-68: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-NEF-69: The decoder must discard a partly decoded raw image on failure and must not deliver it.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order. Strip bytes come from the files named.

### TV-1: headers

- 0x4D 0x4D 0x00 0x2A 0x00 0x00 0x00 0x08: big-endian TIFF, IFD0 at 8 (R-NEF-4).
- 0x49 0x49 0x2A 0x00 0x08 0x00 0x00 0x00: little-endian TIFF, IFD0 at 8.
- Maker note of S13 2249 at file offset 1120: 4E 69 6B 6F 6E 00 02 10 00 00 4D 4D 00 2A 00 00 00 08. The maker-note origin is 1120 + 10 = 1130. The first maker-note IFD is at 1130 + 8 = 1138.

### TV-2: linearization tag 0x0096

- S13 2254 (big-endian, V6), all 46 bytes: 46 30 02 00 02 00 02 00 02 00 00 20 B7 28 AA 9D E0 72 5E 90 1A F6 1E 99 65 82 F0 AF BF 20 D2 D0 2D C8 C7 0C A1 84 C7 58 2D 50 D3 AB 00 00. Version 0x46 0x30, P0 to P3 = 512, bytes 10 and 11 = 0x0020 = 32 (ignored).
- S17 1840 (little-endian, V6), first 12 bytes: 46 30 00 08 00 08 00 08 00 08 22 00. P0 to P3 = 0x0800 = 2048.
- S13 2255 (V8), first 32 bytes: 44 20 01 48 01 48 01 48 01 48 01 01 00 00 00 10 00 20 00 30 00 40 00 50 00 60 00 70 00 80 00 90. Version 0x44 0x20, P0 to P3 = 328, NC = 257, K(0) to K(9) = 0, 16, 32, ..., 144.
- S15 3581 (little-endian V8), first 20 bytes: 44 40 20 05 20 05 20 05 20 05 01 01 00 00 10 00 20 00 30 00. Version 0x44 0x40, P0 to P3 = 0x0520 = 1312, NC = 257, K(0) to K(3) = 0, 16, 32, 48, step S = 16384 / 1024 = 16.
- S20 2060 (V7), first 16 bytes: 44 10 01 48 01 48 01 48 01 48 02 AB 00 00 00 01. Version 0x44 0x10, P0 to P3 = 328, NC = 0x02AB = 683, C(0) = 0, C(1) = 1.

### TV-3: uncompressed samples

- V1 big-endian (S14 3521, first strip bytes): 02 FE 03 00 05 9E 09 13 -> 766, 768, 1438, 2323.
- V1 little-endian (S16 2224): BB 03 E0 04 B8 03 C4 04 -> 955, 1248, 952, 1220.
- V2 (S13 2256): 0C 00 BF 11 22 7F 15 12 B9 16 92 D3 -> 192, 191, 274, 639, 337, 697, 361, 723.
- V2 (S22 824, D200): 06 40 72 0F 30 76 -> 100, 114, 243, 118.
- V3 (S21): 18 20 55 0B 60 3F 0D D0 52 0D 00 4D 0B 30 44 00 -> 386, 85, 182, 63, 221, 82, 208, 77, 179, 68 (byte 15 ignored).
- V4 (S16 2222): F2 D0 13 F4 50 14 F5 30 14 F8 40 14 -> 242, 317, 244, 325, 245, 323, 248, 324.
- V5 (S15 3580): D3 4D AC 83 E3 00 3D 67 8E C3 83 E3 04 3A -> 3539, 3761, 3640, 3904, 3687, 3854, 3640, 3713.

### TV-4: code generation

From BITS of table L12 by S2 Annex C: length 2 has one code 00. Length 3 starts at (00 + 1) shifted left = 010 and has four codes 010, 011, 100, 101. Length 4 starts at (101 + 1) shifted left = 1100 and has two codes 1100, 1101. Length 5: 11100, 11101, 11110. Length 6: 111110. Length 7: 1111110, 1111111. The sum of 2^(-length) over the 13 codes is 1 (complete code).

### TV-5: table L12 (S13 2254)

Strip bytes EA FF AB EB 33 B9 F7 7B 80 B3 A8, P0 to P3 = 512. Pixels (0, 0) to (7, 0): 192, 191, 268, 606, 315, 638, 337, 680. The bit-level steps are the table at the end of section Compression.

### TV-6: table L14 (S14 3520)

Strip bytes EA FF EB 04 DA 33 B8 1A 5A 22 25 4A, P0 to P3 = 2048.

| Pixel | Code | s | E (bits) | d | Prediction | Value |
| --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 11101 | 11 | 01011111111 | -1280 | 2048 | 768 |
| (1, 0) | 11101 | 11 | 01100000100 | -1275 | 2048 | 773 |
| (2, 0) | 1101 | 10 | 1010001100 | 652 | 768 | 1420 |
| (3, 0) | 11101 | 11 | 11000000110 | 1542 | 773 | 2315 |
| (4, 0) | 100 | 5 | 10110 | 22 | 1420 | 1442 |
| (5, 0) | 100 | 5 | 01000 | -23 | 2315 | 2292 |
| (6, 0) | 100 | 5 | 01001 | -22 | 1442 | 1420 |
| (7, 0) | 010 | 6 | 100101 | 37 | 2292 | 2329 |

### TV-7: table Y12 with a type 2 curve (S14 3516)

Strip bytes F9 E7 E7 8D E3 EA 5A DA FE, P0 to P3 = 328, version 0x44 0x20, S = 16.

| Pixel | Code | s | E (bits) | d | Prediction | Code value | Sensor value C |
| --- | --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 111110 | 8 | 01111001 | -134 | 328 | 194 | 194 |
| (1, 0) | 111110 | 8 | 01111000 | -135 | 328 | 193 | 193 |
| (2, 0) | 110 | 7 | 1111000 | 120 | 194 | 314 | 442 |
| (3, 0) | 111110 | 8 | 10100101 | 165 | 193 | 358 | 684 |
| (4, 0) | 101 | 2 | 01 | -2 | 314 | 312 | 433 |
| (5, 0) | 101 | 2 | 10 | 2 | 358 | 360 | 697 |
| (6, 0) | 101 | 2 | 11 | 3 | 312 | 315 | 446 |
| (7, 0) | 11110 | 0 | - | 0 | 360 | 360 | 697 |

### TV-8: table Y14 with a type 2 curve (S14 3519)

Strip bytes FC ED 7E 79 37 53 FA 4B 7D 95 96 3C, P0 to P3 = 1312, version 0x44 0x20, S = 64.

| Pixel | Code | s | E (bits) | d | Prediction | Code value | Sensor value C |
| --- | --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 1111110 | 10 | 0111011010 | -549 | 1312 | 763 | 765 |
| (1, 0) | 1111110 | 10 | 0111100100 | -539 | 1312 | 773 | 774 |
| (2, 0) | 1101 | 9 | 110101001 | 425 | 763 | 1188 | 1440 |
| (3, 0) | 1111110 | 10 | 1001001011 | 587 | 773 | 1360 | 2320 |
| (4, 0) | 011 | 4 | 1110 | 14 | 1188 | 1202 | 1513 |
| (5, 0) | 1100 | 3 | 101 | 5 | 1360 | 1365 | 2348 |
| (6, 0) | 011 | 4 | 0010 | -13 | 1202 | 1189 | 1445 |
| (7, 0) | 1100 | 3 | 011 | -4 | 1365 | 1361 | 2325 |

### TV-9: table Y12 with a type 1 curve (S20 2060)

Strip bytes FD 62 FD 62 F7 BD EF, P0 to P3 = 328, version 0x44 0x10. Pixel (0, 0): code 1111110 (s = 9), E = 101100010 = 354, d = 354, value 682, C(682) = 4095. Pixel (1, 0): the same bits, value 682. Pixels (2, 0) to (5, 0): code 11110 (s = 0), d = 0, value 682. The bit pattern 11110 repeats (bytes F7 BD EF 7B DE) while the sky is saturated.

### TV-10: curve expansion

- Type 2, S13 2255 (S = 16): C(239) = 239, C(240) = 241, C(241) = 242, C(247) = 250, C(248) = 252, C(255) = 261, C(256) = 263, C(257) = 264, C(260) = 269, C(300) = 375, C(687) = 4081, C(688) = 4095, C(4096) = 4095.
- Type 2, S15 3581 (version 0x44 0x40, S = 16, L = 4097): K(99) = 1584, K(100) = 1600, K(101) = 1632. C(1600) = 1600, C(1601) = 1602, C(1608) = 1616, C(1615) = 1630, C(1616) = 1632, C(4096) = 16383.
- Type 1, S20 2060: C(214) to C(220) = 214, 215, 217, 218, 219, 220, 221. C(680) to C(682) = 4070, 4085, 4095.

### TV-11: white balance from tag 0x0097

- Version "0102" (S23 5632): bytes 0 to 17 begin 30 31 30 32, and the 16-bit big-endian numbers at bytes 10, 12, 14, 16 are 466, 256, 256, 463: mR = 1.8203125, mB = 1.80859375.
- Version "0103" (S20 2060): bytes 20 to 27 = 02 2B 01 00 01 84 01 00 = 555, 256, 388, 256: mR = 2.16796875, mB = 1.515625.

### TV-12: levels, white balance and matrix

CFA cell R 2912, G 4912, G 4912, B 1912, BlackLevel 400 400 400 400 (14-bit file), white 16383, WB_RBLevels 3892/2048, 2817/2048, 2048/2048, 2048/2048, Model NIKON D850. Normalized and balanced: 0.2986787, 0.2822999, 0.1301219. Output linear sRGB for a flat area of this cell: 0.3182917, 0.3202284, 0.0638187.

### TV-13: orientation

The vector of arw.md TV-13 applies unchanged: stored 3 x 2 image a b c / d e f, orientation 6 gives d a / e b / f c, orientation 8 gives c f / b e / a d, orientation 3 gives f e d / c b a.

### Fixture recipes

The fixtures are complete small NEF files of 4 x 2 pixels, one per in-scope variant. All use this layout:
- Bytes 0 to 7: TIFF header of the fixture's byte order, IFD0 at 8.
- IFD0 with 5 entries in ascending order: 0x010F Make ASCII "NIKON CORPORATION" (18 bytes), 0x0110 Model ASCII, 0x0112 Orientation SHORT 1, 0x014A SubIFDs LONG 1 (offset of the raw IFD), 0x8769 ExifIFD LONG 1. Next-IFD offset 0. The two strings follow the IFD.
- Exif IFD with one entry: 0x927C MakerNote UNDEFINED. The maker note follows: 4E 69 6B 6F 6E 00 02 10 00 00, a TIFF header of the same byte order with IFD offset 8, the maker-note IFD (entries in ascending order, next-IFD offset 0) and its values.
- Raw IFD with 12 entries in ascending order: 0x00FE LONG 0, 0x0100 LONG 4, 0x0101 LONG 2, 0x0102 SHORT bps, 0x0103 SHORT compression, 0x0106 SHORT 32803, 0x0111 LONG strip offset, 0x0115 SHORT 1, 0x0116 LONG 2, 0x0117 LONG strip bytes, 0x828D SHORT 2 2, 0x828E BYTE pattern. The sensor data start at the next multiple of 16.
- Values longer than 4 bytes follow their IFD, each at an even offset.

| Fixture | Variant | Byte order, Model | bps, Compression | Maker-note entries | Sensor data (strip bytes) | Decoded raster (rows) | Expected output linear sRGB for every pixel |
| --- | --- | --- | --- | --- | --- | --- | --- |
| F1 | V1 | II, NIKON D850 | 14, 1 | 0x000C RATIONAL 3892/2048 2817/2048 2048/2048 2048/2048, 0x003D SHORT 400 x4 | 60 0B 30 13 60 0B 30 13 30 13 78 07 30 13 78 07 | 2912 4912 2912 4912 / 4912 1912 4912 1912 | 0.3182917 0.3202284 0.0638187 |
| F2 | V2 | MM, NIKON D700 | 12, 1 | 0x000C 461/256 338/256 256/256 256/256 | 27 44 68 27 44 68 46 81 7A 46 81 7A | 628 1128 628 1128 / 1128 378 1128 378 | 0.2965884 0.3229839 0.0681422 |
| F3 | V4 | II, NIKON D850 | 12, 34713 | as F1, and 0x0093 SHORT 6 | D8 C2 4C D8 C2 4C, 10 bytes 00, CC E4 1D CC E4 1D, 10 bytes 00 (row stride 16) | 728 1228 728 1228 / 1228 478 1228 478 | 0.3183515 0.3202885 0.0638307 |
| F4 | V5 | II, NIKON Z 6 | 14, 34713 | 0x000C 609/512 1085/512 512/512 512/512, 0x003D 1008 x4, 0x0051 UNDEFINED 24 bytes "01000400", 00 00, 0A 00, 12 bytes 00 | C0 0D 64 05 DC 40 56, 9 bytes 00, 90 15 76 02 59 61 27, 9 bytes 00 (row stride 16) | 3520 5520 3520 5520 / 5520 2520 5520 2520 | 0.1116440 0.3364037 0.1609110 |
| F5 | V3 | MM, NIKON D100 | 12, 34713 | 0x0093 SHORT 2 | 46 82 74 46 82 74, 10 bytes 00 (row 0) and 17 A4 68 17 A4 68, 10 bytes 00 (row 1) | 1128 628 1128 628 / 378 1128 378 1128, pattern 1 0 2 1 | 0.3686162 0.3162643 0.0796264 (generic WB and matrix) |
| F6 | V6 L12 | MM, NIKON D700 | 12, 34713 | as F2, 0x0093 SHORT 3, 0x0096 UNDEFINED 46 bytes of TV-2 (S13 2254) | BD 3E 9A 3D EF A6 8D 79 F7 80 | 628 1128 628 1128 / 1128 378 1128 378 | 0.2965884 0.3229839 0.0681422 |
| F7 | V6 L14 | II, NIKON D850 | 14, 34713 | as F1, 0x0093 SHORT 3, 0x0096 46 bytes: 46 30 00 08 00 08 00 08 00 08 22 00 and 34 bytes 00 | DD 83 CB 30 FB EF 2C C1 BB FD F0 | as F1 | 0.3182917 0.3202284 0.0638187 |
| F8 | V7 Y12 | MM, NIKON D70 | 12, 34713 | 0x0093 SHORT 1, 0x0096 1412 bytes: 44 10, 01 48 x4, 02 AB, C(i) = i for i < 256 and 256 + 6 * (i - 256) for 256 <= i <= 682, 34 bytes 00 | 85 E9 5E F6 94 97 DE | code values 276 402 276 402 / 402 318 402 318, pattern 2 1 1 0, sensor values 376 1132 376 1132 / 1132 628 1132 628 | 0.3724667 0.3137422 0.1106973 (table WB of NIKON D70) |
| F9 | V8 Y12 | MM, NIKON D3 | 12, 34713 | 0x000C 432/256 375/256 256/256 256/256, 0x0093 SHORT 4, 0x0096 624 bytes: 44 20, 01 48 x4, 01 01, K(j) = 16 * j for j <= 16 and min(4095, 256 + 48 * (j - 16)) above, 98 bytes 00 | 9A 7D B7 EF 7D B6 03 DE | code values 380 547 380 547 / 547 297 547 297, sensor values 628 1129 628 1129 / 1129 379 1129 379 | 0.2720191 0.3185376 0.0993099 |
| F10 | V8 Y14 | II, NIKON Z 6 | 14, 34713 | 0x000C and 0x003D as F4, 0x0051 as F4 with bytes 10, 11 = 04 00, 0x0096 624 bytes: 44 40, 20 05 x4, 01 01, K(j) = 16 * j for j <= 64 and min(16383, 1024 + 64 * (j - 64)) above, 98 bytes 00 | DA 87 ED 13 EF BF 68 92 B7 DF 00 | code values 1648 2148 1648 2148 / 2148 1398 2148 1398, sensor values as F4 | 0.1116440 0.3364037 0.1609110 |

Notes: F3, F4 and F5 rows are padded (16-byte stride or 16-byte groups). The expected output of F5 uses the generic WB 2.099, 1, 1.443 and the generic matrix, because NIKON D100 is in neither table. The expected output of F8 uses the table WB 2.16, 1, 1.64 of NIKON D70, because F8 has no tag 0x000C or 0x0097. Every output pixel of a fixture has the same value, because bilinear demosaicing of a raster made of identical 2x2 cells reproduces the cell everywhere (also at the borders, where only neighbors of the same cell structure exist).

F2 (V2), complete file of 364 bytes:

0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 12 00 00 00 4A 01 10 00 02 00 00 00 0B 00 00
0020: 00 5C 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 00 BE 87 69 00 04 00 00
0040: 00 01 00 00 00 68 00 00 00 00 4E 49 4B 4F 4E 20
0050: 43 4F 52 50 4F 52 41 54 49 4F 4E 00 4E 49 4B 4F
0060: 4E 20 44 37 30 30 00 00 00 01 92 7C 00 07 00 00
0070: 00 44 00 00 00 7A 00 00 00 00 4E 69 6B 6F 6E 00
0080: 02 10 00 00 4D 4D 00 2A 00 00 00 08 00 01 00 0C
0090: 00 05 00 00 00 04 00 00 00 1A 00 00 00 00 00 00
00A0: 01 CD 00 00 01 00 00 00 01 52 00 00 01 00 00 00
00B0: 01 00 00 00 01 00 00 00 01 00 00 00 01 00 00 0C
00C0: 00 FE 00 04 00 00 00 01 00 00 00 00 01 00 00 04
00D0: 00 00 00 01 00 00 00 04 01 01 00 04 00 00 00 01
00E0: 00 00 00 02 01 02 00 03 00 00 00 01 00 0C 00 00
00F0: 01 03 00 03 00 00 00 01 00 01 00 00 01 06 00 03
0100: 00 00 00 01 80 23 00 00 01 11 00 04 00 00 00 01
0110: 00 00 01 60 01 15 00 03 00 00 00 01 00 01 00 00
0120: 01 16 00 04 00 00 00 01 00 00 00 02 01 17 00 04
0130: 00 00 00 01 00 00 00 0C 82 8D 00 03 00 00 00 02
0140: 00 02 00 02 82 8E 00 01 00 00 00 04 00 01 01 02
0150: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0160: 27 44 68 27 44 68 46 81 7A 46 81 7A

Annotations: IFD0 at 0x08 (5 entries, SubIFDs = 0xBE, ExifIFD = 0x68). Make at 0x4A, Model "NIKON D700" at 0x5C. Exif IFD at 0x68 with the maker note (68 bytes) at 0x7A. Maker-note origin 0x7A + 10 = 0x84. Maker-note IFD at 0x84 + 8 = 0x8C with one entry 0x000C (RATIONAL, 4 values at origin + 0x1A = 0x9E: 461/256, 338/256, 256/256, 256/256). Raw IFD at 0xBE with 12 entries, strip at 0x160 with 12 bytes.

F6 (V6, table L12), complete file of 426 bytes:

0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 12 00 00 00 4A 01 10 00 02 00 00 00 0B 00 00
0020: 00 5C 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 01 04 87 69 00 04 00 00
0040: 00 01 00 00 00 68 00 00 00 00 4E 49 4B 4F 4E 20
0050: 43 4F 52 50 4F 52 41 54 49 4F 4E 00 4E 49 4B 4F
0060: 4E 20 44 37 30 30 00 00 00 01 92 7C 00 07 00 00
0070: 00 8A 00 00 00 7A 00 00 00 00 4E 69 6B 6F 6E 00
0080: 02 10 00 00 4D 4D 00 2A 00 00 00 08 00 03 00 0C
0090: 00 05 00 00 00 04 00 00 00 32 00 93 00 03 00 00
00A0: 00 01 00 03 00 00 00 96 00 07 00 00 00 2E 00 00
00B0: 00 52 00 00 00 00 00 00 01 CD 00 00 01 00 00 00
00C0: 01 52 00 00 01 00 00 00 01 00 00 00 01 00 00 00
00D0: 01 00 00 00 01 00 46 30 02 00 02 00 02 00 02 00
00E0: 00 20 B7 28 AA 9D E0 72 5E 90 1A F6 1E 99 65 82
00F0: F0 AF BF 20 D2 D0 2D C8 C7 0C A1 84 C7 58 2D 50
0100: D3 AB 00 00 00 0C 00 FE 00 04 00 00 00 01 00 00
0110: 00 00 01 00 00 04 00 00 00 01 00 00 00 04 01 01
0120: 00 04 00 00 00 01 00 00 00 02 01 02 00 03 00 00
0130: 00 01 00 0C 00 00 01 03 00 03 00 00 00 01 87 99
0140: 00 00 01 06 00 03 00 00 00 01 80 23 00 00 01 11
0150: 00 04 00 00 00 01 00 00 01 A0 01 15 00 03 00 00
0160: 00 01 00 01 00 00 01 16 00 04 00 00 00 01 00 00
0170: 00 02 01 17 00 04 00 00 00 01 00 00 00 0A 82 8D
0180: 00 03 00 00 00 02 00 02 00 02 82 8E 00 01 00 00
0190: 00 04 00 01 01 02 00 00 00 00 00 00 00 00 00 00
01A0: BD 3E 9A 3D EF A6 8D 79 F7 80

Annotations: maker-note IFD at 0x8C with 3 entries: 0x000C (values at 0x84 + 0x32 = 0xB6), 0x0093 = 3, 0x0096 (46 bytes at 0x84 + 0x52 = 0xD6). Raw IFD at 0x104, Compression 0x8799 = 34713, strip at 0x1A0 with 10 bytes. The strip encodes, with P0 to P3 = 512: (0, 0) code 101 (s = 7) E = 1110100 (d = 116), (1, 0) code 111110 (s = 10) E = 1001101000 (d = 616), (2, 0) and (3, 0) code 11110 (d = 0), (0, 1) as (1, 0), (1, 1) code 1101 (s = 8) E = 01111001 (d = 121 - 255 = -134), (2, 1) and (3, 1) code 11110. That is 74 bits, followed by 6 zero bits.

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 Codes 11111110 and 111111111 of table Y12 and the subtree 111111111 of table Y14 never occurred in 12 decoded lossy files (9 with table Y12, 3 with table Y14, M7, M8). They would carry categories 11 and 12 (Y12) or 13 and 14 (Y14), which the curve ranges make rare or impossible. Fallback: R-NEF-31 (fail, preview path).
- OQ-2 Order of the start predictors P0 to P3. All measured files have four equal values. Fallback: P(2 * y + x) as in section Compression.
- OQ-3 Order of the four BlackLevel values for CFA patterns other than 0 1 1 2, and unequal values. Not observed. Fallback: by CFA position (R-NEF-37).
- OQ-4 BlackLevel scale. The division by 2^(14 - bps) is measured on 12-bit files of D750, D850, D4S, D6, Z 6 and 1 V3 (M11). Fallback: R-NEF-37 for all models.
- OQ-5 Crop of files without tag 0x0045. The centered preview rule matches six models (M15). The D70 aligns 2 pixels left of center. Fallback: R-NEF-40.
- OQ-6 White balance of models without tag 0x000C other than D70 and D2H (D100, Coolpix E series) and ColorBalance versions other than "0102" and "0103" (S5 calls most of them encrypted). The D100 version "0100" holds numbers whose meaning the author could not establish. Fallback: table or generic scales (R-NEF-38).
- OQ-7 Color matrices. S9 gives D50 matrices with 2 decimals for 49 models and nothing for others (for example D6, D780, Z 5, Z 50, Z fc, Z 30, Z f, Z 8, D3500, Nikon 1, Coolpix A). The D50 matrix is used for every scene illuminant. Fallback: R-NEF-41 with the generic matrix.
- OQ-8 White level. Some 14-bit lossy curves end at 16380 (S25: D4S, D5500, D5600, D7200, D750, D810). The sensor may saturate below 2^bps - 1. Fallback: w = 2^bps - 1 (R-NEF-36). Highlight reconstruction is not specified.
- OQ-9 High Efficiency (X1). No permitted source describes the coding. Fallback: preview path.
- OQ-10 Small raw (X2). The data look like luma and chroma pairs (M17), but the transform, the levels and the color handling are not documented. Fallback: preview path.
- OQ-11 D1-family lossy files (X3) carry no curve. D1X pixels are not square (X4). Fallback: preview path.
- OQ-12 Coolpix E-series NEF (X5): field-interleaved rows and CMYG patterns. Fallback: preview path.
- OQ-13 NEFCompression 5, 7 and 9 (S5) were not found in any NEF sample (they may belong to Coolpix NRW files). Fallback: X7, preview path.
- OQ-14 V1 with BitsPerSample 12 was not observed (12-bit uncompressed files use V2 or V4). Fallback: R-NEF-3 rule 4 accepts it, and R-NEF-25 clips to 4095.
- OQ-15 Strip padding: up to 30 bits after the last code, and 6565 bytes in the 1 V3 file (M18). Fallback: R-NEF-29 ignores them.
- OQ-16 The bytes after the curve in tag 0x0096 (34 or 98 bytes) and bytes 10 and 11 of lossless tags (32 or 34) have no known meaning. They did not change the decoding of any measured file. Fallback: ignored.
- OQ-17 Orientation of the embedded preview for rotated shots. No rotated sample was measured. Fallback: R-NEF-57.
- OQ-18 Lens corrections (S5 lists distortion and vignetting data in tag 0xC7D5 of Z models). Fallback: no lens correction.
- OQ-19 Maker notes with a byte order different from the file byte order were not observed. Fallback: R-NEF-5 uses the maker-note header.
- OQ-20 The reduced CFA images of Z models (SubIFDs with NewSubfileType 1 and Compression 34713, for example 1520x1010) are not described. Fallback: not used (R-NEF-15 selects NewSubfileType 0).
