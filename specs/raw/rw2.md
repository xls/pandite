# Panasonic RW2 raw format: decoder specification

Document: specs/raw/rw2.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite RW2 decoder (clean-room step 1 of 2).

This document describes the Panasonic RW2 raw file format, and its predecessor RAW format of the same container, as facts. It is written for an implementer who has only this document, TIFF 6.0 (S1) and ITU-T T.81 (S2). Every normative statement is traceable to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Conventions

- Requirements have the form R-RW2-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x49 0x49 or "49 49".
- Bit 0 is the least significant bit of a value.
- floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0). max(a, b) and min(a, b) are the larger and the smaller value.
- Pixel coordinates are (x, y): x is the column, y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (tags 0x0002 and 0x0003 of IFD0).
- D is the sample bit depth of a variant: 12 or 14.
- F is the full-scale value 2^D - 1: 4095 for D = 12 and 16383 for D = 14.

### Glossary

- RW2: the raw file format of Panasonic Lumix cameras from about 2008 on. RW2 is a TIFF-like file with its own magic number.
- RAW (Panasonic): the older raw format of some Lumix cameras (file extension .RAW). It uses the same container and the same IFD0 tags.
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2).
- IFD0: the first IFD of the file. In RW2 it holds the sensor data description and the camera tags.
- CFA: color filter array. In all measured files it is a 2x2 Bayer pattern (S8).
- CFA position: the index 2 * (y mod 2) + (x mod 2) of a pixel inside its 2x2 repeat cell (0, 1, 2 or 3).
- RawFormat: the value of IFD0 tag 0x002D. It selects the storage method of the sensor data.
- Variant: one storage method of the sensor data. This document defines eight variants, V1 to V8, in the section Scope and variants.
- Block: 16 bytes of sensor data that hold a fixed number of pixels.
- Block code: the lossy fixed-rate coding of variants V1, V4 and V5. Each block holds two initial values and coded differences.
- Packed variant: a variant that stores plain D-bit values in blocks (V2, V3, V6, V7).
- Chunk: 16384 bytes of sensor data in variants V1, V2 and V3. The bytes of a chunk are stored rotated.
- Logical stream: the sensor data bytes in their coding order, after the chunk rotation is undone.
- Active area: the rectangle of the stored raster that holds image data, given by the border tags 0x0004 to 0x0007.
- Black level: the sensor value that represents zero light.
- White level: the sensor value at saturation.
- WB multipliers: the per-channel gains that make a neutral surface give equal red, green and blue values.
- Camera RGB: red, green and blue values in the spectral sensitivities of the camera.
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S9). The decoder output is linear sRGB.
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Container layout (IFD structure, field types), Decoding procedure (orientation) |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Embedded preview (baseline JPEG, markers), Error handling |
| S3 | ExifTool Tag Names: PanasonicRaw tags table (metadata tag reference documentation, last revised Jul 11, 2024) | https://exiftool.org/TagNames/PanasonicRaw.html | community documentation | 2026-11-14 | Scope and variants (Compression names), Container layout, Metadata tags (tag names, CFAPattern values 1 to 4, Orientation values) |
| S4 | K. Okamoto, Y. Nagaishi, T. Juri (Panasonic Corp.), Digital signal encoding and decoding device and method, US Patent 8,275,207 B2 (2012, priority JP 2005-215785) | https://patents.google.com/patent/US8275207B2/en | patent | 2026-11-14 | Compression (structure of the block code: initial pixel values per packing unit, zones that share a class value, fixed-length codes, offset value = previous same-color value minus the quantization reference value with reset to zero, decoding by bit shift and addition) |
| S5 | diglloyd blog, RawDigger support for Panasonic S1R, 2019-04-11, quoting the RawDigger developers (article text only) | https://diglloyd.com/blog/2019/20190411_1220-RawDigger-support-for-PanasonicS1R.html | community documentation | 2026-11-14 | Scope and variants, Compression (V5: "delta encoding, 14-bit base, 10-bit delta", "older cameras are 12-bit base/8bit delta", "Delta is scaled separately for each 3 pixels") |
| S6 | DXOMARK camera sensor Measurements pages, tab Color response (one page per model, URL pattern https://www.dxomark.com/Cameras/Panasonic/<name>---Measurements, names in the Color conversion table) | https://www.dxomark.com/Cameras/Panasonic/Lumix-DC-GH5---Measurements | community documentation | 2026-11-14 | Color conversion (matrices, D50 white balance scales), Levels and white balance (fallback WB) |
| S7 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear demosaicing) |
| S8 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-14 | Sensor data layout (Bayer arrangement) |
| S9 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (output primaries and D65 white), measurement tool (sRGB decoding of previews) |
| S10 | Measurement: DMC-LX3 sample "Panasonic - DMC-LX3 - 1:1.RW2" (CC0), saved as DMC-LX3_1x1.RW2 | https://raw.pixls.us/getfile.php/1395/nice/Panasonic%20-%20DMC-LX3%20-%201:1.RW2 | measurement | 2026-11-14 | V1, records M3, M5, M6, M8, M9, M10, M12 |
| S11 | Measurement: DMC-GF1 sample "Panasonic - DMC-GF1 - 1:1.RW2" (CC0) | https://raw.pixls.us/getfile.php/1475/nice/Panasonic%20-%20DMC-GF1%20-%201:1.RW2 | measurement | 2026-11-14 | V1, records M6, M8, M9, M10, M11 |
| S12 | Measurement: DMC-GH4 sample "Panasonic - DMC-GH4 - 1:1.RW2" (CC0) | https://raw.pixls.us/getfile.php/2568/nice/Panasonic%20-%20DMC-GH4%20-%201:1.RW2 | measurement | 2026-11-14 | V1, records M6, M8, M9, M10 |
| S13 | Measurement: DMC-GX8 sample "Panasonic - DMC-GX8 - 1:1.RW2" (CC0) | https://raw.pixls.us/getfile.php/2110/nice/Panasonic%20-%20DMC-GX8%20-%201:1.RW2 | measurement | 2026-11-14 | V1, records M6, M8, M9, M10 |
| S14 | Measurement: DC-GH5 sample "Panasonic - DC-GH5 - 1:1.RW2" (CC0), a scene with a color chart | https://raw.pixls.us/getfile.php/1514/nice/Panasonic%20-%20DC-GH5%20-%201:1.RW2 | measurement | 2026-11-14 | V1, records M3, M5, M6, M8, M9, M10, M12, test vectors |
| S15 | Measurement: DC-GH5S samples "Panasonic - DC-GH5S - 1:1.RW2", archive remarks "12bit 1:1" and "14bit 1:1" (CC0) | https://raw.pixls.us/getfile.php/2610/nice/Panasonic%20-%20DC-GH5S%20-%201:1.RW2 | measurement | 2026-11-14 | V3, records M3, M4, M8, M10, M15 (12-bit remark file: https://raw.pixls.us/getfile.php/2606/nice/Panasonic%20-%20DC-GH5S%20-%201:1.RW2) |
| S16 | Measurement: DC-G9 sample "Panasonic - DC-G9 - 4:3.RW2", archive remark High resolution mode, "simulated one shot" secondary file (CC0) | https://raw.pixls.us/getfile.php/2585/nice/Panasonic%20-%20DC-G9%20-%204:3.RW2 | measurement | 2026-11-14 | V2, records M3, M4, M8, M10 |
| S17 | Measurement: DC-S1 sample "Panasonic - DC-S1 - 3:2.RW2" (CC0) | https://raw.pixls.us/getfile.php/3038/nice/Panasonic%20-%20DC-S1%20-%203:2.RW2 | measurement | 2026-11-14 | V5, records M2, M7, M8, M9, M10, test vectors |
| S18 | Measurement: DC-GH5M2 sample "Panasonic - DC-GH5M2 - 1:1.RW2" (CC0) | https://raw.pixls.us/getfile.php/5085/nice/Panasonic%20-%20DC-GH5M2%20-%201:1.RW2 | measurement | 2026-11-14 | V4, records M7, M8, M9, M10, M13 |
| S19 | Measurement: DC-S5 sample "Panasonic - DC-S5 - 3:2.RW2", archive remark "Live Composite mode (panasonic v7, 14bit)" (CC0) | https://raw.pixls.us/getfile.php/6339/nice/Panasonic%20-%20DC-S5%20-%203:2.RW2 | measurement | 2026-11-14 | V6, records M2, M4, M8, M9 |
| S20 | Measurement: DC-GH6 sample "Panasonic - DC-GH6 - 1:1.RW2" (CC0) | https://raw.pixls.us/getfile.php/5879/nice/Panasonic%20-%20DC-GH6%20-%201:1.RW2 | measurement | 2026-11-14 | Scope and variants (X1), Embedded preview, records M13, M14 |
| S21 | Measurement: DMC-FZ8 sample "Panasonic - DMC-FZ8 - 4:3.RAW" (CC0) | https://raw.pixls.us/getfile.php/2282/nice/Panasonic%20-%20DMC-FZ8%20-%204:3.RAW | measurement | 2026-11-14 | V7, records M1, M4, M9 |
| S22 | Measurement: DMC-FZ50 sample "Panasonic - DMC-FZ50 - 16:9.RAW" (CC0) | https://raw.pixls.us/getfile.php/2323/nice/Panasonic%20-%20DMC-FZ50%20-%2016:9.RAW | measurement | 2026-11-14 | V8, records M1, M9, M10 |
| S23 | Measurement: DMC-G2, DMC-GM1S and DMC-FZ2000 samples "1:1" (CC0), archive ids 6230, 6821, 2168 | https://raw.pixls.us/getfile.php/6230/nice/Panasonic%20-%20DMC-G2%20-%201:1.RW2 | measurement | 2026-11-14 | Levels and white balance (white level), Container layout (CameraIFD), records M10, M11, M16 |
| S24 | Measurement: container survey, first 64 KiB of up to 3 files of each of 106 Panasonic models (272 files) in the CC0 part of the archive | https://raw.pixls.us/data/Panasonic/ | measurement | 2026-11-14 | Scope and variants, Container layout, Metadata tags, record M1 |

Sample files are from the public raw sample archive raw.pixls.us. The archive lists a license for each file. Only files marked CC0 (public domain) were downloaded or surveyed. Folders that contain files with another license (DMC-FZ18, FZ200, FZ30, FZ72, G1, GF3, GF5, GX7, L1, L10, LF1, LX2, LX5, TZ71, and the LX3 files imported from another site) were left out of the survey. The author downloaded the files and inspected them with measurement tools that the author wrote from S1, S2 and S9 for this purpose. The tools are not part of the deliverable. One comparison decoded the embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder. The files kept for the decoder tests are listed in .testdata/rw2/SOURCES.txt. The files of S11, S12, S13 and S23 and the 12-bit file of S15 were measured but are not kept there.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S10 to S24): every RW2 file starts with 49 49 55 00 18 00 00 00 (little-endian, magic number 0x0055, IFD0 at offset 24). Bytes 8 to 23 are the same 16 bytes in all RW2 files: 88 E7 74 D8 F8 25 1D 4D 94 7A 6E 77 82 2B 5D 6A. The RAW files S21 and S22 start with 49 49 55 00 08 00 00 00 (IFD0 at offset 8). IFD0 entries are in ascending tag order in all files. IFD0 has no next IFD (next-IFD offset 0). The survey S24 parsed IFD0 of all 272 files. RawFormat and BitsPerSample by model: RawFormat 4 with 12 bits in 88 models (DMC-GH1 to DC-G9, see Scope and variants), RawFormat 5 with 12 bits (DC-G9 high resolution) and 14 bits (DC-GH5S), RawFormat 6 with 12 bits (DC-GH5M2) and 14 bits (DC-S1, S1H, S1R, S5), RawFormat 7 with 14 bits (DC-S5), RawFormat 8 with 12, 14 or 16 bits (DC-GH6, GH7, G9M2, S1M2, S1M2ES, S1RM2, S5M2, S5M2X, S9), RawFormat 2 with 12 bits (DMC-FZ8), no RawFormat tag with Compression 34828 (DMC-FZ50, DMC-LX1).
- M2 (data length): for RawFormat 4 and 5, the bytes from RawDataOffset to the end of the file are a whole number of 16384-byte chunks (S10 544, S11 641, S12 841, S13 1434, S14 1434, S15 844, S16 2013 chunks), and this length is the smallest whole number of chunks that holds W * H / p blocks of 16 bytes (p pixels per block). For RawFormat 6 and 7 the data length is exactly W * H / p * 16 bytes (S17 35469312, S18 23486464, S19 43180032), and StripByteCounts equals it when present. StripByteCounts is not the data length in RawFormat 4 files (S10: 15576624 = 2 * W * H; S14: 0).
- M3 (chunk rotation, S10, S14, S15, S16): with the logical stream "stored bytes 8184 to 16383 of each chunk, then stored bytes 0 to 8183" every decoded image is clean. Inside the active area, the mean absolute vertical same-color residual (pixel minus the mean of the same-color pixels two rows above and below) of the pixels of block 0, block 512 (the block that straddles the stored chunk end) and block 1023 of each chunk equals that of all pixels (S10: 50.8, 55.8, 55.1, all 53.3. S14: 28.2, 29.4, 28.8, all 29.0. S15: 123.4, 134.1, 126.9, all 126.3. S16: 25.6, 25.2, 25.2, all 26.1). With the split at 8200 in place of 8184 block 1023 of each chunk is wrong (S10: 447.0. S14: 161.1. S15: 1993.2. S16: 179.3). A scan of all split points 8, 24, ..., 16376 on S15 finds 8184 as the only split with continuous image data across chunk boundaries. Inside a chunk the 16-byte blocks start at stored offsets 8 mod 16.
- M4 (packed variants and V8): reading each block as a little-endian 128-bit integer with pixel i in bits D * i to D * i + D - 1 gives clean images. Mean vertical same-color residual for this reading and for the big-endian reading: S15 (D = 14, 9 pixels) 126 against over 2000, S16 (D = 12, 10 pixels) 25.6 against 740.6, S19 (D = 14, 9 pixels, no chunks) 19.0 against 2671.0, S21 (D = 12, 10 pixels, no chunks) 50.0 against 1006.1. The unused top bits (126 and 127 for D = 14, 120 to 127 for D = 12) are zero. S22 (V8): read as 16-bit little-endian samples, 99.9 percent of the samples have their 4 low bits equal to 0, the maximum is 65520, and the mean vertical residual is 290 (big-endian: 19057).
- M5 (block code fields, S10, S14, S18): the mean of each bit of the little-endian 128-bit block integer over 200000 blocks shows two 12-bit fields at bits 116 to 127 and 104 to 115 (top bits rarely set, like image values), 2-bit fields at bits 102, 76, 50 and 24 (means 0.14 and 0.06 on S10, 0.03 and 0.02 on S14), and 8-bit fields in between whose top bit has a mean of about 0.35. This is the layout of the table in section Compression, V1. The codes of class-0 zones cluster around 128 (S10 median 118, S14 median 128).
- M6 (block code rule, S10 to S14): decoding classes 0, 1 and 2 with step 1, 2 and 4 and the offset of R-RW2-27 gives clean images. For class 3, three rules were compared with a consistency test that uses the class choice of S4: a zone of class c in 1 or 2 must contain a decoded difference of at least 64 * 2^c and none of 128 * 2^c or more, and a class-0 zone must have all differences below 128. Zones that follow a class-3 zone violate this test in 7 to 23 percent of cases when class 3 decodes as step 8 with offset, and in 0.0 to 1.7 percent (S11: 6.1 percent) when class 3 decodes as code * 16 without prediction. With code * 16, class-3 zones have a difference of at least 512 in 96.8 to 99.8 percent of cases. In saturated sky (neighbor initial values 4095) class-3 pixels carry code 255 (S10: 153 of 243 cases) and class 0 to 2 pixels carry code 128. Values above 4095 occur inside the active area only as 4096 or 4097 (S10: 400 pixels, S12: 3 pixels). Outside the active area larger values occur (S10 up to 5326, columns 2772 to 2827).
- M7 (block code with 14 bits, S17; with 12 bits, S18): S17 blocks have 14-bit fields at bits 114 to 127 and 100 to 113, 2-bit fields at bits 98, 66 and 34, 10-bit fields in between, and bits 0 to 3 equal to 0 in all blocks except the all-FF blocks right of the active area. S18 has the layout of V1. With class 3 decoded as code * 16 the value range inside the active area is 509 to 16353 (S17) and 137 to 3361 (S18). With class 3 as step 8 it is 100 to 36711 and 2 to 4237. The consistency test of M6 gives 0.2 and 0.8 percent violations after class-3 zones (2.1 and 12.7 percent with step 8), and class-3 zones have a difference of at least 2048 (S17) or 512 (S18) in 99.9 and 99.0 percent of cases.
- M8 (comparison with the embedded JPEG): each file was decoded by this document, reduced to half size (one RGB value per 2x2 cell), leveled, white balanced, converted with the matrix of section Color conversion and compared with the embedded JPEG (S2, S9 decoding) resized to the same size, in 16x16 blocks. Correlation of the cube root of the block luminance and mean chromaticity error (sum of absolute differences of r, g, b chromaticity): S10 0.960 and 0.109, S11 0.989 and 0.110, S12 0.942 and 0.059, S13 0.952 and 0.044, S14 0.993 and 0.037, S15 0.945 and 0.161, S16 0.854 and 0.297 (0.189 with the identity matrix), S17 0.989 and 0.098, S18 0.938 and 0.150, S19 0.966 and 0.040. All decoded images show the same scene as their previews with plausible colors. S21 and S22 have no preview. Their decoded images are clean photographs with plausible colors after white balance.
- M9 (CFA pattern, S10 to S19, S21, S22): CFAPattern (tag 0x0009) values 1, 3 and 4 occur. For each file the chromaticity error of M8 (identity matrix) was computed for all four values. The value in the tag gives the smallest error in every file (S10: 0.316 against 0.364, 0.839, 0.840. S14: 0.070 against 0.175, 0.432, 0.432). The pattern origin is the stored pixel (0, 0): S22 has an odd top border (7). With the origin at (0, 0) the mean white-balanced image color is R/G 1.08, B/G 1.01. With the origin at the border corner it is R/G 2.24, B/G 4.49.
- M10 (black level): the pixels outside the active area are not optically masked (their medians are 136 to 4080). In S14 the gray patches of a color chart (12 patches from white to black) fit the model "R - d = k (G - d)" with d = 14.2 raw units above the tag value (rms residual 1.0). A sweep of a constant offset added to the tag values, scored by the chromaticity error of M8, has its minimum at +15 (S14, S12), +18 (S13, S18), +15 (S16), +24 to +27 (S10, S11, flat minimum) and is flat for 14-bit files (S17 minimum at +24 to +36 with 0.095 against 0.102 at 0). The 0.01 percentile of the values inside the active area is 143 (S12, S14, tag 128 and 129), 144 (S23 GM1S, tag 128), 145 (S18, S23 FZ2000, tags 130 and 127), 527 and 529 (S15, S17, tag 511) and 19 to 32 (S10, S11, S23 G2, tag 0). S22 (RAW, 16-bit) has 1 percent of its values at 7 or below, so its black level is near 0.
- M11 (white level): LinearityLimit (tags 0x000E to 0x0010) is 4095 or 16383 in most files. S23 G2 has 3967 and its saturated pixels pile up at 3976 to 3978. S10 and S12 (4095) pile up at 4095. S11 has 16383 although its values are 12-bit. S23 GM1S has 2111 and S23 FZ2000 has 2626 (both at an extended low ISO setting), and 33613 and 144 of their pixels exceed it without any pile-up.
- M12 (stored raster outside the active area): S14 holds four all-FF blocks at the end of each row (columns 5208 to 5263). S17 holds four all-FF blocks at the end of each row (columns 6028 to 6071). The columns right of the right border of S10 hold values that do not continue the image.
- M13 (previews): IFD0 tag 0x002E holds a baseline JPEG (SOF0) of 1920 pixels wide: 1920x1920 (1:1 files), 1920x1440 (4:3 sensor), 1920x1280 (3:2 sensor). It starts with FF D8 FF E1 (APP1 Exif segment). Its image covers the active area of the 4:3 or 3:2 sensor, also in files shot with another aspect ratio setting. Some streams have bytes after the EOI marker (S10: 316 bytes, S11: 439, S12: 95, S13: 89). S20 has tag 0x0127 with a 5776x4336 baseline JPEG that starts with FF D8 FF DB and has 294 bytes after EOI. S18 has tag 0x0127 with count 0. The RAW files S21 and S22 have neither tag.
- M14 (out-of-scope structures, S20, S24): RawFormat 8 files have no Compression tag, BitsPerSample 12, 14 or 16, tags 0x0039 to 0x0048 with binary parameter blocks, and data lengths that are not a fixed number of bytes per pixel (S20: 1.494 bytes per pixel; DC-G9M2 samples 1.376 and 1.382; DC-S5M2 1.258 and 1.261).
- M15 (12-bit mode stored as 14-bit, S15): the file with the archive remark "12bit" has BitsPerSample 14 and RawFormat 5, and every value inside the active area is a multiple of 4. The "14bit" file has 24.9 percent multiples of 4.
- M16 (CameraIFD, S15, S23): tag 0x0120 holds a TIFF structure that starts with 49 49 2A 00 in S15, and 16 bytes that do not form a TIFF header (05 6F 71 40 ...) in S23 GM1S.

### Sources encountered and not used

Web search result lists showed titles of pages that contain or discuss decoder source code: ExifTool source modules (PanasonicRaw.pm on sources.debian.org, cpancover.com and codeclimate.com), an exiv2 source file on bazaar.launchpad.net, a discuss.pixls.us thread titled "About the algorithm of decompressing Panasonic RW2", libraw.org pages about the GH6 format and LibRaw comment lists. They were seen only as titles and short summaries. They were not opened and not used. Two ExifTool forum pages (msg=31163, msg=38249) were fetched and returned only a notice that the forum is offline. See the report of the author for the full list.

## Scope and variants

### Generations and variants

IFD0 tag 0x002D (RawFormat) selects the storage method. S3 names the tag but gives no values. The measured values and models are in M1. S3 names the Compression values 34316 "Panasonic RAW 1", 34826 "Panasonic RAW 2", 34828 "Panasonic RAW 3" and 34830 "Panasonic RAW 4". RawFormat 6 and 8 files have no Compression tag (M14, M7), so the decoder classifies by RawFormat and BitsPerSample.

| Variant | RawFormat (0x002D) | D | Pixels per block p | Chunk rotation | Coding | Measured models |
| --- | --- | --- | --- | --- | --- | --- |
| V1 | 4 | 12 | 14 | yes | block code, 8-bit codes | DMC-LX3, GF1, GH4, GX8, DC-GH5 (88 survey models, DMC-GH1 to DC-G9) |
| V2 | 5 | 12 | 10 | yes | packed | DC-G9 (high resolution mode files) |
| V3 | 5 | 14 | 9 | yes | packed | DC-GH5S (12-bit and 14-bit camera modes, M15) |
| V4 | 6 | 12 | 14 | no | block code, 8-bit codes | DC-GH5M2 |
| V5 | 6 | 14 | 11 | no | block code, 10-bit codes | DC-S1, S1H, S1R, S5 |
| V6 | 7 | 14 | 9 | no | packed | DC-S5 (live composite and multiple exposure files) |
| V7 | 2 | 12 | 10 | no | packed | DMC-FZ8 (.RAW) |
| V8 | absent, Compression 34828 | 12 | 1 | no | 16-bit containers | DMC-FZ50, DMC-LX1 (.RAW) |

S5 states for a full-frame Lumix single shot "delta encoding, 14-bit base, 10-bit delta", for older cameras "12-bit base/8bit delta", and "Delta is scaled separately for each 3 pixels". S4 describes the general method. M5 to M7 fix the bit layout and the rules.

### Out-of-scope relatives

| Id | Relative | Detection from file contents |
| --- | --- | --- |
| X1 | RawFormat 8 (DC-GH6, GH7, G9M2, S1M2, S1M2ES, S1RM2, S5M2, S5M2X, S9): variable-length coding with parameters in tags 0x0039 to 0x0048 (M14). No permitted source describes it. | RawFormat 8 |
| X2 | any other RawFormat value, and RawFormat 7 with 12 bits (not measured) | RawFormat present and the pair (RawFormat, BitsPerSample) is not in the variant table |
| X3 | RAW files with RawFormat absent and another Compression value or another data length | RawFormat absent and the rule of V8 does not match |
| X4 | big-endian files or another magic number | header bytes 0 to 3 differ from 49 49 55 00 |
| X5 | files whose raster width is not a multiple of p | W mod p is not 0 |

Leica cameras write RWL files with the same tags (S3). No RWL sample was measured. The decoder classifies them by the same rules.

### Requirements

- R-RW2-1: The decoder must decode the sensor data of variants V1 to V8 as this document specifies.
- R-RW2-2: The decoder must use the preview path (section Embedded preview) for X1 to X5 and must not interpret their sensor data.
- R-RW2-3: The decoder must classify the file by these rules, in this order, and must treat a file that matches no rule as X2 or X3:
  1. Header bytes 0 to 3 are not 49 49 55 00: X4.
  2. RawFormat is 8: X1.
  3. RawFormat 4 and BitsPerSample 12: V1.
  4. RawFormat 5 and BitsPerSample 12: V2.
  5. RawFormat 5 and BitsPerSample 14: V3.
  6. RawFormat 6 and BitsPerSample 12: V4.
  7. RawFormat 6 and BitsPerSample 14: V5.
  8. RawFormat 7 and BitsPerSample 14: V6.
  9. RawFormat 2 and BitsPerSample 12: V7.
  10. RawFormat absent, Compression 34828, BitsPerSample 12 and StripByteCounts at least 2 * W * H: V8.
- R-RW2-4: The decoder must use the preview path (X5) when W mod p is not 0 for the classified variant.

## Container layout

### Byte order and header

An RW2 file is a TIFF file (S1 section 2) with the magic number 85 (0x0055) in place of 42. All measured files are little-endian (M1).

| Offset | Size | Field | Value |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 49 49 ("II", little-endian) |
| 2 | 2 | magic number | 85 (bytes 55 00) |
| 4 | 4 | offset of IFD0 | 24 in RW2 files, 8 in RAW files |
| 8 | 16 | RW2 only: a constant (M1) | not used by the decoder |

- R-RW2-5: The decoder must accept a file only if bytes 0 to 3 are 49 49 55 00.
- R-RW2-6: The decoder must read every multi-byte field and every sample of V8 as little-endian.
- R-RW2-7: The decoder must read IFD0 at the offset in header bytes 4 to 7 and must not depend on bytes 8 to 23.

### IFD structure

IFD0 has the structure of a TIFF 6.0 IFD (S1 section 2):

| Offset in IFD | Size | Field |
| --- | --- | --- |
| 0 | 2 | N, the number of entries |
| 2 + 12 * i | 2 | tag of entry i |
| 4 + 12 * i | 2 | field type of entry i |
| 6 + 12 * i | 4 | count of entry i (number of values, not bytes) |
| 10 + 12 * i | 4 | value or offset of entry i |
| 2 + 12 * N | 4 | offset of the next IFD, 0 in all measured files |

Field types and their sizes in bytes (S1 section 2): 1 BYTE 1, 2 ASCII 1, 3 SHORT 2, 4 LONG 4, 5 RATIONAL 8, 6 SBYTE 1, 7 UNDEFINED 1, 8 SSHORT 2, 9 SLONG 4, 10 SRATIONAL 8, 11 FLOAT 4, 12 DOUBLE 8. The value bytes of an entry occupy count * size bytes. If that total is 4 or less, the value is in the 4-byte value field, starting at its first byte. If the total is more than 4, the value field holds the file offset of the value bytes.

- R-RW2-8: The decoder must read a value from the value field when count * size <= 4 and from the offset in the value field when count * size > 4.
- R-RW2-9: The decoder must find tags by tag number and must not depend on the entry order.
- R-RW2-10: The decoder must accept SHORT and LONG for every integer tag of the table in section Metadata tags, must skip entries with a type outside 1 to 12, and must use the first value when an integer tag has more than one value.
- R-RW2-11: The decoder must treat IFD0 as invalid when its offset lies outside the file, when N is 0 or greater than 1024, or when the 2 + 12 * N + 4 bytes of the IFD do not lie completely inside the file, and must then report an error (there is no preview without IFD0).
- R-RW2-12: The decoder must compare ASCII values after it removes all trailing zero bytes and spaces.

### Other structures

- Exif IFD (IFD0 tag 0x8769): standard Exif data. Not needed for decoding.
- CameraIFD (IFD0 tag 0x0120, type UNDEFINED, S3): a block of camera data. In some files it is a small TIFF structure, in others it is not (M16).
- JpgFromRaw (0x002E) and JpgFromRaw2 (0x0127): embedded JPEG streams (section Embedded preview).
- R-RW2-13: The decoder must not depend on the Exif IFD, the CameraIFD or the maker note to decode the sensor data.

## Metadata tags

All tags below are in IFD0. "Required" means the variant cannot be decoded without the tag. The decoder then uses the preview path. "Default" is the value to use when an optional tag is absent or invalid. Names are from S3.

| Tag | Name (S3) | Type | Count | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- |
| 0x0002 | SensorWidth | SHORT | 1 | stored width W in pixels | all | none |
| 0x0003 | SensorHeight | SHORT | 1 | stored height H in pixels | all | none |
| 0x0004 | SensorTopBorder | SHORT | 1 | first row of the active area | optional | 0 |
| 0x0005 | SensorLeftBorder | SHORT | 1 | first column of the active area | optional | 0 |
| 0x0006 | SensorBottomBorder | SHORT | 1 | row after the last row of the active area | optional | H |
| 0x0007 | SensorRightBorder | SHORT | 1 | column after the last column of the active area | optional | W |
| 0x0009 | CFAPattern | SHORT | 1 | 1 = R G / G B, 2 = G R / B G, 3 = G B / R G, 4 = B G / G R (S3, M9) | all | none |
| 0x000A | BitsPerSample | SHORT | 1 | D: 12 or 14 (12 for V7 and V8) | all | none |
| 0x000B | Compression | SHORT | 1 | 34316 or 34828, absent in RawFormat 6 and 8 files | V8 | none |
| 0x000E | LinearityLimitRed | SHORT | 1 | used for the white level (R-RW2-31) | optional | F |
| 0x000F | LinearityLimitGreen | SHORT | 1 | as above, green | optional | F |
| 0x0010 | LinearityLimitBlue | SHORT | 1 | as above, blue | optional | F |
| 0x001C | BlackLevelRed | SHORT | 1 | black level tag value of red pixels | optional | 0 |
| 0x001D | BlackLevelGreen | SHORT | 1 | black level tag value of green pixels | optional | 0 |
| 0x001E | BlackLevelBlue | SHORT | 1 | black level tag value of blue pixels | optional | 0 |
| 0x0024 | WBRedLevel | SHORT | 1 | as-shot white balance level of red | optional | R-RW2-34 |
| 0x0025 | WBGreenLevel | SHORT | 1 | as-shot white balance level of green | optional | R-RW2-34 |
| 0x0026 | WBBlueLevel | SHORT | 1 | as-shot white balance level of blue | optional | R-RW2-34 |
| 0x002D | RawFormat | SHORT | 1 | storage method (section Scope and variants) | V1 to V7 | none |
| 0x002E | JpgFromRaw | UNDEFINED | length | embedded JPEG stream | optional | none |
| 0x002F to 0x0032 | CropTop, CropLeft, CropBottom, CropRight | SHORT | 1 | aspect ratio crop chosen in the camera, not used | ignored | none |
| 0x0110 | Model | ASCII | any | model name, for example "DC-GH5" | optional | empty |
| 0x0111 | StripOffsets | LONG | 1 | V7, V8: file offset of the sensor data. 0xFFFFFFFF in RW2 files | V7, V8 | none |
| 0x0112 | Orientation | SHORT | 1 | 1 to 8, S1 definition | optional | 1 |
| 0x0117 | StripByteCounts | LONG | 1 | V7, V8: bytes of sensor data. Not reliable in RW2 files (M2) | V8 | none |
| 0x0118 | RawDataOffset | LONG | 1 | V1 to V6: file offset of the sensor data | V1 to V6 | none |
| 0x0120 | CameraIFD | UNDEFINED | any | not used (M16) | ignored | none |
| 0x0121 | Multishot | LONG | 1 | 0 = off, 65536 = pixel shift (S3), informational | ignored | none |
| 0x0127 | JpgFromRaw2 | UNDEFINED | length | embedded full-size JPEG stream (M13) | optional | none |

Other tags seen in IFD0 and not used: 0x0001, 0x0008, 0x000D, 0x0011, 0x0012, 0x0013, 0x0017 to 0x001B, 0x0027, 0x0029 to 0x002C, 0x0033 to 0x0048, 0x0064, 0x0116, 0x0119, 0x011A to 0x011F, 0x0122, 0x0126, 0x013B, 0x02BC, 0x8298, 0x8769.

- R-RW2-14: The decoder must use the preview path when a tag marked "required" for the classified variant is absent, has a count of 0, or has a value of 0 for W or H.
- R-RW2-15: The decoder must accept the border tags only if 0 <= top < bottom <= H, 0 <= left < right <= W, bottom - top >= 16 and right - left >= 16, and must otherwise use the whole raster as the active area.
- R-RW2-16: The decoder must use the preview path when CFAPattern is not 1, 2, 3 or 4.

## Sensor data layout

### Raster and active area

The raw raster has W columns and H rows. The active area is the rectangle of rows top to bottom - 1 and columns left to right - 1 (tags 0x0004 to 0x0007). The stored raster is wider than the active area in many files. The pixels outside the active area are not image data and are not optically masked (M10, M12). The embedded preview shows the active area (M13).

Measured dimensions (M1):

| File | Variant | W x H | top, left, bottom, right | Active area |
| --- | --- | --- | --- | --- |
| DMC-LX3 (S10) | V1 | 2828 x 2754 | 6, 8, 2742, 2744 | 2736 x 2736 |
| DMC-GH4 (S12) | V1 | 3472 x 3472 | 8, 8, 3464, 3464 | 3456 x 3456 |
| DC-GH5 (S14) | V1 | 5264 x 3904 | 8, 12, 3896, 5196 | 5184 x 3888 |
| DC-G9 (S16) | V2 | 5280 x 3904 | 8, 12, 3896, 5196 | 5184 x 3888 |
| DC-GH5S (S15) | V3 | 2808 x 2768 | 8, 12, 2760, 2764 | 2752 x 2752 |
| DC-GH5M2 (S18) | V4 | 5264 x 3904 | 8, 12, 3896, 5196 | 5184 x 3888 |
| DC-S1 (S17) | V5 | 6072 x 4016 | 8, 12, 4008, 6012 | 6000 x 4000 |
| DC-S5 (S19) | V6 | 6048 x 4016 | 8, 12, 4008, 6012 | 6000 x 4000 |
| DMC-FZ8 (S21) | V7 | 3130 x 2319 | 7, 15, 2311, 3087 | 3072 x 2304 |
| DMC-FZ50 (S22) | V8 | 3690 x 2067 | 7, 16, 2059, 3664 | 3648 x 2052 |

- R-RW2-17: The decoder must deliver the active area as the image and must not apply the crop tags 0x002F to 0x0032.

### CFA arrangement

The CFA pattern repeats every 2 rows and 2 columns. Its origin is the stored pixel (0, 0), not the corner of the active area (M9). The colors of CFA positions 0, 1, 2, 3 for each CFAPattern value:

| CFAPattern | Position 0 (x even, y even) | Position 1 (x odd, y even) | Position 2 (x even, y odd) | Position 3 (x odd, y odd) |
| --- | --- | --- | --- | --- |
| 1 | red | green | green | blue |
| 2 | green | red | blue | green |
| 3 | green | blue | red | green |
| 4 | blue | green | green | red |

- R-RW2-18: The decoder must give pixel (x, y) the color of CFA position 2 * (y mod 2) + (x mod 2) from the table above, with the origin at stored pixel (0, 0).

### Sensor data start and size

The sensor data start at RawDataOffset (0x0118) in V1 to V6 and at StripOffsets (0x0111) in V7 and V8. Pixels are stored row by row from row 0, each row from column 0, with no padding between rows. In V1 to V7 every block of 16 bytes holds p consecutive pixels of one row (W is a multiple of p). Block n holds the pixels with the raster index n * p to n * p + p - 1, where the raster index of pixel (x, y) is y * W + x. The number of blocks is NB = W * H / p.

- R-RW2-19: The decoder must take the data start from tag 0x0118 for V1 to V6 and from tag 0x0111 for V7 and V8.
- R-RW2-20: The decoder must require for V4 to V7 that the data start plus 16 * NB bytes lies inside the file, and for V8 that the data start plus 2 * W * H bytes lies inside the file.

### Chunk rotation (V1, V2, V3)

The logical stream of V1, V2 and V3 is cut into chunks of 16384 bytes. Chunk c occupies file bytes S + 16384 * c to S + 16384 * c + 16383, where S is the data start. Inside a chunk the bytes are stored rotated (M3): logical byte i of chunk c (i from 0 to 16383) is at file offset S + 16384 * c + ((i + 8184) mod 16384). The logical byte with the index L = 16384 * c + i is byte L of the logical stream, and block n is logical bytes 16 * n to 16 * n + 15.

Example: logical byte 0 is at S + 8184. Logical byte 8199 is at S + 16383. Logical byte 8200 is at S + 0. Logical byte 16383 is at S + 8183. Block 512 of a chunk (logical bytes 8192 to 8207) has its first 8 bytes at chunk offsets 16376 to 16383 and its last 8 bytes at chunk offsets 0 to 7.

- R-RW2-21: The decoder must read logical byte L of a V1, V2 or V3 stream from file offset S + 16384 * floor(L / 16384) + ((L mod 16384 + 8184) mod 16384).
- R-RW2-22: The decoder must require for V1, V2 and V3 that the data start plus 16384 * ceil(16 * NB / 16384) bytes lies inside the file, and must use the preview path otherwise.

## Compression

This section specifies, for each variant, how the 16 bytes of a block become pixel values. Every block is read as an unsigned 128-bit integer in little-endian order: B = sum over k = 0 to 15 of byte[k] * 2^(8k), where byte[k] is byte k of the block in the logical stream. The field with start bit s and width n has the value floor(B / 2^s) mod 2^n.

### Packed variants (V2, V3, V6, V7)

A block holds p = floor(128 / D) values of D bits. Value j (j from 0 to p - 1) of block n is the field with start bit D * j and width D, and is the value of the pixel with raster index n * p + j (M4). The bits above D * p are 0 in all measured blocks.

| Variant | D | p | Value j | Unused bits |
| --- | --- | --- | --- | --- |
| V2, V7 | 12 | 10 | bits 12j to 12j + 11 | 120 to 127 |
| V3, V6 | 14 | 9 | bits 14j to 14j + 13 | 126 and 127 |

- R-RW2-23: The decoder must set pixel n * p + j of a packed variant to floor(B / 2^(D * j)) mod 2^D of block n.

Worked example (vector TV-6, V3): block bytes 00 02 FA F0 FF 03 00 FF 9F 34 11 0E 61 09 BC 02. B mod 2^14 = 0x0200 = 512. Bits 14 to 27 are floor(B / 2^14) mod 2^14: bytes 1 to 3 give 0xF0FA02, floor(0xF0FA02 / 2^6) mod 2^14 = 1000. The nine values are 512, 1000, 16383, 0, 8191, 1234, 4321, 600, 700.

### V8: 16-bit containers

Each pixel is a 16-bit unsigned little-endian integer at offset 2 * (y * W + x) from the data start. The 12-bit value is in the top 12 bits (M4).

- R-RW2-24: The decoder must read a V8 pixel as floor((b0 + 256 * b1) / 16), where b0 and b1 are its two bytes.

Worked example: bytes B0 46 give 0x46B0 = 18096, and floor(18096 / 16) = 1131.

### Block code (V1, V4, V5)

Source of the method: S4 describes a camera codec that writes, in each packing unit of fixed size, the first pixel value of each color as an initial value, followed by zones of pixels that share one class value (the quantization width) and one fixed-length code per pixel. The code of a pixel is the quantized value "target - offset", where the offset is the previous decoded pixel of the same color minus the quantization reference value, and is reset to 0 when it is negative. The decoder shifts the code left by the quantization width and adds the offset (S4, decoding steps S206a to S206h). S5 gives the bit widths of two generations. M5 to M7 give the bit layout, the pixel order and the meaning of the four class values.

Each block holds p pixels of one row. Pixels 0 and 1 are the initial values, one for each of the two colors of the row. Pixels 2 to p - 1 form zones of three pixels each: zone z holds pixels 2 + 3z, 3 + 3z and 4 + 3z. Each zone has a 2-bit class value c and three codes of N bits. The predecessor of pixel i (i >= 2) is pixel i - 2 of the same block, which has the same color.

| Variant | D | N (code bits) | Zones | p | Unused bits |
| --- | --- | --- | --- | --- | --- |
| V1, V4 | 12 | 8 | 4 | 14 | none |
| V5 | 14 | 10 | 3 | 11 | 0 to 3 |

#### Bit layout

Fields are read from the most significant end of B (M5, M7):

| Field | V1, V4: start bit, width | V5: start bit, width |
| --- | --- | --- |
| pixel 0 (initial value) | 116, 12 | 114, 14 |
| pixel 1 (initial value) | 104, 12 | 100, 14 |
| class of zone z | 102 - 26z, 2 | 98 - 32z, 2 |
| code j (0 to 2) of zone z, for pixel 2 + 3z + j | 94 - 26z - 8j, 8 | 88 - 32z - 10j, 10 |

For V1 and V4 this gives: zone 0 class at bit 102, codes at 94, 86, 78. Zone 1 class at 76, codes at 68, 60, 52. Zone 2 class at 50, codes at 42, 34, 26. Zone 3 class at 24, codes at 16, 8, 0. For V5: zone 0 class at 98, codes at 88, 78, 68. Zone 1 class at 66, codes at 56, 46, 36. Zone 2 class at 34, codes at 24, 14, 4.

- R-RW2-25: The decoder must extract the fields of a V1, V4 or V5 block from the little-endian 128-bit integer B with the start bits and widths of the table above.

#### Reconstruction

Let Q = 2^(N - 1) (128 for V1 and V4, 512 for V5). For pixel i >= 2 with class c, code k and predecessor value P (the reconstructed value of pixel i - 2):

| Class c | Step s | Reconstructed value before the limit |
| --- | --- | --- |
| 0 | 1 | max(P - Q * s, 0) + k * s |
| 1 | 2 | max(P - Q * s, 0) + k * s |
| 2 | 4 | max(P - Q * s, 0) + k * s |
| 3 | none | k * 2^(D - N), that is k * 16 in all three variants |

The value is then limited to F: a value above F becomes F. The limited value is the pixel value and is the predecessor of pixel i + 2. Pixels 0 and 1 are their initial values without change.

When P >= Q * s, the formula of classes 0 to 2 is P + (k - Q) * s: the code is a signed difference with the bias Q. When P < Q * s, the offset is 0 and the code holds the value itself (S4, offset reset). Class 3 holds the value itself with 16 steps over the full range (M6, M7).

- R-RW2-26: The decoder must set pixels 0 and 1 of a block code block to its two initial values.
- R-RW2-27: The decoder must reconstruct pixels 2 to p - 1 in increasing order by the table above, with P the already limited value of pixel i - 2 of the same block.
- R-RW2-28: The decoder must limit every reconstructed value to F before it uses the value as a predecessor or as a pixel value.
- R-RW2-29: The decoder must decode each block without information from other blocks.

#### Worked example (vector TV-3, V1)

Block bytes: 03 FF 64 03 FE 2B 08 28 83 1C 20 9E 20 D0 87 3E.
- B as a hexadecimal number (byte 15 first): 3E87D020 9E201C83 28082BFE 0364FF03.
- Pixel 0 = bits 116 to 127 = 0x3E8 = 1000. Pixel 1 = bits 104 to 115 = 0x7D0 = 2000.
- Zone 0: class bits 102 to 103 = 0, codes 130, 120, 128. Pixel 2 = max(1000 - 128, 0) + 130 = 1002. Pixel 3 = 2000 - 128 + 120 = 1992. Pixel 4 = 1002 - 128 + 128 = 1002.
- Zone 1: class 1 (s = 2), codes 200, 50, 128. Pixel 5 = 1992 - 256 + 400 = 2136. Pixel 6 = 1002 - 256 + 100 = 846. Pixel 7 = 2136 - 256 + 256 = 2136.
- Zone 2: class 2 (s = 4), codes 10, 255, 128. Pixel 8 = 846 - 512 + 40 = 374. Pixel 9 = 2136 - 512 + 1020 = 2644. Pixel 10 = max(374 - 512, 0) + 512 = 512 (offset reset).
- Zone 3: class 3, codes 100, 255, 3. Pixels 11, 12, 13 = 1600, 4080, 48.
- Result: 1000 2000 1002 1992 1002 2136 846 2136 374 2644 512 1600 4080 48.

## Levels and white balance

The decoder determines the white level, the black levels and the WB multipliers first, then normalizes each CFA value.

### White level

- R-RW2-30: The decoder must set F to 4095 for D = 12 (V1, V2, V4, V7, V8) and to 16383 for D = 14 (V3, V5, V6).
- R-RW2-31: For each color c, the decoder must use LinearityLimit of that color (tags 0x000E, 0x000F, 0x0010) as the white level w_c when 0.9 * F <= value <= F, and F otherwise.

M11 is the basis: a tag value close to F marks the saturation point (G2: 3967, pile-up at 3976 to 3978). Lower values occur at extended ISO settings where valid data exceed them, and one model writes 16383 for 12-bit data.

### Black level

The tags 0x001C to 0x001E give a black level per color. In V1 to V6 the measured black level is 15 raw units above the tag value (M10: color chart fit 14.2 on S14, chromaticity sweeps with minima at +15 to +18 on five 12-bit files, and the darkest 0.01 percent of the values at tag + 15 to tag + 18 in eight files). In V8 the black level is near the tag value 0 (M10). V7 has a tag value of 0 and no preview for a comparison. Files of the 12-bit camera mode of DC-GH5S store 14-bit values that are multiples of 4 (M15) and need no special handling.

- R-RW2-32: The decoder must use the black level b_c = BlackLevel tag of color c + 15 for V1 to V6, and b_c = BlackLevel tag of color c for V7 and V8, where c is the color of the CFA position (R-RW2-18).
- R-RW2-33: The decoder must use b_c = 0 for a missing black level tag before it adds the offset of R-RW2-32, and must use the preview path when b_c >= w_c - 255.

### White balance multipliers

The decoder computes three multipliers mR, mG, mB. Sources in priority order:
1. Tags 0x0024, 0x0025, 0x0026 (WBRedLevel, WBGreenLevel, WBBlueLevel) with all three values greater than 0: mR = WBRedLevel / WBGreenLevel, mG = 1, mB = WBBlueLevel / WBGreenLevel.
2. The D50 white balance scales of the Model in the table of section Color conversion (S6): mR and mB from the table, mG = 1.
3. The generic scales mR = 2.337, mG = 1, mB = 1.735 (the means of the table).

- R-RW2-34: The decoder must take the WB multipliers from the first available source of the list above.

Measured tag values: S10 523, 263, 416. S14 606, 256, 456. S17 538, 256, 476. In S14 the as-shot multipliers 2.367 and 1.781 and the black level of R-RW2-32 bring the 12 gray chart patches (0.5 to 38 percent of full scale) to R/G 0.92 to 0.97 and B/G 1.02 to 1.10, without a trend with brightness (M10).

Worked example: tags 600, 300, 450 give mR = 600 / 300 = 2, mB = 450 / 300 = 1.5.

### Normalization

For a CFA pixel with value v, color c, black level b_c, white level w_c and multiplier m_c:
- n = (v - b_c) / (w_c - b_c)
- u = n * m_c, clipped to the range 0 to 1.

- R-RW2-35: The decoder must compute u = clip((v - b_c) / (w_c - b_c) * m_c, 0, 1) for every CFA pixel of the active area before demosaicing.

Worked example (fixture F1): V1, tags 128, so b = 143. w = 4095. R = 1131: (1131 - 143) / 3952 = 0.25, times 2 = 0.5. G = 2119: 1976 / 3952 = 0.5. B = 1131: 0.25 times 1.5 = 0.375.

## Color conversion

### Matrix sources

The measured RW2 files carry no color matrix in IFD0 (the tag list of M1 has none, and S3 names none). The decoder therefore uses a per-model table. S6 (DXOMARK) publishes for each tested camera a "color matrix as defined in ISO standard 17321" for two illuminants, with white balance scales. The page shows two blocks without illuminant labels. As in specs/raw/arw.md, the D50 block is the block with the larger ratio of the R scale to the B scale. In every row below that is the first block of the page. Each printed row ("R raw", "G raw", "B raw") is one output channel (R, G, B of sRGB). Each column is one camera channel. Two facts fix this orientation: the rows sum to 1 (to within 0.01), so a white-balanced neutral stays neutral, and M8 confirms it: with the transposed matrix the chromaticity error is 0.46 to 0.76 in all measured files, against 0.04 to 0.16 with the rows as outputs.

How to read S6: open https://www.dxomark.com/Cameras/Panasonic/<name>---Measurements, tab "Color response". The table gives the D50 block of each listed model, read on 2026-11-14. Values are rounded to 2 decimals by S6.

| Model (IFD0 0x0110) | S6 name | WB R | WB B | m11 | m12 | m13 | m21 | m22 | m23 | m31 | m32 | m33 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DMC-LX3 | Lumix-DMC-LX3 | 2.55 | 1.52 | 1.81 | -0.84 | 0.03 | -0.14 | 1.63 | -0.49 | 0.08 | -0.76 | 1.68 |
| DMC-G1 | Lumix-DMC-G1 | 2.39 | 1.32 | 1.68 | -0.65 | -0.03 | -0.18 | 1.55 | -0.36 | 0.01 | -0.39 | 1.38 |
| DMC-GH1 | Lumix-DMC-GH1 | 2.84 | 1.45 | 1.82 | -0.79 | -0.03 | -0.17 | 1.72 | -0.55 | 0.07 | -0.66 | 1.59 |
| DMC-GF1 | Lumix-DMC-GF1 | 2.32 | 1.28 | 1.70 | -0.71 | 0.01 | -0.16 | 1.48 | -0.33 | 0.03 | -0.43 | 1.40 |
| DMC-G2 | Lumix-DMC-G2 | 1.95 | 1.52 | 1.68 | -0.64 | -0.04 | -0.16 | 1.47 | -0.31 | 0.05 | -0.49 | 1.44 |
| DMC-GH2 | Lumix-DMC-GH2 | 2.33 | 1.79 | 1.83 | -0.80 | -0.03 | -0.19 | 1.65 | -0.46 | 0.09 | -0.77 | 1.68 |
| DMC-G3 | Lumix-DMC-G3 | 2.59 | 1.71 | 1.77 | -0.82 | 0.05 | -0.21 | 1.69 | -0.48 | 0.13 | -0.77 | 1.64 |
| DMC-GX1 | Lumix-DMC-GX1 | 2.59 | 1.74 | 1.75 | -0.74 | 0.00 | -0.20 | 1.68 | -0.49 | 0.12 | -0.80 | 1.67 |
| DMC-GH3 | Lumix-DMC-GH3 | 2.34 | 1.97 | 2.11 | -1.06 | -0.05 | -0.24 | 1.67 | -0.43 | 0.13 | -0.71 | 1.58 |
| DMC-G5 | Lumix-DMC-G5 | 2.28 | 1.76 | 1.91 | -0.87 | -0.04 | -0.19 | 1.68 | -0.49 | 0.12 | -0.88 | 1.76 |
| DMC-GX7 | Lumix-DMC-GX7 | 2.39 | 1.66 | 1.77 | -0.68 | -0.09 | -0.16 | 1.59 | -0.42 | 0.09 | -0.71 | 1.63 |
| DMC-GM1 | Lumix-DMC-GM1 | 2.46 | 1.74 | 1.70 | -0.55 | -0.15 | -0.15 | 1.56 | -0.42 | 0.08 | -0.67 | 1.59 |
| DMC-GH4 | Lumix-DMC-GH4 | 2.35 | 1.96 | 2.05 | -0.95 | -0.11 | -0.24 | 1.67 | -0.42 | 0.11 | -0.65 | 1.54 |
| DMC-LX100 | Lumix-DMC-LX100 | 2.36 | 1.75 | 1.77 | -0.69 | -0.08 | -0.16 | 1.57 | -0.41 | 0.11 | -0.76 | 1.65 |
| DMC-GX8 | Lumix-DMC-GX8 | 2.23 | 2.05 | 2.10 | -1.08 | -0.02 | -0.24 | 1.64 | -0.40 | 0.14 | -0.72 | 1.58 |
| DMC-G80 | Lumix-DMC-G80 | 2.29 | 1.60 | 1.70 | -0.55 | -0.15 | -0.15 | 1.58 | -0.43 | 0.07 | -0.63 | 1.57 |
| DMC-FZ1000 | Lumix-DMC-FZ1000 | 2.32 | 1.98 | 1.78 | -0.72 | -0.07 | -0.21 | 1.53 | -0.32 | 0.10 | -0.72 | 1.62 |
| DMC-ZS100 | Lumix-DMC-ZS100 | 2.42 | 1.90 | 1.88 | -0.81 | -0.07 | -0.23 | 1.57 | -0.33 | 0.08 | -0.71 | 1.63 |
| DC-GH5 | Lumix-DC-GH5 | 2.14 | 2.01 | 2.00 | -0.91 | -0.09 | -0.22 | 1.61 | -0.38 | 0.09 | -0.61 | 1.52 |
| DC-GH5M2 | Lumix-DC-GH5-II | 2.19 | 2.03 | 2.01 | -0.96 | -0.05 | -0.23 | 1.61 | -0.39 | 0.10 | -0.71 | 1.61 |
| DC-S1 | Lumix-DC-S1 | 2.17 | 1.80 | 1.91 | -0.83 | -0.07 | -0.15 | 1.48 | -0.33 | 0.08 | -0.63 | 1.56 |
| DC-S1R | Lumix-DC-S1R | 2.04 | 1.59 | 1.61 | -0.52 | -0.09 | -0.08 | 1.15 | -0.07 | 0.05 | -0.40 | 1.35 |
| DC-S5 | Lumix-DC-S5 | 2.20 | 1.78 | 1.94 | -0.85 | -0.09 | -0.17 | 1.55 | -0.38 | 0.07 | -0.63 | 1.56 |

The model names in the first column are IFD0 Model strings measured in S10 to S24 or the folder names of the sample archive, which uses the IFD0 Model string. The S6 page for DC-GH5M2 is titled "Lumix DC-GH5 II". M8 measured S18 with this row: chromaticity error 0.150 against 0.210 with the identity matrix.

Generic matrix for models that are not in the table: the element-wise mean of the 23 matrices above, rounded to 3 decimals. It improves the chromaticity error against the identity matrix on 8 of the 10 measured files and makes it worse on S16 (0.297 against 0.189, Open questions OQ-9):

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 1.838 | -0.783 | -0.055 |
| sRGB G | -0.184 | 1.580 | -0.395 |
| sRGB B | 0.087 | -0.661 | 1.575 |

The generic WB scales are the means of the table: R 2.337, B 1.735.

### Procedure

- R-RW2-36: The decoder must select the table row whose Model column equals the IFD0 Model string (compared by R-RW2-12), and must use the generic matrix when no row matches.
- R-RW2-37: The decoder must divide each row of the selected matrix by the sum of that row before use, so that every row sums to 1.
- R-RW2-38: The decoder must compute the output (r, g, b) from the white-balanced, normalized camera values (u_R, u_G, u_B) with the normalized matrix as follows, and must clip r, g and b to the range 0 to 1:
  - r = m11 * u_R + m12 * u_G + m13 * u_B
  - g = m21 * u_R + m22 * u_G + m23 * u_B
  - b = m31 * u_R + m32 * u_G + m33 * u_B
- R-RW2-39: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S9), without the sRGB transfer function.

Worked example: the values of fixture F1 (u_R, u_G, u_B) = (0.5, 0.5, 0.375) and the DC-GH5 row. Row sums are 1.00, 1.01 and 1.00. After normalization, row 2 is -0.2178218, 1.5940594, -0.3762376.
- r = 2.00 * 0.5 - 0.91 * 0.5 - 0.09 * 0.375 = 1.0 - 0.455 - 0.03375 = 0.51125.
- g = -0.2178218 * 0.5 + 1.5940594 * 0.5 - 0.3762376 * 0.375 = -0.1089109 + 0.7970297 - 0.1410891 = 0.5470297.
- b = 0.09 * 0.5 - 0.61 * 0.5 + 1.52 * 0.375 = 0.045 - 0.305 + 0.57 = 0.31.

A neutral value (0.25, 0.25, 0.25) stays (0.25, 0.25, 0.25) for every normalized matrix.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and read the header (R-RW2-52, R-RW2-5, R-RW2-6, R-RW2-7).
2. Read IFD0 and validate it (R-RW2-11, R-RW2-8, R-RW2-9, R-RW2-10, R-RW2-12).
3. Collect the preview candidates now, so that every later failure can fall back (R-RW2-45, R-RW2-46).
4. Classify the file (R-RW2-3, R-RW2-4). For X1 to X5 use the preview path (R-RW2-2). Check the required tags (R-RW2-14, R-RW2-16).
5. Determine the active area (R-RW2-15, R-RW2-17), the CFA colors (R-RW2-18), the white levels (R-RW2-30, R-RW2-31), the black levels (R-RW2-32, R-RW2-33) and the WB multipliers (R-RW2-34).
6. Check all sizes against the file and the limits before allocating memory (R-RW2-19 to R-RW2-22, section Error handling).
7. Decode the sensor data:
   - V1, V2, V3: undo the chunk rotation (R-RW2-21).
   - V1, V4, V5: R-RW2-25 to R-RW2-29.
   - V2, V3, V6, V7: R-RW2-23.
   - V8: R-RW2-24.
8. Normalize the CFA values of the active area (R-RW2-35).
9. Demosaic inside the active area (R-RW2-40, R-RW2-41, R-RW2-42).
10. Convert to linear sRGB (R-RW2-36, R-RW2-37, R-RW2-38).
11. Apply the orientation (R-RW2-43, R-RW2-44).
12. Deliver the image (R-RW2-39). If any step from 4 to 11 fails, deliver the preview instead (R-RW2-49).

### Demosaicing

The decoder uses bilinear interpolation, the basic non-adaptive method described in S7. For each pixel and each color that the pixel lacks, the missing value is the mean of the pixels of that color among the 8 neighbors of the pixel. With a 2x2 Bayer pattern this rule gives these cases:
- Green at a red or blue pixel: the mean of the 4 orthogonal neighbors.
- Red or blue at a green pixel: the mean of the 2 row neighbors or of the 2 column neighbors, whichever pair has that color.
- Red at a blue pixel, and blue at a red pixel: the mean of the 4 diagonal neighbors.

- R-RW2-40: For each pixel (x, y) of the active area, the decoder must set its own color to its normalized value, and each missing color to the arithmetic mean of the normalized values of the pixels of that color among the 8 neighbors (x + dx, y + dy), with dx and dy from -1 to 1 and not both 0.
- R-RW2-41: The decoder must ignore neighbors outside the active area in that mean, and must use 0 when no neighbor of the missing color lies inside the active area.
- R-RW2-42: The decoder must use the CFA colors of the stored raster (R-RW2-18) also when the active area starts at an odd row or column.

Worked example: active area with left = 15 and top = 7 (S21) and CFAPattern 1. The first active pixel (15, 7) has CFA position 2 * 1 + 1 = 3 and is blue. Its neighbor (16, 7) is green, (15, 8) is green and (16, 8) is red. The neighbors in row 6 and column 14 are outside the active area and are ignored.

### Orientation

- R-RW2-43: The decoder must read Orientation from IFD0 tag 0x0112, use 1 when the tag is absent or outside 1 to 8, and apply it after demosaicing and color conversion.
- R-RW2-44: The decoder must map pixel (x, y) of a W' x H' image (the active area) to the output pixel of the table below, with an output of W' x H' pixels for values 1 to 4 and H' x W' pixels for values 5 to 8.

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

These mappings follow the definitions of S1. For example, S1 defines value 6 as "the 0th row represents the visual right-hand side of the image, and the 0th column represents the visual top". Stored row 0 therefore goes to the rightmost output column, and stored column 0 goes to the top output row. S3 names the same value "Rotate 90 CW".

## Embedded preview

### Locations

| Location | Tag | Content (M13) |
| --- | --- | --- |
| IFD0 | 0x002E JpgFromRaw, type UNDEFINED, count = stream length, value field = file offset | baseline JPEG, 1920 pixels wide, shows the active area, starts with FF D8 FF E1 (APP1 Exif) |
| IFD0 | 0x0127 JpgFromRaw2, same encoding | full-size baseline JPEG in RawFormat 8 files (S20: 5776x4336), count 0 in some files |
| none | RAW files (V7, V8) | no embedded JPEG was found (S21, S22) |

The preview streams are decoded with S2 (baseline sequential DCT process). Their length can include bytes after the EOI marker (M13).

### Requirements

- R-RW2-45: The decoder must collect as preview candidates the streams of IFD0 tags 0x002E and 0x0127, and must keep a candidate only if count >= 4, offset + count lies inside the file and the first two bytes are FF D8.
- R-RW2-46: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment (S2 B.2.2) and must choose the candidate with the largest width * height.
- R-RW2-47: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan.
- R-RW2-48: The decoder must apply the IFD0 Orientation (R-RW2-44) to the preview, except that it must not apply values 5 to 8 when the preview is taller than wide.
- R-RW2-49: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 to X5, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-RW2-50: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-RW2-39) or a preview (sRGB-encoded JPEG values, not linear).
- R-RW2-51: The decoder must not depend on the Exif data inside the preview stream.

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-RW2-49).

### Limits

- R-RW2-52: The decoder must fail for a file shorter than 8 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-RW2-53: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728).
- R-RW2-54: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values.
- R-RW2-55: The decoder must check, before reading, that every value, data range and preview stream lies completely inside the file (R-RW2-20, R-RW2-22, R-RW2-45).
- R-RW2-56: The decoder must allocate the raster only after the checks of R-RW2-53 and R-RW2-55, with W * H values of at least 16 bits.

### Structure checks

- R-RW2-57: The decoder must fail when the data start (tag 0x0118 or 0x0111) is 0xFFFFFFFF, lies before the end of IFD0, or lies outside the file.
- R-RW2-58: The decoder must fail when the pair (RawFormat, BitsPerSample) does not match the variant table, and must never choose a bit layout from the file size alone.
- R-RW2-59: The decoder must compute the block code values before the limit in an integer type of at least 32 bits, because they reach F + (2^N - 1 - Q) * 4 (F + 508 for V1 and V4, F + 2044 for V5).
- R-RW2-60: The decoder must decode all-FF blocks and other blocks outside the active area by the normal rules and must not report them as errors.
- R-RW2-61: The decoder must clip every normalized value to 0 to 1 (R-RW2-35) and every output value to 0 to 1 (R-RW2-38).
- R-RW2-62: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check selects the default of the quantity.
- R-RW2-63: The decoder must not read the CameraIFD (0x0120) as an IFD.

### Behaviour on error

- R-RW2-64: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-RW2-65: The decoder must discard a partly decoded raw image on failure and must not deliver it.
- R-RW2-66: For a RAW file (V7, V8) without a preview candidate, the decoder must report a decoding failure as an error, because no fallback exists.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order. The vectors from camera files were decoded by the tools and form part of clean images (M8).

### TV-1: header

Bytes 0x49 0x49 0x55 0x00 0x18 0x00 0x00 0x00: RW2, little-endian, magic 85, IFD0 at offset 24 (R-RW2-5). Bytes 0x49 0x49 0x55 0x00 0x08 0x00 0x00 0x00: RAW file, IFD0 at offset 8. Bytes 0x49 0x49 0x2A 0x00: plain TIFF, X4. Bytes 0x4D 0x4D 0x00 0x55: big-endian, X4.

### TV-2: IFD entries

- Entry 2D 00 03 00 01 00 00 00 04 00 00 00: tag 0x002D (RawFormat), SHORT, count 1, value 4.
- Entry 0B 00 03 00 01 00 00 00 0C 86 00 00: tag 0x000B (Compression), SHORT, value 0x860C = 34316.
- Entry 18 01 04 00 01 00 00 00 00 02 00 00 (fixture F1): tag 0x0118 (RawDataOffset), LONG, value 512.
- Entry 10 01 02 00 07 00 00 00 26 01 00 00 (fixture F1): tag 0x0110 (Model), ASCII, count 7, at offset 0x126.
- Entry 2E 00 07 00 00 E2 0B 00 00 06 00 00 (S10): tag 0x002E (JpgFromRaw), UNDEFINED, count 0x0BE200 = 778752, stream at offset 0x600 = 1536.
- Entry 18 01 04 00 01 00 00 00 00 E8 0B 00 (S10): tag 0x0118 (RawDataOffset), LONG, value 0x0BE800 = 780288.

### TV-3: V1 block, all classes and the offset reset

Input: 0x03 0xFF 0x64 0x03 0xFE 0x2B 0x08 0x28 0x83 0x1C 0x20 0x9E 0x20 0xD0 0x87 0x3E.
Fields: pixel 0 1000, pixel 1 2000. Zone 0 class 0, codes 130 120 128. Zone 1 class 1, codes 200 50 128. Zone 2 class 2, codes 10 255 128. Zone 3 class 3, codes 100 255 3.
Output pixels 0 to 13: 1000 2000 1002 1992 1002 2136 846 2136 374 2644 512 1600 4080 48.

### TV-4: V1 block, limit at F

Input: 0x80 0x80 0x80 0x00 0x02 0x02 0x02 0x08 0x08 0x08 0x20 0xC0 0x3F 0xC8 0xF0 0xFF.
Fields: pixel 0 4095, pixel 1 200. All classes 0. Codes 255 0 128, then 128 for all other pixels.
Pixel 2 = 4095 - 128 + 255 = 4222, limited to 4095. Pixel 3 = 200 - 128 + 0 = 72. Pixel 5 = 72 - 128 is negative, so the offset is 0 and pixel 5 = 0 + 128 = 128.
Output: 4095 200 4095 72 4095 128 4095 128 4095 128 4095 128 4095 128.

### TV-5: V1 blocks from a camera file, with the chunk rotation

S14 (DC-GH5), data start S = 718336, W = 5264.
- Row 0, columns 0 to 13: block 0, logical bytes 0 to 15, file offsets 726520 to 726535 (S + 8184). Bytes: 0x81 0x87 0x9D 0x08 0x96 0x15 0x72 0x88 0x17 0x07 0xE0 0xA0 0x1E 0x00 0x61 0x0C. Fields: 198, 256, classes 0 0 0 0, codes 122 131 128 113 120 135 133 101 130 157 135 129. Output: 198 256 192 259 192 244 184 251 189 224 191 253 198 254.
- Row 1952, columns 2604 to 2617: block 734138, logical byte 11746208 = 16384 * 716 + 15264, file offset S + 16384 * 716 + ((15264 + 8184) mod 16384) = 718336 + 11730944 + 7064 = 12456344. Bytes: 0x89 0x7E 0x82 0xF4 0xDD 0x1D 0x02 0x38 0x88 0x88 0x5F 0x60 0x21 0xB0 0x90 0x09. Fields: 153, 176, classes 0 0 0 0, codes 133 129 126 136 131 128 135 119 125 130 126 137. Output: 153 176 158 177 156 185 159 185 166 176 163 178 161 187.

### TV-6: packed blocks (V2, V3, V6, V7)

- D = 14, p = 9: 0x00 0x02 0xFA 0xF0 0xFF 0x03 0x00 0xFF 0x9F 0x34 0x11 0x0E 0x61 0x09 0xBC 0x02 -> 512 1000 16383 0 8191 1234 4321 600 700.
- D = 12, p = 10: 0x80 0xF0 0xFF 0x00 0x00 0x80 0x2C 0x21 0x4D 0x8A 0x7C 0x3E 0x01 0xD0 0x04 0x00 -> 128 4095 0 2048 300 1234 3210 999 1 77.
- S15 (V3), row 1000, columns 1404 to 1412: block 312156, logical byte 4994496 = 16384 * 304 + 13760, file offset 922112 + 4980736 + ((13760 + 8184) mod 16384) = 5908408. Bytes: 0x0A 0x03 0x65 0x61 0x2F 0x38 0x15 0xF9 0x82 0x3D 0x11 0x2E 0xBC 0x14 0xFB 0x02 -> 778 1428 758 1358 761 1270 737 1327 763.
- S16 (V2), row 1000, columns 2640 to 2649: file offset 9096312. Bytes: 0xA4 0x50 0x0E 0xA2 0x20 0x0E 0xA7 0xA0 0x0E 0xA3 0x70 0x0E 0xA5 0x10 0x0F 0x00 -> 164 229 162 226 167 234 163 231 165 241.
- S19 (V6, no rotation), row 2000, columns 3024 to 3032: file offset 340480 + 16 * 1344336 = 21849856. Bytes: 0x9C 0x06 0xDC 0xA1 0x6A 0xD4 0x1D 0xC1 0x06 0xDB 0x31 0x69 0x74 0x1D 0xA5 0x06 -> 1692 1904 1706 1909 1729 1900 1683 1885 1701.
- S21 (V7), row 1000, columns 1560 to 1569: file offset 1536 + 16 * 313156 = 5012032. Bytes: 0xBF 0x63 0x81 0xD5 0x53 0x7A 0xC8 0x73 0x76 0xBA 0xA3 0x78 0xCD 0x83 0x77 0x00 -> 959 2070 981 1957 968 1895 954 1930 973 1912.

### TV-7: V5 blocks

- Synthetic, classes 0, 1 and 3: 0xC0 0xEB 0xFF 0x20 0x8C 0xE5 0xFF 0x00 0x04 0x20 0x7D 0x08 0x82 0x32 0x22 0x4E. Fields: 5000, 9000. Zone 0 class 0, codes 520 500 512. Zone 1 class 1, codes 0 1023 600. Zone 2 class 3, codes 32 1023 700. Output: 5000 9000 5008 8988 5008 7964 6030 8140 512 16368 11200. Check: pixel 5 = 8988 - 1024 + 0 = 7964. Pixel 6 = 5008 - 1024 + 2046 = 6030.
- Synthetic, class 2 with the offset reset: 0x00 0xE0 0xFF 0x00 0x00 0x20 0x80 0x00 0x42 0x38 0x96 0x64 0x88 0xBB 0x70 0x17. Fields: 1500, 3000. Zone 0 class 2, codes 100 600 900. Zones 1 and 2 class 0, codes 512 512 512 and 0 1023 512. Output: 1500 3000 400 3352 3600 3352 3600 3352 3088 3863 3088. Check: pixel 2 = max(1500 - 2048, 0) + 400 = 400.
- S17 (DC-S1), row 2000, columns 3003 to 3013: block 1104273, file offset 519168 + 16 * 1104273 = 18187536. Bytes: 0xF0 0x5E 0x79 0xEB 0x41 0x20 0x98 0xF6 0xA1 0x96 0x7F 0x53 0x82 0x3F 0xF0 0x18. Fields: 1596, 1016, classes 0 0 0, codes 595 510 362 502 608 516 491 485 495. Output: 1596 1016 1679 1014 1529 1004 1625 1008 1604 981 1587.

### TV-8: V8 samples

Bytes 0xB0 0x46 -> 18096 -> 1131. Bytes 0x70 0x84 -> 33904 -> 2119. Bytes 0xF0 0xFF -> 65520 -> 4095.

### TV-9: levels, white balance and matrix

V1, BlackLevel tags 128 (b = 143), LinearityLimit 4095 (w = 4095), WB tags 600 300 450, Model DC-GH5, CFA cell R 1131, G 2119, G 2119, B 1131. Normalized and balanced: 0.5, 0.5, 0.375. Output linear sRGB for a flat area of this cell: 0.51125, 0.5470297, 0.31.
V3, BlackLevel tags 497 (b = 512), w = 16383, same WB and Model, cell R 4480, G 8448, B 4480: normalized 0.5000315, 0.5000315, 0.3750236. Output 0.5112822, 0.5470642, 0.3100195.
V7 or V8, BlackLevel tags 0 (b = 0), w = 4095, cell R 1131, G 2119, B 1131: normalized 0.5523810, 0.5174603, 0.4142857. Output 0.5965873, 0.5486720, 0.3637778.

### TV-10: orientation

Stored 3 x 2 image with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a.

### Fixture recipes

The fixtures are complete small files for the decoder tests. All multi-byte numbers are little-endian. Tags are in ascending order. Every fixture has a flat CFA image: every red pixel has the value R, every green pixel G, every blue pixel B, with CFAPattern 1. After bilinear demosaicing every output pixel of a flat image has the same three values. The expected outputs are in TV-9. The output size is W x H (borders equal the whole raster, orientation 1).

Common part of F1 to F6 (RW2): bytes 0 to 7 are 49 49 55 00 18 00 00 00. Bytes 8 to 23 are 0. IFD0 at offset 24 has 22 entries, all SHORT with count 1 except where noted: 0x0002 W, 0x0003 H, 0x0004 0, 0x0005 0, 0x0006 H, 0x0007 W, 0x0009 1, 0x000A D, 0x000B 34316, 0x000E F, 0x000F F, 0x0010 F, 0x001C K, 0x001D K, 0x001E K, 0x0024 600, 0x0025 300, 0x0026 450, 0x002D RawFormat, 0x0110 ASCII count 7 at offset 0x126 ("DC-GH5" and a zero byte), 0x0112 1, 0x0118 LONG 512. The next-IFD offset 0 is at 0x122. Bytes 0x126 to 0x12D hold 44 43 2D 47 48 35 00 00. Bytes up to 511 are 0. The sensor data start at 512.

F1 (V1, W = 28, H = 2, D = 12, K = 128, RawFormat 4), complete header, IFD0 and Model string (bytes 0x000 to 0x12F):

0000: 49 49 55 00 18 00 00 00 00 00 00 00 00 00 00 00
0010: 00 00 00 00 00 00 00 00 16 00 02 00 03 00 01 00
0020: 00 00 1C 00 00 00 03 00 03 00 01 00 00 00 02 00
0030: 00 00 04 00 03 00 01 00 00 00 00 00 00 00 05 00
0040: 03 00 01 00 00 00 00 00 00 00 06 00 03 00 01 00
0050: 00 00 02 00 00 00 07 00 03 00 01 00 00 00 1C 00
0060: 00 00 09 00 03 00 01 00 00 00 01 00 00 00 0A 00
0070: 03 00 01 00 00 00 0C 00 00 00 0B 00 03 00 01 00
0080: 00 00 0C 86 00 00 0E 00 03 00 01 00 00 00 FF 0F
0090: 00 00 0F 00 03 00 01 00 00 00 FF 0F 00 00 10 00
00A0: 03 00 01 00 00 00 FF 0F 00 00 1C 00 03 00 01 00
00B0: 00 00 80 00 00 00 1D 00 03 00 01 00 00 00 80 00
00C0: 00 00 1E 00 03 00 01 00 00 00 80 00 00 00 24 00
00D0: 03 00 01 00 00 00 58 02 00 00 25 00 03 00 01 00
00E0: 00 00 2C 01 00 00 26 00 03 00 01 00 00 00 C2 01
00F0: 00 00 2D 00 03 00 01 00 00 00 04 00 00 00 10 01
0100: 02 00 07 00 00 00 26 01 00 00 12 01 03 00 01 00
0110: 00 00 01 00 00 00 18 01 04 00 01 00 00 00 00 02
0120: 00 00 00 00 00 00 44 43 2D 47 48 35 00 00 00 00

F1 data: one chunk of 16384 bytes at offsets 512 to 16895 (file length 16896). The logical stream has 4 blocks (64 bytes) and the rest of the chunk is 0. By R-RW2-21 the logical bytes 0 to 63 are at file offsets 8696 to 8759 (512 + 8184 = 0x21F8):

21F8: 80 80 80 00 02 02 02 08 08 08 20 20 20 47 B8 46
2208: 80 80 80 00 02 02 02 08 08 08 20 20 20 47 B8 46
2218: 80 80 80 00 02 02 02 08 08 08 20 20 20 6B 74 84
2228: 80 80 80 00 02 02 02 08 08 08 20 20 20 6B 74 84

Each block has the initial values (1131, 2119) in row 0 and (2119, 1131) in row 1, all classes 0 and all codes 128, so every pixel equals its initial value: R = 1131, G = 2119, B = 1131.

F5 (V4, W = 28, H = 2, D = 12, K = 128, RawFormat 6): the IFD0 of F1 with RawFormat 6 (byte 0x0FA = 06). The 64 bytes of the four F1 blocks are stored in order without rotation at offsets 512 to 575. File length 576.

Byte positions of the variable values in the IFD0 of F1 to F6: W at 0x022 and 0x05E, H at 0x02E and 0x052, D at 0x076, F at 0x08E, 0x09A and 0x0A6 (two bytes each), K at 0x0B2, 0x0BE and 0x0CA (two bytes each), RawFormat at 0x0FA.

F3 (V2, W = 20, H = 2, D = 12, K = 128, RawFormat 5): the IFD0 of F1 with W = 20 (bytes 0x022 and 0x05E = 14), RawFormat 5 (byte 0x0FA = 05). Data: one chunk of 16384 bytes, logical bytes at offsets 8696 to 8759: two blocks 6B 74 84 6B 74 84 6B 74 84 6B 74 84 6B 74 84 00 (row 0: 1131, 2119 five times), then two blocks 47 B8 46 47 B8 46 47 B8 46 47 B8 46 47 B8 46 00 (row 1: 2119, 1131 five times). File length 16896.

F2 (V3, W = 18, H = 2, D = 14, K = 497, RawFormat 5): the IFD0 of F1 with W = 18 (byte 12 hex), D = 14 (0E), F = 16383 (FF 3F), K = 497 (F1 01), RawFormat 5 (05). Values R = 4480, G = 8448, B = 4480. Data: one chunk, logical bytes at offsets 8696 to 8759:
- block 0 (row 0, pixels 0 to 8: R G R G R G R G R): 80 11 40 08 18 01 84 80 11 40 08 18 01 84 80 11
- block 1 (row 0, pixels 9 to 17: G R G R G R G R G): 00 21 60 04 10 02 46 00 21 60 04 10 02 46 00 21
- block 2 (row 1, pixels 0 to 8: G B G B G B G B G): 00 21 60 04 10 02 46 00 21 60 04 10 02 46 00 21
- block 3 (row 1, pixels 9 to 17: B G B G B G B G B): 80 11 40 08 18 01 84 80 11 40 08 18 01 84 80 11
File length 16896.

F6 (V6, W = 18, H = 2, D = 14, K = 497, RawFormat 7): the IFD0 of F2 with RawFormat 7. The four F2 blocks in order without rotation at offsets 512 to 575. File length 576.

F4 (V5, W = 22, H = 2, D = 14, K = 497, RawFormat 6): the IFD0 of F2 with W = 22 (byte 16 hex) and RawFormat 6. Data without rotation at offsets 512 to 575:
- block 0 (row 0, pixels 0 to 10, initial values 4480 and 8448): 00 20 80 00 02 20 80 00 02 20 80 00 02 10 02 46
- block 1 (row 0, pixels 11 to 21, initial values 8448 and 4480): 00 20 80 00 02 20 80 00 02 20 80 00 02 18 01 84
- block 2 (row 1, pixels 0 to 10, initial values 8448 and 4480): 00 20 80 00 02 20 80 00 02 20 80 00 02 18 01 84
- block 3 (row 1, pixels 11 to 21, initial values 4480 and 8448): 00 20 80 00 02 20 80 00 02 20 80 00 02 10 02 46
All classes are 0 and all codes are 512. File length 576.

F7 (V7, RAW, W = 20, H = 2, D = 12, K = 0, RawFormat 2): bytes 0 to 7 are 49 49 55 00 08 00 00 00. IFD0 at offset 8 with 23 entries: the 22 entries of F1, except W = 20, K = 0, RawFormat 2, and tag 0x0111 (LONG, value 512) in place of tag 0x0118, plus tag 0x0117 (LONG, value 64). Both sort between 0x0110 and 0x0112. The next-IFD offset 0 is at 0x11E and the Model string at 0x122. Data: the four F3 blocks in order without rotation at offsets 512 to 575. File length 576.

F8 (V8, RAW, W = 4, H = 2, D = 12, K = 0): as F7 but with 22 entries: W = 4, Compression 34828 (0C 88), no tag 0x002D, StripByteCounts (0x0117) 16. The next-IFD offset 0 is at 0x112 and the Model string at 0x116. Data at offset 512, 16 bytes: B0 46 70 84 B0 46 70 84 70 84 B0 46 70 84 B0 46 (row 0: R G R G, row 1: G B G B). File length 528.

Expected output of every pixel (TV-9): F1, F3 and F5: 0.51125, 0.5470297, 0.31. F2, F4 and F6: 0.5112822, 0.5470642, 0.3100195. F7 and F8: 0.5965873, 0.5486720, 0.3637778. The fixtures were built and decoded with the measurement tools and gave a flat CFA image with exactly these R, G and B values.

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 RawFormat 8 (DC-GH6, GH7, G9M2, S1M2, S1M2ES, S1RM2, S5M2, S5M2X, S9). The data length per pixel varies between files (M14), so the coding is variable-length. Tags 0x0039 to 0x0048 hold binary parameters. No permitted source describes the coding. Fallback: preview path (X1). These files carry a full-size preview in tag 0x0127 (S20).
- OQ-2 Black level offset. The offset +15 is measured on 12-bit files of formats 4, 5 and 6 with nonzero tags (M10). For 14-bit files the chromaticity sweeps are flat. The darkest 0.01 percent of the values sit at tag + 16 to tag + 18 (S15, S17), which supports +15 and makes +60 (15 units on the 12-bit scale) unlikely. Files with the tag value 0 (older V1 models) have flat sweep minima at +24 to +27. Fallback: R-RW2-32 as written.
- OQ-3 Black level of V7. S21 has the tag value 0 and no preview. Its darkest 0.01 percent of values are at 16. Fallback: the tag value (R-RW2-32).
- OQ-4 White level at extended ISO settings. LinearityLimit is lower than the data in S23 GM1S and FZ2000 (M11). The real saturation point of such files was not measured. Fallback: R-RW2-31.
- OQ-5 RawFormat 7 with 12 bits, RawFormat values 1, 3 and others, and RAW files with Compression 34826 or 34830 were not observed. Fallback: preview path (X2, X3).
- OQ-6 Chunk rotation for other chunk sizes. All measured V1 to V3 files use 16384-byte chunks and the split 8184. Fallback: R-RW2-21 for every V1 to V3 file.
- OQ-7 Rounding inside the block code. The camera rounds codes (S4), so values above F by 1 or 2 occur inside the active area (M6). Whether the camera limits the predecessor before it codes the next pixel is not known. Fallback: R-RW2-28 (limit before use). The difference is at most 2 units in the measured files.
- OQ-8 Orientation of the preview pixels for rotated shots. All samples have Orientation 1. Fallback: R-RW2-48.
- OQ-9 Color matrices. S6 gives D50 matrices with 2 decimals for 23 models and nothing for others (for example DC-G9, DC-GH5S, DC-G100, DC-GX9, DC-S1H, DMC-G7). The generic matrix makes the colors of S16 (DC-G9, high resolution secondary file) worse than the identity matrix (M8). The D50 matrix is used for every scene illuminant. Fallback: R-RW2-36 with the generic matrix.
- OQ-10 Crop tags 0x002F to 0x0032. They mark the aspect ratio chosen in the camera (for example 1:1 inside a 4:3 sensor). The embedded preview ignores them (M13). Fallback: R-RW2-17 (not applied).
- OQ-11 Leica RWL files. S3 states that the same tags occur. No RWL file was measured. Fallback: classification by R-RW2-3.
- OQ-12 Multishot (pixel shift) files with RawFormat 5, 6 or 7 have large rasters (DC-G9: 10480 x 7794, M1). Only the G9 secondary file was decoded. Fallback: the normal rules of the variant and the size limits of R-RW2-53.
- OQ-13 Meaning of tags 0x0001, 0x0008, 0x000D, 0x0013, 0x0017 to 0x001B, 0x0027, 0x0029 to 0x002C, 0x0033 to 0x0038, 0x0119, 0x011A to 0x011F (S3 names some of them: noise reduction, distortion, gamma, WB presets). Fallback: ignored.
- OQ-14 Lens distortion correction (tag 0x0119, S3 DistortionInfo) and the parameter semantics are not documented. Fallback: no lens correction.
- OQ-15 Highlights. White-balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-RW2-35.
- OQ-16 Previews of RAW files. No JPEG was found in IFD0 of S21 and S22. Other locations (maker note) were not examined. Fallback: R-RW2-66.
- OQ-17 CFAPattern 2 was not observed in the decoded samples (the survey shows it in DMC-G3, GF6, GX1, LX7). Fallback: the table of R-RW2-18.
