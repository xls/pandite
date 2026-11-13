# Samsung SRW raw format: decoder specification

Document: specs/raw/srw.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite SRW decoder (clean-room step 1 of 2).

This document describes the Samsung SRW raw file format as facts. It is written for an implementer who has this document, TIFF 6.0 (S1) and the Sony specification specs/raw/arw.md of this repository. This document names the parts of arw.md that it reuses. Every normative statement traces to a source in the Sources section or to a measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Conventions

- Requirements have the form R-SRW-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are hexadecimal bytes in file order, for example 0x4D 0x4D or "4D 4D".
- Bit 0 is the least significant bit of a value.
- floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0).
- Pixel coordinates are (x, y). x is the column and y is the row. (0, 0) is the first stored pixel, at the top-left of the stored raster.
- W and H are the stored width and height of the raw raster (tags 0x0100 and 0x0101 of the raw IFD).
- The signed value of a 32-bit field u is u when u < 2^31, and u - 2^32 otherwise (two's complement).

### Glossary

- SRW: the raw file format of Samsung NX system cameras and of some Samsung compact cameras (S6). An SRW file is a TIFF file.
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2).
- IFD0, IFD1: the first and second IFD of the main IFD chain.
- SubIFD: an IFD whose offset is in tag 0x014A (SubIFDs) of an IFD of the main chain.
- Raw IFD: the SubIFD that describes the sensor data (R-SRW-12).
- Exif IFD: the IFD at the offset in IFD0 tag 0x8769 (S2).
- Maker note: the value of Exif tag 0x927C (MakerNote, S2). In SRW it holds an IFD (R-SRW-13).
- P: the file offset of the first byte of the maker note.
- Key: the 11 values of maker-note tag 0xA020 (EncryptionKey, S4).
- Encrypted tag: a maker-note tag whose stored values differ from the real values by key values (section Metadata tags).
- CFA: color filter array. In all measured SRW files it is a 2x2 Bayer pattern (S9).
- CFA position: the index 2 * (y mod 2) + (x mod 2) of a pixel inside its 2x2 repeat cell (0, 1, 2 or 3).
- Valid area: the rectangle of the raw raster that holds the image. Maker-note tag 0xA010 (SensorAreas) gives it.
- Variant: one storage method of the sensor data. This document defines two variants, V1 and V2, and five out-of-scope classes, X1 to X5.
- Raw data byte order: the byte order of the sensor data. It can differ from the byte order of the TIFF structure (R-SRW-25).
- 24-bit group: three consecutive bytes of V2 sensor data that hold two 12-bit pixels.
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S7). The decoder output is linear sRGB.
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (copy of the specification PDF in the documentation folder of the libtiff project, 121 pages) | https://download.osgeo.org/libtiff/doc/TIFF6.pdf | official | 2026-11-14 | Container layout (header, IFD, field types, value offsets), Metadata tags, Sensor data layout (strips), Decoding procedure (orientation) |
| S2 | Exif Version 2.32, CIPA DC-X008-Translation-2019 | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-14 | Container layout (Exif IFD, MakerNote tag 0x927C), Sensor data layout (CFA color codes of Table 14) |
| S3 | Digital Negative (DNG) Specification 1.6.0.0, Adobe (copy read at the URL) | https://paulbourke.net/dataformats/dng/dng_spec_1_6_0_0.pdf | official | 2026-11-14 | Sensor data layout (CFARepeatPatternDim 0x828D, CFAPattern 0x828E, origin of the pattern) |
| S4 | ExifTool Tag Names: Samsung tags table, section "Samsung Type2 Tags" (metadata tag reference documentation) | https://exiftool.org/TagNames/Samsung.html | community documentation | 2026-11-14 | Metadata tags (names and types of the maker-note tags, "key used to decrypt the tags below", "full and valid sensor areas", RawDataByteOrder values, WB_RGGBLevelsUncorrected "not corrected for WB_RGGBLevelsBlack", tone curve layout), Scope and variants |
| S5 | ExifTool Tag Names: EXIF tags table, "EXIF Compression Values" (metadata tag reference documentation) | https://exiftool.org/TagNames/EXIF.html | community documentation | 2026-11-14 | Scope and variants (32769 Packed RAW, 32770 Samsung SRW Compressed, 32772 Samsung SRW Compressed 2) |
| S6 | Samsung NX500 User Manual, page on file formats (vendor manual, copy hosted by ManualsLib) | https://www.manualslib.com/manual/956406/Samsung-Nx500.html?page=80 | vendor | 2026-11-14 | Glossary, Scope and variants (RAW files are stored without conversion to JPEG and have the extension SRW) |
| S7 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (output primaries and D65 white), measurement tools (sRGB transfer function for the JPEG comparison) |
| S8 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear interpolation, non-adaptive) |
| S9 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-14 | Sensor data layout (Bayer arrangement) |
| S10 | Measurement: NX10 sample SAM_3630.SRW, public domain (CC0) | https://raw.pixls.us/data/Samsung/NX10/SAM_3630.SRW | measurement | 2026-11-14 | all sections, records M1 to M12 |
| S11 | Measurement: NX210 sample nx210.SRW (CC0) | https://raw.pixls.us/data/Samsung/NX210/nx210.SRW | measurement | 2026-11-14 | Compression (V2 little-endian), Levels, Color conversion, records M2, M3, M6 to M12 |
| S12 | Measurement: EX1 sample SAM_5548.SRW (CC0) | https://raw.pixls.us/data/Samsung/EX1/SAM_5548.SRW | measurement | 2026-11-14 | Compression (V1), Levels (white level), records M4, M6 to M12 |
| S13 | Measurement: WB2000 sample SAM_4727.SRW (CC0) | https://raw.pixls.us/data/Samsung/WB2000/SAM_4727.SRW | measurement | 2026-11-14 | Compression (V1), Levels (white level), records M4, M6 to M12 |
| S14 | Measurement: NX300 sample nx300.SRW and NX30 sample 2015-03-07-163604_sam_7204.srw (CC0) | https://raw.pixls.us/data/Samsung/NX300/nx300.SRW | measurement | 2026-11-14 | Scope and variants (X1), Embedded preview, records M11, M13 |
| S15 | Measurement: NX3000 sample _3184416.SRW (CC0) | https://raw.pixls.us/data/Samsung/NX3000/_3184416.SRW | measurement | 2026-11-14 | Scope and variants (X2), Container layout (little-endian files), records M1, M11, M13 |
| S16 | Measurement: NX500 sample SAM_2927.SRW and NX1 sample 2016-07-23-142101_sam_9364.srw (CC0) | https://raw.pixls.us/data/Samsung/NX500/SAM_2927.SRW | measurement | 2026-11-14 | Scope and variants (X3), Embedded preview, records M11, M13 |
| S17 | Measurement: container survey of all 24 CC0 SRW files of the archive (first 1 MiB of each file, 4 MiB for the NX30), models EK-GN120, EX1, NX1, NX5, NX10, NX11, NX20, NX30, NX200, NX210, NX300, NX500 (4 files), NX1000, NX1100 (3 files), NX2000, NX3000, NX3300, NX mini, WB2000, plus 200 rows from the middle of the strip of the 10 V2 files | https://raw.pixls.us/data/Samsung/ | measurement | 2026-11-14 | Scope and variants, Container layout, Metadata tags, records M1 to M3, M5, M9, M13 |
| S18 | Measurement: comparison of the stored maker-note values of the 24 files of S17 with the values that the ExifTool 13.63 Windows executable prints for them (the executable was used only to print tag values, and its source modules were not opened) | https://exiftool.org/ | measurement | 2026-11-14 | Metadata tags (decryption rule), record M5 |

Sample files are from the public raw sample archive raw.pixls.us. The archive lists the license of each file. All files used here are marked CC0 (public domain). The archive also holds NX100, NX300M and EX2F samples under CC BY-NC-SA. They were not used. The author inspected the files with measurement tools that the author wrote from S1 and S2 for this purpose. The tools are not part of the deliverable. One comparison decoded the camera's own embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S17): 21 of 24 files start with 4D 4D 00 2A 00 00 00 08 (big-endian, IFD0 at offset 8). The NX3000, NX3300 and NX mini files start with 49 49 2A 00 08 00 00 00 (little-endian). IFD0 holds Make, Model, Orientation, DateTime (0x0132), SubIFDs and the Exif IFD pointer. IFD0 entries are not always in ascending tag order (0x8769 precedes 0x014A in the NX300 and NX3000 files), and the NX3000 IFD0 holds one all-zero entry (tag 0, type 0, count 0). The SubIFDs tag of IFD0 holds two offsets: the first SubIFD holds the full-size JPEG preview (tags 0x0201 and 0x0202, no tag 0x0100), the second SubIFD is the raw IFD. IFD1 holds one entry, a SubIFDs tag with one offset to a SubIFD with the thumbnail JPEG. The raw IFD has no PhotometricInterpretation tag. Its SamplesPerPixel is 1 (NX5, NX10, NX11) or 3 (all other models), although every variant stores one sample per pixel. The Make value is "SAMSUNG", padded with spaces in the NX5, NX10 and NX11 files. In the NX300 file the Make value has non-zero bytes after the terminating zero, and in the NX2000 file the Model value has them.
- M2 (V2 bit packing, S10, S11, S17): mean absolute difference between pixels two columns apart. NX10 (whole raster, every 7th row): 42.6 with the big-endian group reading of R-SRW-28 and 325.7 with the little-endian reading of R-SRW-27. NX210: 296.8 big-endian and 32.8 little-endian. On 200 middle rows, big-endian reading first and little-endian reading second: NX5 7 and 81, NX11 34 and 311, NX20 236 and 29, NX200 235 and 27, NX1000 458 and 55, NX1100 (3 files) 272 to 422 and 35 to 55. Two other byte assignments (nibble-swapped) give 674 to 985 in the NX10 and NX210 files.
- M3 (raw data byte order, S17): maker-note tag 0x0040 is absent in the NX5, NX10 and NX11 files, which use the big-endian reading (M2). It is present with value 0 in the NX20, NX200, NX210, NX1000 and NX1100 files, which use the little-endian reading, and in the EX1 and WB2000 files (M4). All files without the tag have a big-endian TIFF header. Value 1 was not observed.
- M4 (V1, S12, S13): StripByteCounts equals 2 * W * H. Read as little-endian 16-bit samples, the mean absolute difference between pixels two columns apart is 154.6 (EX1) and 137.9 (WB2000). Read as big-endian it is 20551 and 11507. The EX1 samples have non-zero low 2 bits in 75 percent of the pixels (14-bit data). The WB2000 samples in the valid area are 4095 or less, although its BitsPerSample is 14. Both files carry maker-note tag 0x0040 with value 0.
- M5 (maker-note decryption, S18): in all 24 files the maker-note IFD starts at the first byte of the MakerNote value, uses the byte order of the TIFF header, and its value offsets count from that first byte (P). The key (tag 0xA020) is 305 72 737 456 282 307 519 724 13 505 193 in all 24 files. For every file and every tag of the table in section Metadata tags, the printed value equals the stored signed value plus the sign times the key value with the start index of that table, for every element. Tags 0x0040, 0x0050, 0xA010, 0xA020 and 0xA025 print their stored values.
- M6 (CFA pattern and origin, S10 to S13): half-resolution developments with each of the four Bayer patterns, compared with the camera JPEG (tone curve of tag 0xA042 inverted, mean chromaticity error over 64x64 blocks). NX10 (tag pattern 2 1 1 0): 0.0113 with the tag pattern, 0.40 to 0.61 with the other patterns. NX210 (1 0 2 1): 0.0442 against 0.37 to 0.43. EX1 (0 1 1 2): 0.0185 against 0.054 to 0.41. WB2000 (0 1 1 2): 0.0477 against 0.26 to 0.42. The NX10 valid area starts at the odd column 21, so the pattern origin is the stored pixel (0, 0), not the valid-area origin. Maker-note tag 0x0050 is 65535 (NX10) or 1 (NX210, EX1, WB2000) and does not change the pattern.
- M7 (color matrix orientation, S10 to S13): same comparison with the matrix of tag 0xA031 read with rows as output channels, with the identity, and with the transposed matrix. NX10: 0.0113, 0.0797, 0.2233. NX210: 0.0442, 0.0261, 0.3013. EX1: 0.0185, 0.0118, 0.3401. WB2000: 0.0477, 0.0448, 0.3118. A least-squares fit from white-balanced camera RGB to the NX10 JPEG gives the rows 1.62 -0.44 -0.18, -0.16 1.42 -0.26, 0.16 -0.84 1.68 after normalization, close to the tag matrix 1.59 -0.45 -0.15, -0.13 1.36 -0.23, -0.03 -0.41 1.44. The NX210 file has custom picture settings (maker-note PictureWizard mode 255, contrast -4), and its fitted matrix is less saturated than the tag matrix. The EX1 and WB2000 scenes are near neutral (snow), and their fits are not stable. In all four files the transposed matrix is the worst choice. Tag 0xA030 equals tag 0xA031 in all 12 in-scope files.
- M8 (white balance, S10 to S13): same comparison with the multipliers of R-SRW-33, their inverses, and no multipliers. NX10: 0.0113, 0.2785, 0.2025. NX210: 0.0442, 0.7632, 0.4571. EX1: 0.0185, 0.8119, 0.5686. WB2000: 0.0477, 0.6809, 0.3428. Tags 0xA021 and 0xA022 hold the same values in all 24 files, also in the NX2000 file, which has Exif WhiteBalance 1 (manual).
- M9 (black level, S10 to S12, S17): tag 0xA028 gives 3 3 3 3 (NX10), 3 2 2 3 (NX11), 2 2 2 2 (NX5) and 0 0 0 0 in the other in-scope files. Dark margins: NX10 columns 0 to 7 and 4696 to 4703 have means 3 to 6. EX1 rows 2762 to 2769 have means 1 to 3. NX210 columns 0 to 41 are exactly 0. The comparison of M6 with black levels 0, 64 and 128 in place of the tag values gives NX10 0.0120, 0.0909, 0.1715 (tag 0.0113).
- M10 (white level, S10 to S13, S17): pixels at the largest value of the valid area: NX210 51 pixels at 4095, and in the 200-row excerpts NX20 400, NX200 5105, NX1000 1190, NX1100 325 to 348 pixels at 4095. WB2000: 169058 pixels at 4095 (BitsPerSample 14). EX1: 89194 pixels at 15881 and 85786 at 15882 (15881 in even columns, 15882 in odd columns), no larger value. NX10 (whole raster) and the 200-row excerpts of NX5 and NX11: no saturated pixels (largest values 3742, 1374 and 1527). Decrypted tag 0xA048 is 0 0 0 0 followed by eight equal values: 4095 in the NX5, NX10, NX11, NX20, NX200, NX210, NX1000, NX1100 and WB2000 files, and 16383 in the EX1 file.
- M11 (valid area, S10 to S16): the valid rectangle of tag 0xA010 has the size of the full-size JPEG in NX10 (4592 x 3056), NX210 and NX1100 (5472 x 3648) and EX1 (3648 x 2736). The WB2000 valid rectangle is 3648 x 2736 and its JPEG is a centered 3648 x 2048 crop (16:9). Cross-correlation at the image center places the JPEG origin at raw (20.5, 12.5) for NX10 (tag 21, 14), (88.5, 17.5) for NX210 (tag 78, 16), (13.5, 17.5) for EX1 (tag 16, 16) and (36.5, 365.5) for WB2000 (tag 36, 22 + 344). The NX10, EX1 and WB2000 JPEGs are enlarged by 0.7 to 1.7 percent toward the edges against the raw data (lens correction in the camera). The NX210 JPEG has the same offset at all five measured positions. Outside the valid rectangle the rasters hold zero columns (NX210 columns 0 to 41), dark columns, and junk: EX1 columns 3684 to 3687 are 65535 and its last 10 rows are 15881, WB2000 columns 3720 to 3727 are near 63500 and its last row is near 64300.
- M12 (pipeline check, S10 to S13): the complete decoding of section Decoding procedure, compared with the full-size JPEG after inversion of the camera tone curve (tag 0xA042), gives a mean chromaticity error of 0.0112 (NX10), 0.0442 (NX210), 0.0184 (EX1) and 0.0480 (WB2000), and a JPEG to raw brightness ratio of 1.127, 0.967, 0.928 and 1.002. The scenes and their colors agree with the JPEG on visual inspection.
- M13 (previews and out-of-scope structures, S14 to S17): the full-size JPEGs are 4592 x 3056 (NX10), 5472 x 3648 (NX210, NX300, NX30, NX3000), 3648 x 2736 (EX1), 3648 x 2048 (WB2000) and 6480 x 4320 (NX500, NX1). The IFD1 thumbnails are 160 x 120 (320 x 216 for NX500 and NX1). All are baseline JPEG (SOF0, 3 components) and start with FF D8 FF DB. The NX210 and NX300 full-size streams end with 392 and 45 bytes after the EOI marker. All decode with Pillow. Maker-note tag 0x0035 points to an IFD with a further JPEG of 640 x 424 (NX10, NX210) or 1616 x 1080 (NX300), with offsets that count from P. X1 files: Compression 32770, StripByteCounts / (W * H) = 0.877 to 1.071, raw IFD tags 0xA010 and 0xA011 with 0xA011 = 4 * H and 0xA010 + 0xA011 = StripOffsets (in the NX300 file the 4 * H bytes before the strip are 3714 increasing little-endian 32-bit values, from 16 to 18528664). X2 files: Compression 32772, little-endian TIFF, tags 0xA010 = 0xA011 = 0, RowsPerStrip = W. X3 files: Compression 32773, StripByteCounts / (W * H) = 1.128 (NX1, 14 bits) and 0.587 to 1.013 (NX500, 12 and 14 bits). Read as TIFF PackBits (S1 section 9), the NX500 strip expands to 31231886 bytes and the NX1 strip to 60497410 bytes, and both runs pass the end of the strip. Neither length equals 1.5, 1.75 or 2 times W * H, so the data is not PackBits.

### Sources encountered and not used

A web search result summary for the query "Samsung SRW raw file format compression description structure" described the content of a decoder source file (rawloader srw.rs on docs.rs) and mentioned Samsung converter source code. The author did not open the page and used nothing from the summary. The variants of this document come from S4, S5 and the measurements. A search result titled "NX1 raw compression algorithm" (dpreview forum thread 3764646) was not opened.

## Scope and variants

### Models and variants

S6 states that RAW files are stored without conversion to JPEG and have the extension SRW. S5 names the Compression values 32769 "Packed RAW", 32770 "Samsung SRW Compressed" and 32772 "Samsung SRW Compressed 2". S5 lists 32773 as "PackBits", but the SRW files with this value do not hold PackBits data (M13). The measured files map to the variants below (M1 to M4, M13).

| Variant | Compression (0x0103) | BitsPerSample | Strip bytes | Sensor data | Measured models |
| --- | --- | --- | --- | --- | --- |
| V1 | 32769 | 14 | 2 * W * H | 16-bit samples, raw data byte order (R-SRW-25) | EX1, WB2000 |
| V2 | 32770 | 12 | 3 * W * H / 2 | 12-bit packed, two pixels in each 24-bit group | NX5, NX10, NX11 (big-endian groups), NX20, NX200, NX210, NX1000, NX1100 (little-endian groups) |

### Out-of-scope classes

No permitted source describes the compressed codings. The measurements did not establish them. The decoder uses the preview path for them.

| Id | Class | Detection from file contents | Measured models |
| --- | --- | --- | --- |
| X1 | Samsung compressed coding of Compression 32770 | Compression 32770 and total strip bytes not equal to 3 * W * H / 2 | NX300, NX30, NX2000, EK-GN120 |
| X2 | Samsung compressed coding of Compression 32772 | Compression 32772 | NX3000, NX3300, NX mini |
| X3 | Samsung compressed coding of Compression 32773 | Compression 32773 | NX1, NX500 (12 and 14 bits, two quality settings) |
| X4 | any other combination, for example Compression 32769 with fewer than 2 * W * H strip bytes, Compression 1 or 7, or an unexpected BitsPerSample | no rule of R-SRW-3 for V1 to X3 matches | none |
| X5 | file without a raw IFD | R-SRW-12 finds no raw IFD | none |

### Requirements

- R-SRW-1: The decoder must decode the sensor data of variants V1 and V2 as this document specifies.
- R-SRW-2: The decoder must use the preview path (section Embedded preview) for X1, X2, X3, X4 and X5 files, and must not interpret their sensor data.
- R-SRW-3: The decoder must classify the file by these rules, in this order:
  1. No raw IFD exists (R-SRW-12): X5.
  2. Compression 32769, BitsPerSample absent or 12 or 14, total strip bytes >= 2 * W * H: V1.
  3. Compression 32770, BitsPerSample absent or 12, W even, total strip bytes = 3 * W * H / 2: V2.
  4. Compression 32770 in any other case: X1.
  5. Compression 32772: X2.
  6. Compression 32773: X3.
  7. Any other case: X4.

"Total strip bytes" is the sum of all values of StripByteCounts (0x0117). BitsPerSample is the first value of tag 0x0102.

## Container layout

### Byte order and header

An SRW file is a TIFF file (S1 section 2). Both byte orders occur (M1).

| Offset | Size | Field | Value in SRW |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 4D 4D ("MM", big-endian) or 49 49 ("II", little-endian) |
| 2 | 2 | TIFF magic number | 42 (4D 4D files: 00 2A, 49 49 files: 2A 00) |
| 4 | 4 | offset of IFD0 | 8 in all measured files |

- R-SRW-4: The decoder must accept a file only if bytes 0 to 3 are 4D 4D 00 2A or 49 49 2A 00, and must report an error without a preview for any other start.
- R-SRW-5: The decoder must read every multi-byte TIFF field, including the fields of the maker note, in the byte order of the header.

### IFD structure

An IFD (S1 section 2) has a 2-byte entry count N, N entries of 12 bytes and a 4-byte offset of the next IFD. Each entry holds a 2-byte tag, a 2-byte field type, a 4-byte count (number of values, not bytes) and a 4-byte value or offset. The field types and sizes are those of S1 section 2 (1 BYTE 1, 2 ASCII 1, 3 SHORT 2, 4 LONG 4, 5 RATIONAL 8, 6 SBYTE 1, 7 UNDEFINED 1, 8 SSHORT 2, 9 SLONG 4, 10 SRATIONAL 8, 11 FLOAT 4, 12 DOUBLE 8). The table in arw.md section "Container layout" lists them with type 13 (IFD, 4 bytes). The value bytes occupy count * size bytes. If that total is 4 or less, the value is in the value field, starting at its first byte. Otherwise the value field holds the offset of the value bytes.

- R-SRW-6: The decoder must read a value from the value field when count * size <= 4, and from the offset in the value field otherwise. For a maker-note entry the decoder must add P to that offset (R-SRW-14).
- R-SRW-7: The decoder must find tags by tag number in any entry order, must use the first entry of a tag number, and must skip entries with a field type outside 1 to 13, for example the all-zero entry of M1.
- R-SRW-8: The decoder must accept SHORT, LONG and type 13 for integer tags that this document lists as "SHORT or LONG", BYTE or UNDEFINED for tag 0x828E, and LONG or SLONG for the maker-note tags of section Metadata tags.
- R-SRW-9: The decoder must compare ASCII values by their bytes before the first zero byte, after it removes trailing spaces.

### IFD chain and SubIFDs

Measured layout (M1, values from the NX10 file S10):

| Structure | Where | Content in SRW |
| --- | --- | --- |
| IFD0 | offset 8 | Make, Model, Orientation, DateTime, SubIFDs (two offsets), Exif IFD pointer 0x8769 |
| first SubIFD of IFD0 | first offset of IFD0 tag 0x014A | full-size JPEG preview (0x0201, 0x0202) |
| raw IFD | second offset of IFD0 tag 0x014A | sensor data and the tags of section Metadata tags |
| IFD1 | next-IFD offset of IFD0 | one entry: SubIFDs with one offset |
| SubIFD of IFD1 | IFD1 tag 0x014A | thumbnail JPEG (0x0201, 0x0202) |
| Exif IFD | IFD0 tag 0x8769 | Exif tags and the MakerNote (0x927C) |
| maker-note IFD | at P, the offset of the MakerNote value | Samsung tags, section Metadata tags |

Example (S10): IFD0 at 8 with 6 entries. Exif IFD at 146. Maker note at P = 432, 98179 bytes, 67 entries. IFD1 at 98794. Thumbnail SubIFD at 98812, JPEG at 98928 (8969 bytes). JPEG SubIFD at 107898, JPEG at 108144 (6413437 bytes). Raw IFD at 108004 with 11 entries. Sensor strip at 6521584 (22042944 bytes, to the end of the file).

- R-SRW-10: The decoder must read IFD0 at the offset in header bytes 4 to 7, must follow the next-IFD offsets to collect IFD1 and later IFDs, and must stop at offset 0, at an invalid IFD, at a repeated offset or after 16 IFDs.
- R-SRW-11: The decoder must treat an IFD as invalid when its offset lies outside the file, when N is 0 or greater than 1024, or when the 2 + 12 * N + 4 bytes of the IFD do not lie completely inside the file.
- R-SRW-12: The decoder must use as the raw IFD the first SubIFD of IFD0 (in the order of the offsets in tag 0x014A) that is a valid IFD and holds the tags 0x0100, 0x0101, 0x0103, 0x0111 and 0x0117.

### Maker note

The Exif IFD (S2) holds tag 0x927C MakerNote, type UNDEFINED. In SRW the value is an IFD without a header (M5). Its first two bytes are the entry count, for example 00 43 (67 entries) in S10. The maker-note IFD uses the byte order of the TIFF header. Its value offsets count from P, the file offset of the first byte of the MakerNote value. Example (S10): Exif entry 92 7C 00 07 00 01 7F 83 00 00 01 B0 gives type 7, count 98179 and P = 0x1B0 = 432. The maker-note entry A0 21 00 04 00 00 00 04 00 00 94 F4 gives tag 0xA021, type LONG, count 4, offset 0x94F4 = 38132, so its values are at 432 + 38132 = 38564.

- R-SRW-13: The decoder must locate the maker note as the value of tag 0x927C in the Exif IFD (IFD0 tag 0x8769), must read an IFD at P, and must treat the file as having no maker note when the Exif IFD or the maker-note IFD is invalid (R-SRW-11).
- R-SRW-14: The decoder must compute the file offset of a maker-note value that does not fit in the value field as P plus the stored offset.
- R-SRW-15: The decoder must ignore the raw IFD tags 0x00FE, 0x0115 (SamplesPerPixel) and 0x0106 (PhotometricInterpretation), because SamplesPerPixel is 3 in most raw IFDs although the data has one sample per pixel, and PhotometricInterpretation is absent (M1).

## Metadata tags

### Raw IFD and IFD0

"Required" means that the variant cannot be decoded without the tag. The decoder then uses the preview path. "Default" is the value to use when an optional tag is absent or invalid.

| Tag | Name | Type | Count | IFD | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0x0100 | ImageWidth | SHORT or LONG | 1 | raw | stored width W | required | none |
| 0x0101 | ImageLength | SHORT or LONG | 1 | raw | stored height H | required | none |
| 0x0102 | BitsPerSample | SHORT | 1 | raw | 12 (V2), 14 (V1, also for 12-bit data, M4) | optional | none |
| 0x0103 | Compression | SHORT or LONG | 1 | raw | 32769, 32770, 32772 or 32773 (S5) | required | none |
| 0x0111 | StripOffsets | SHORT or LONG | strips | raw | file offset of each strip | required | none |
| 0x0116 | RowsPerStrip | SHORT or LONG | 1 | raw | rows in each strip | optional | H |
| 0x0117 | StripByteCounts | SHORT or LONG | strips | raw | bytes in each strip | required | none |
| 0x828D | CFARepeatPatternDim | SHORT | 2 | raw | rows, columns of the CFA repeat (S3): 2, 2 | optional | 2, 2 |
| 0x828E | CFAPattern | BYTE | 4 | raw | colors of CFA positions 0 to 3, 0 = red, 1 = green, 2 = blue (S2 Table 14, S3) | optional | 0, 1, 1, 2 |
| 0xA010 | (no public name) | LONG | 1 | raw (X1 to X3) | 0xA010 + 0xA011 = StripOffsets in X1, detection aid only (M13) | ignored | none |
| 0xA011 | (no public name) | LONG | 1 | raw (X1 to X3) | 4 * H in X1, 0 in X2 and X3 (M13) | ignored | none |
| 0x0110 | Model | ASCII | any | IFD0 | model name, for example "NX10" | optional | empty |
| 0x0112 | Orientation | SHORT | 1 | IFD0 | 1 to 8 (S1) | optional | 1 |
| 0x014A | SubIFDs | LONG | 1 or more | IFD0, IFD1 | offsets of SubIFDs | required | none |
| 0x8769 | Exif IFD pointer | LONG | 1 | IFD0 | offset of the Exif IFD (S2) | required | none |
| 0x0201 | JPEGInterchangeFormat | LONG | 1 | SubIFDs | offset of a JPEG preview | optional | none |
| 0x0202 | JPEGInterchangeFormatLength | LONG | 1 | SubIFDs | length of that JPEG preview | optional | none |

Tags observed and not used: 0x00FE, 0x0106 (never present), 0x0115, 0x011A, 0x011B, 0x0128, 0x0132, 0x0213, 0xA101, 0xA102.

### Maker-note tags

Names are those of S4. "Enc" gives the sign s and the start index k0 of the decryption rule below. A dash means that the tag is not encrypted (M5).

| Tag | Name (S4) | Type | Count | Enc (s, k0) | Meaning | Use |
| --- | --- | --- | --- | --- | --- | --- |
| 0x0035 | PreviewIFD | LONG | 1 | - | offset (from P) of an IFD with a JPEG preview, offsets inside it also count from P (M13) | optional preview |
| 0x0040 | RawDataByteOrder | LONG | 1 | - | 0 = little-endian, 1 = big-endian (S4) | R-SRW-25 |
| 0x0050 | RawDataCFAPattern | LONG | 1 | - | 0 = Unchanged, 1 = Swap, 65535 = Roll (S4) | ignored (R-SRW-21) |
| 0xA010 | SensorAreas | LONG | 8 | - | "full and valid sensor areas" (S4): FX0, FY0, FX1, FY1, VX0, VY0, VX1, VY1 | valid area (R-SRW-22) |
| 0xA020 | EncryptionKey | LONG | 11 | - | "key used to decrypt the tags below" (S4) | key |
| 0xA021 | WB_RGGBLevelsUncorrected | LONG | 4 | -1, 0 | as-shot white balance levels R, G, G, B, "not corrected for WB_RGGBLevelsBlack" (S4) | R-SRW-33 |
| 0xA022 | WB_RGGBLevelsAuto | LONG | 4 | -1, 4 | equal to 0xA021 in all files (M8) | not used |
| 0xA023 | WB_RGGBLevelsIlluminator1 | LONG | 4 | -1, 8 | preset levels | not used |
| 0xA024 | WB_RGGBLevelsIlluminator2 | LONG | 4 | -1, 1 | preset levels | not used |
| 0xA025 | HighlightLinearityLimit | LONG | 1 | - | 3791 to 4575, meaning unknown | not used |
| 0xA028 | WB_RGGBLevelsBlack | SLONG | 4 | -1, 0 | black levels R, G, G, B | R-SRW-32 |
| 0xA030 | ColorMatrix | SLONG | 9 | +1, 0 | 3x3 matrix, scale 256 | R-SRW-35 |
| 0xA031 | ColorMatrixSRGB | SLONG | 9 | +1, 0 | 3x3 matrix to sRGB, scale 256 | R-SRW-35 |
| 0xA032 | ColorMatrixAdobeRGB | SLONG | 9 | +1, 0 | 3x3 matrix to Adobe RGB | not used |
| 0xA033, 0xA034 | CbCrMatrixDefault, CbCrMatrix | SLONG | 4 | +1, 0 and +1, 4 | camera JPEG processing | not used |
| 0xA035, 0xA036 | CbCrGainDefault, CbCrGain | LONG | 2 | -1, 0 and -1, 2 | camera JPEG processing | not used |
| 0xA040 to 0xA043 | ToneCurve tags | LONG | 23 | see below | value 0 = number of points n = 11, then 11 X and 11 Y values (S4) | not used |
| 0xA048 | RawData? | LONG | 12 | +1, 0 | 0 0 0 0, then eight equal values: the white level (M10) | R-SRW-31 |

The tone-curve tags use a split rule (M5): value 0 is not encrypted, values 1 to 11 use s = +1 and key index (i - 1) mod 11, values 12 to 22 use s = -1 and key index (i - 12) mod 11. The decoder does not need them. The measurements M6, M7 and M12 used tag 0xA042 only to compare with the camera JPEG.

### Decryption rule

Let S(i) be the signed value of stored element i (i = 0, 1, 2, ...) of an encrypted tag, K(0) to K(10) the 11 values of tag 0xA020, s the sign and k0 the start index of the table. The real value is V(i) = S(i) + s * K((k0 + i) mod 11) (M5).

Worked example (S10, big-endian maker note, key 305 72 737 456 282 307 519 724 13 505 193):
- Tag 0xA021 bytes 00 00 1C 4C 00 00 10 48 00 00 12 E1 00 00 12 37: S = 7244, 4168, 4833, 4663. With s = -1 and k0 = 0: 7244 - 305 = 6939, 4168 - 72 = 4096, 4833 - 737 = 4096, 4663 - 456 = 4207.
- Tag 0xA028 bytes 00 00 01 34 00 00 00 4B 00 00 02 E4 00 00 01 CB: S = 308, 75, 740, 459. V = 3, 3, 3, 3.
- Tag 0xA031 first bytes 00 00 00 67 FF FF FF 46: S(0) = 103, S(1) = 0xFFFFFF46 - 2^32 = -186. With s = +1: V(0) = 103 + 305 = 408, V(1) = -186 + 72 = -114.

- R-SRW-16: The decoder must use the preview path when a tag marked "required" for the classified variant is absent or has a count of 0, and when the maker note or its tag 0xA010 is absent (R-SRW-22).
- R-SRW-17: For V1 and V2, the decoder must accept a CFA pattern only if CFARepeatPatternDim is 2, 2 (or absent) and the four CFAPattern values contain exactly one 0, two 1 and one 2, and must otherwise use the preview path.
- R-SRW-18: The decoder must use H as RowsPerStrip when tag 0x0116 is absent, is 0 or is greater than H.
- R-SRW-19: The decoder must compute the real values of the tags 0xA021, 0xA028, 0xA030, 0xA031 and 0xA048 with the decryption rule and the (s, k0) of the table, and must treat these tags as absent when tag 0xA020 is absent or does not have exactly 11 values.
- R-SRW-20: The decoder must read each stored element of an encrypted tag as a signed 32-bit value (also for type LONG) and must compute V(i) without overflow, in at least 64-bit signed arithmetic.
- R-SRW-21: The decoder must ignore maker-note tag 0x0050 and must take the CFA pattern from raw IFD tag 0x828E alone (M6).

## Sensor data layout

### Raster and valid area

The raw raster has W columns and H rows. Tag 0xA010 holds eight values. S4 calls them the full and valid sensor areas. The values VX0, VY0, VX1, VY1 (elements 4 to 7) give the valid area: columns VX0 to VX1 - 1 and rows VY0 to VY1 - 1 of the stored raster, with the origin at stored pixel (0, 0). The valid area is the final image area. Its size equals the camera JPEG size (M11). The first four values (FX0, FY0, FX1, FY1) are not used. FX1 can exceed W (X1 files: 5632 for W = 5546).

The pixels outside the valid area are not image data. They hold zero columns, dark columns and junk values up to 65535 (M11). The decoder must not use them.

Measured dimensions (M1, M11):

| Models | Variant | W x H | Tag 0xA010 | Valid area |
| --- | --- | --- | --- | --- |
| NX5, NX10, NX11 | V2 | 4704 x 3124 | 0 0 4688 3124 21 14 4613 3070 | 4592 x 3056 |
| NX20, NX200, NX210, NX1000, NX1100 | V2 | 5632 x 3720 | 0 0 5582 3696 78 16 5550 3664 | 5472 x 3648 |
| EX1 | V1 | 3688 x 2780 | 0 0 3688 2780 16 16 3664 2752 | 3648 x 2736 |
| WB2000 | V1 | 3728 x 2781 | 0 0 3728 2780 36 22 3684 2758 | 3648 x 2736 |
| NX300, NX30, EK-GN120 (NX2000: W 5536) | X1 | 5546 x 3714 | 0 0 5632 3714 78 16 5526 3664 | 5448 x 3648 |
| NX3000, NX3300 | X2 | 5600 x 3714 | 0 0 5600 3714 96 42 5568 3690 | 5472 x 3648 |
| NX mini | X2 | 5664 x 3710 | 0 0 5664 3710 132 40 5604 3688 | 5472 x 3648 |
| NX1, NX500 | X3 | 6496 x 4336 | 0 0 6496 4336 0 0 6480 4320 | 6480 x 4320 |

- R-SRW-22: The decoder must take the valid area from elements 4 to 7 of maker-note tag 0xA010 (not encrypted), must require 0 <= VX0, VX0 + 2 <= VX1 <= W, 0 <= VY0 and VY0 + 2 <= VY1 <= H, and must otherwise use the preview path.

### CFA arrangement

The CFA pattern repeats every 2 rows and 2 columns (S9). CFAPattern lists the colors of CFA positions 0, 1, 2, 3, which are (x even, y even), (x odd, y even), (x even, y odd), (x odd, y odd). This is the row-major order of S2 (Figure 20, Table 14: value 0 red, 1 green, 2 blue). S3 places the pattern origin at the top-left of the active area. SRW has no ActiveArea tag, and M6 confirms the origin at stored pixel (0, 0), also when the valid area starts at an odd column (NX10: VX0 = 21). Four patterns occur: 2 1 1 0 (NX5, NX10, NX11), 1 0 2 1 (NX20, NX200, NX210, NX1000, NX1100, NX30, NX1, NX500), 0 1 1 2 (EX1, WB2000, NX300, NX2000, EK-GN120) and 1 2 0 1 (NX3000, NX3300, NX mini).

- R-SRW-23: The decoder must give pixel (x, y) the color CFAPattern[2 * (y mod 2) + (x mod 2)], with the origin of the pattern at stored pixel (0, 0), independent of the valid-area origin.

### Strips

All measured files have one strip with RowsPerStrip = H (the X2 files store RowsPerStrip = W, which R-SRW-18 replaces by H). Row y starts at byte offset StripOffset + y * R, where R is the row size in bytes: R = 2 * W (V1) and R = 3 * W / 2 (V2). There is no padding between rows.

- R-SRW-24: The decoder must treat the strips as consecutive groups of RowsPerStrip rows (the last strip can hold fewer rows), each strip starting at its StripOffsets value, with the row size R of the variant.

## Compression

This section specifies how the stored bytes of V1 and V2 become sensor values. Neither variant is compressed. The values are used as read, without a shift or a scale.

### Raw data byte order

The byte order of the sensor data comes from maker-note tag 0x0040 (S4, M3, M4). The NX5, NX10 and NX11 files have no tag 0x0040 and store big-endian groups. Their TIFF header is big-endian.

- R-SRW-25: The decoder must determine the raw data byte order as little-endian when maker-note tag 0x0040 is 0, big-endian when it is 1, and the byte order of the TIFF header when the tag or the maker note is absent, and must use the preview path for any other value.

### V1: 16-bit samples

Each pixel is a 16-bit unsigned integer. Pixel (x, y) is at byte offset StripStart(y) + 2 * x, where StripStart(y) is the start of row y from R-SRW-24.

- R-SRW-26: The decoder must read V1 pixel (x, y) from the bytes b0 and b1 at offsets StripStart(y) + 2 * x and StripStart(y) + 2 * x + 1 as b0 + 256 * b1 when the raw data byte order is little-endian, and as 256 * b0 + b1 when it is big-endian.

Worked example (S12, little-endian, row 1000, columns 2000 to 2003, file offset 8574033): bytes 0x21 0x0B 0x70 0x14 0x7D 0x0B 0xD2 0x15 give 0x21 + 256 * 0x0B = 33 + 2816 = 2849, then 112 + 5120 = 5232, 125 + 2816 = 2941 and 210 + 5376 = 5586. With the pattern 0 1 1 2 these are red, green, red, green.

### V2: 12-bit packed

Each row holds 3 * W / 2 bytes. Pixels 2k and 2k + 1 of a row come from the three bytes b0, b1, b2 at row offset 3k (M2).

Little-endian groups: G = b0 + 256 * b1 + 65536 * b2. Pixel 2k is G mod 4096 (bits 0 to 11) and pixel 2k + 1 is floor(G / 4096) (bits 12 to 23).

Big-endian groups: G = 65536 * b0 + 256 * b1 + b2. Pixel 2k is floor(G / 4096) (bits 12 to 23) and pixel 2k + 1 is G mod 4096 (bits 0 to 11).

- R-SRW-27: When the raw data byte order is little-endian, the decoder must compute V2 pixel 2k as b0 + 256 * (b1 mod 16) and pixel 2k + 1 as floor(b1 / 16) + 16 * b2.
- R-SRW-28: When the raw data byte order is big-endian, the decoder must compute V2 pixel 2k as 16 * b0 + floor(b1 / 16) and pixel 2k + 1 as 256 * (b1 mod 16) + b2.
- R-SRW-29: The decoder must use V1 and V2 pixel values as read, without a shift or a scale. The levels of section Levels and white balance refer to these values.

Worked examples:
- Big-endian (S10, row 1000, columns 2000 to 2003, file offset 13580584): 0x21 0x82 0x37 0x18 0xB1 0x6D. Group 1: 16 * 0x21 + floor(0x82 / 16) = 528 + 8 = 536, and 256 * (0x82 mod 16) + 0x37 = 512 + 55 = 567. Group 2: 16 * 0x18 + 11 = 395, and 256 * 1 + 0x6D = 365. With the pattern 2 1 1 0 these are blue, green, blue, green.
- Little-endian (S11, row 1000, columns 2000 to 2003, file offset 11246424): 0x1A 0xB5 0x18 0x00 0xC5 0x17. Group 1: 0x1A + 256 * 5 = 1306, and floor(0xB5 / 16) + 16 * 0x18 = 11 + 384 = 395. Group 2: 0 + 256 * 5 = 1280, and 12 + 16 * 0x17 = 380. With the pattern 1 0 2 1 these are green, red, green, red.

### Out-of-scope codings

- R-SRW-30: The decoder must not read the strip of X1, X2 and X3 files, and must not use the tables that tags 0xA010 and 0xA011 of their raw IFD point to.

## Levels and white balance

The decoder determines the white level first, then the black levels, then the white balance multipliers. The values come from the maker note (R-SRW-19). SRW files carry no WhiteLevel or BlackLevel tag in the raw IFD.

### White level

Decrypted tag 0xA048 gives the white level in its elements 4 to 11 (M10). It gives 4095 for the 12-bit data of V2 and of the WB2000 V1 file, which has BitsPerSample 14 (M4). The EX1 file saturates at 15881, below the tag value 16383 (M10). The table below corrects this model.

| Model (IFD0 0x0110) | White level |
| --- | --- |
| EX1 | 15881 |

- R-SRW-31: The decoder must determine the white level w as the first of these that applies:
  1. The value of the table above, when the Model (compared by R-SRW-9) is in the table.
  2. Element 4 of decrypted tag 0xA048, when the tag has 12 elements and that element is from 256 to 65535.
  3. 4095 for V2, and for V1 4095 when BitsPerSample is 12 and 16383 otherwise.

### Black level

Decrypted tag 0xA028 gives four black levels in the order R, G, G, B (S4 name, M9). The measured values are 0 to 3.

- R-SRW-32: The decoder must use the black level bR = V(0) for red pixels, bG = (V(1) + V(2)) / 2 for green pixels and bB = V(3) for blue pixels of decrypted tag 0xA028 when the tag has 4 elements and each element is from 0 to w - 256, and must use 0 for every color otherwise.

### White balance multipliers

Decrypted tag 0xA021 gives the as-shot levels R, G1, G2, B. S4 states that they are not corrected for the black levels. The decoder subtracts them (M8).

- R-SRW-33: The decoder must compute G = (G1 + G2) / 2 from decrypted tag 0xA021 and the multipliers mR = (R - bR) / (G - bG), mG = 1 and mB = (B - bB) / (G - bG), and must use mR = 2.1137, mG = 1, mB = 1.5737 when the tag is absent, when G - bG, R - bR or B - bB is not greater than 0, or when mR or mB is outside 0.1 to 10.

The fallback multipliers are the means of the as-shot multipliers of the 12 in-scope files of S17.

Worked example (S10): levels 6939, 4096, 4096, 4207 and black levels 3, 3, 3, 3 give G = 4096, mR = 6936 / 4093 = 1.6946, mB = 4204 / 4093 = 1.0271.

### Normalization of CFA values

For a pixel with value s, black level b of its color, white level w and multiplier m of its color:
- n = (s - b) / (w - b)
- v = n * m, clipped to the range 0 to 1.

- R-SRW-34: The decoder must compute v = clip((s - b) / (w - b) * m, 0, 1) for every pixel of the valid area before demosaicing. This is the rule R-ARW-47 of arw.md.

Worked example (fixture F1): s = 1000 (red), 1500 (green), 600 (blue), b = 3, w = 4095, levels 9776, 4096, 4096, 7232.
- mR = 9773 / 4093 = 2.3877351576, mB = 7229 / 4093 = 1.7661861715.
- n = 997 / 4092 = 0.2436461388 (red), 1497 / 4092 = 0.3658357771 (green), 597 / 4092 = 0.1458944282 (blue).
- v = 0.5817625 (red), 0.3658358 (green), 0.2576767 (blue).

## Color conversion

### Matrix source

The maker note carries a color matrix (S4, M5). Tag 0xA031 (ColorMatrixSRGB) holds 9 integers in rows of 3. Each row sums to 256 in every measured file, so a white-balanced neutral stays neutral. Each row is one output channel (sRGB R, G, B). Each column is one camera channel (camera R, G, B). M7 confirms this orientation against the camera JPEG. The transposed reading is the worst choice in all four tested files. Tag 0xA030 equals tag 0xA031 in all in-scope files. In X1 to X3 files tag 0xA031 holds the same matrix for all models and tag 0xA030 varies (Open questions OQ-8).

Measured matrices of tag 0xA031 (M5), for use when the tag is absent:

| Model (IFD0 0x0110) | m11 | m12 | m13 | m21 | m22 | m23 | m31 | m32 | m33 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| NX5, NX10, NX11 | 408 | -114 | -38 | -34 | 348 | -58 | -8 | -104 | 368 |
| NX20, NX210, NX1000, NX1100 | 430 | -152 | -22 | -36 | 352 | -60 | 4 | -128 | 380 |
| NX200 | 426 | -164 | -6 | -24 | 322 | -42 | 8 | -120 | 368 |
| EX1 | 440 | -180 | -4 | -80 | 416 | -80 | 14 | -188 | 430 |
| WB2000 | 530 | -246 | -28 | -118 | 520 | -146 | -26 | -202 | 484 |
| any other model (generic: element-wise mean of the 10 models above) | 434.0 | -154.0 | -24.0 | -46.8 | 371.0 | -68.2 | -1.2 | -133.4 | 390.6 |

### Procedure

- R-SRW-35: The decoder must take the matrix from the first valid source of this list: decrypted tag 0xA031, decrypted tag 0xA030, the table row of the Model (compared by R-SRW-9), the generic row of the table.
- R-SRW-36: The decoder must treat a tag matrix as valid only if it has 9 elements and the sum of each row is from 128 to 512, and must divide each element of the selected matrix by the sum of its row before use.
- R-SRW-37: The decoder must compute the output (r, g, b) from the normalized camera values (vR, vG, vB) as r = m11 * vR + m12 * vG + m13 * vB, g = m21 * vR + m22 * vG + m23 * vB, b = m31 * vR + m32 * vG + m33 * vB with the normalized matrix, and must clip r, g and b to the range 0 to 1.
- R-SRW-38: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S7), without the sRGB transfer function. This is the rule R-ARW-53 of arw.md.

Worked example (fixture F1, matrix 430 -152 -22, -36 352 -60, 4 -128 380, every row sum 256, values v of section Levels and white balance):
- r = 1.6796875 * 0.5817625 - 0.59375 * 0.3658358 - 0.0859375 * 0.2576767 = 0.9771791 - 0.2172150 - 0.0221441 = 0.7378200.
- g = -0.140625 * 0.5817625 + 1.375 * 0.3658358 - 0.234375 * 0.2576767 = -0.0818103 + 0.5030242 - 0.0603930 = 0.3608209.
- b = 0.015625 * 0.5817625 - 0.5 * 0.3658358 + 1.484375 * 0.2576767 = 0.0090900 - 0.1829179 + 0.3824889 = 0.2086610.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and read the header (R-SRW-50, R-SRW-4, R-SRW-5).
2. Read IFD0 and the IFD chain. Validate every IFD (R-SRW-10, R-SRW-11, R-SRW-6, R-SRW-7, R-SRW-8, R-SRW-9).
3. Collect the preview candidates now, so that every later failure can fall back (R-SRW-44, R-SRW-45).
4. Find the raw IFD (R-SRW-12) and classify the file (R-SRW-3, R-SRW-15). For X1 to X5 use the preview path (R-SRW-2, R-SRW-30).
5. Check the required tags (R-SRW-16), the CFA pattern (R-SRW-17, R-SRW-21, R-SRW-23) and the strip layout (R-SRW-18, R-SRW-24).
6. Read the maker note (R-SRW-13, R-SRW-14, R-SRW-59), decrypt its tags (R-SRW-19, R-SRW-20), and determine the valid area (R-SRW-22) and the raw data byte order (R-SRW-25).
7. Determine the white level (R-SRW-31), the black levels (R-SRW-32), the multipliers (R-SRW-33) and the matrix (R-SRW-35, R-SRW-36).
8. Check all sizes against the file and the limits before allocating memory (section Error handling).
9. Decode the sensor data: V1 with R-SRW-26, V2 with R-SRW-27 or R-SRW-28, both with R-SRW-29.
10. Normalize the pixels of the valid area (R-SRW-34).
11. Demosaic inside the valid area (R-SRW-39, R-SRW-40).
12. Convert to linear sRGB (R-SRW-37).
13. Deliver the valid area as the image (R-SRW-41) after the orientation (R-SRW-42, R-SRW-43, R-SRW-38). If any step from 4 to 13 fails, deliver the preview instead (R-SRW-48).

### Demosaicing

The decoder uses bilinear interpolation, the basic non-adaptive method that S8 names. The rule is the rule R-ARW-54 and R-ARW-55 of arw.md, applied to the valid area. For each pixel and each color that the pixel lacks, the missing value is the mean of the pixels of that color among the 8 neighbors of the pixel.

- R-SRW-39: For each pixel (x, y) of the valid area, the decoder must set its own color to its normalized value v, and each missing color to the arithmetic mean of the normalized values of the pixels of that color among the 8 neighbors (x + dx, y + dy), with dx and dy from -1 to 1 and not both 0.
- R-SRW-40: The decoder must ignore neighbors outside the valid area in that mean, and must use 0 when no neighbor of the missing color lies inside the valid area.
- R-SRW-41: The decoder must deliver exactly the valid area: an image of (VX1 - VX0) x (VY1 - VY0) pixels before the orientation, where output pixel (x - VX0, y - VY0) comes from stored pixel (x, y).

Worked example: a pixel at stored (22, 14) of an NX10 file (pattern 2 1 1 0, valid area from (21, 14)) has CFA position 0 and is blue. Its neighbors inside the valid area are (21, 14) and (23, 14) green, (21, 15) and (23, 15) red, (22, 15) green. Row 13 is outside the valid area. Green is the mean of the three green neighbors, red is the mean of the two red neighbors.

### Orientation

- R-SRW-42: The decoder must read Orientation from IFD0 tag 0x0112, must use 1 when the tag is absent or outside 1 to 8, and must apply it after the crop of R-SRW-41.
- R-SRW-43: The decoder must map stored pixel (x, y) of a W' x H' image to the output pixel of the table below, with an output of W' x H' pixels for values 1 to 4 and H' x W' pixels for values 5 to 8. This is the table of R-ARW-58 of arw.md (S1 definitions).

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

## Embedded preview

### Locations

| Location | Tags | Content (M13) |
| --- | --- | --- |
| first SubIFD of IFD0 | 0x0201 offset, 0x0202 length | full-size JPEG: the valid area size, or an aspect-ratio crop of it (WB2000: 3648 x 2048) |
| SubIFD of IFD1 | 0x0201, 0x0202 | thumbnail JPEG, 160 x 120 (320 x 216 in NX1 and NX500 files) |
| maker-note tag 0x0035 | IFD at P + value, its 0x0201 plus P, 0x0202 | JPEG of 640 x 424 or 1616 x 1080 |

All preview streams are baseline JPEG (SOF0, 8-bit, 3 components). A decoder of S2-conformant baseline JPEG decodes them. They start with FF D8 FF DB. Some streams have bytes after the EOI marker (392 bytes in S11, 45 bytes in S14). The full-size JPEG is the largest candidate in every measured file. The preview path works the same for in-scope and out-of-scope files.

### Requirements

- R-SRW-44: The decoder must collect as preview candidates the pairs (0x0201, 0x0202) of every IFD of the main chain and of every valid SubIFD of these IFDs, and must keep a candidate only if offset + length lies inside the file, length >= 4, and the first two bytes are FF D8. The decoder does not need the maker-note preview of tag 0x0035.
- R-SRW-45: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment and must choose the candidate with the largest width * height.
- R-SRW-46: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan.
- R-SRW-47: The decoder must apply the IFD0 Orientation (R-SRW-43) to the preview, except that it must not apply values 5 to 8 when the preview is taller than wide.
- R-SRW-48: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 to X5, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-SRW-49: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-SRW-38) or a preview (sRGB-encoded JPEG values, not linear).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-SRW-48).

### Limits

- R-SRW-50: The decoder must fail for a file shorter than 8 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length (including P + offset) and every product of W, H and a byte count in at least 64-bit arithmetic.
- R-SRW-51: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728).
- R-SRW-52: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values.
- R-SRW-53: The decoder must check, before reading, that every value and every strip byte range lies completely inside the file.

### Structure checks

- R-SRW-54: The decoder must fail in each of these cases:
  1. The sum of StripByteCounts is less than the row size R times H.
  2. StripOffsets and StripByteCounts have different counts.
  3. The strip count is not ceil(H / RowsPerStrip).
  4. A strip holds fewer bytes than R times its number of rows.
- R-SRW-55: The decoder must never read more than 2 * W * H bytes of sensor data for V1 or 3 * W * H / 2 bytes for V2, also when StripByteCounts is larger.
- R-SRW-56: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check selects the default of the quantity.
- R-SRW-57: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-SRW-58: The decoder must discard a partly decoded raw image on failure and must not deliver it.
- R-SRW-59: The decoder must apply R-SRW-11 and R-SRW-52 to the maker-note IFD with the offsets of R-SRW-14, and must treat a maker-note tag whose value bytes do not lie inside the file as absent.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order.

### TV-1: TIFF headers

- Bytes 0x4D 0x4D 0x00 0x2A 0x00 0x00 0x00 0x08: big-endian TIFF, IFD0 at offset 8 (21 of 24 measured files).
- Bytes 0x49 0x49 0x2A 0x00 0x08 0x00 0x00 0x00: little-endian TIFF, IFD0 at offset 8 (NX3000, NX3300, NX mini).
- Bytes 0x4D 0x4D 0x2A 0x00: no valid header (R-SRW-4).

### TV-2: IFD entries

Big-endian entries of the NX10 raw IFD (S10):
- 01 03 00 03 00 00 00 01 80 02 00 00: tag 0x0103, type SHORT, count 1, value 0x8002 = 32770 in the first two bytes of the value field.
- 01 11 00 04 00 00 00 01 00 63 82 F0: tag 0x0111, type LONG, value 0x006382F0 = 6521584.
- 01 17 00 04 00 00 00 01 01 50 59 40: tag 0x0117, value 0x01505940 = 22042944 = 3 * 4704 * 3124 / 2, so the file is V2 (R-SRW-3).
- 82 8E 00 01 00 00 00 04 02 01 01 00: tag 0x828E, type BYTE, count 4, inline values 2, 1, 1, 0.

Little-endian entries of the NX3000 IFD0 (S15):
- 69 87 04 00 01 00 00 00 A6 00 00 00: tag 0x8769, type LONG, value 166. It precedes the SubIFDs entry 4A 01 04 00 02 00 00 00 9E 00 00 00 (two offsets at 158), so the entries are not sorted (R-SRW-7).
- 00 00 00 00 00 00 00 00 00 00 00 00: type 0, skipped (R-SRW-7).

### TV-3: maker note and decryption (S10)

- Exif entry 92 7C 00 07 00 01 7F 83 00 00 01 B0: MakerNote, UNDEFINED, 98179 bytes, P = 432. The bytes at 432 are 00 43 (67 entries).
- Entry A0 10 00 04 00 00 00 08 00 00 94 90: SensorAreas at 432 + 0x9490 = 38464. Bytes 00 00 00 00 00 00 00 00 00 00 12 50 00 00 0C 34 00 00 00 15 00 00 00 0E 00 00 12 05 00 00 0B FE: 0, 0, 4688, 3124, 21, 14, 4613, 3070. Valid area 4592 x 3056 from (21, 14).
- Entry A0 20 00 04 00 00 00 0B 00 00 94 C8: key at 432 + 0x94C8 = 38520. Bytes 00 00 01 31 00 00 00 48 00 00 02 E1 00 00 01 C8 00 00 01 1A 00 00 01 33 00 00 02 07 00 00 02 D4 00 00 00 0D 00 00 01 F9 00 00 00 C1: 305, 72, 737, 456, 282, 307, 519, 724, 13, 505, 193.
- Tag 0xA021 (s = -1, k0 = 0), bytes 00 00 1C 4C 00 00 10 48 00 00 12 E1 00 00 12 37: 6939, 4096, 4096, 4207.
- Tag 0xA028 (s = -1, k0 = 0), bytes 00 00 01 34 00 00 00 4B 00 00 02 E4 00 00 01 CB: 3, 3, 3, 3.
- Tag 0xA031 (s = +1, k0 = 0), bytes 00 00 00 67 FF FF FF 46 FF FF FC F9 FF FF FE 16 00 00 00 42 FF FF FE 93 FF FF FD F1 FF FF FC C4 00 00 01 63: stored 103, -186, -775, -490, 66, -365, -527, -828, 355, real 408, -114, -38, -34, 348, -58, -8, -104, 368.
- Tag 0xA048 (s = +1, k0 = 0), bytes FF FF FE CF FF FF FF B8 FF FF FD 1F FF FF FE 38 00 00 0E E5 00 00 0E CC 00 00 0D F8 00 00 0D 2B 00 00 0F F2 00 00 0E 06 00 00 0F 3E 00 00 0E CE: stored -305, -72, -737, -456, 3813, 3788, 3576, 3371, 4082, 3590, 3902, 3790, real 0, 0, 0, 0, 4095 (eight times). Element 11 uses key index 11 mod 11 = 0: 3790 + 305 = 4095.

### TV-4: V2 groups

- Big-endian (R-SRW-28): 0x21 0x82 0x37 0x18 0xB1 0x6D -> 536, 567, 395, 365 (S10). 0xFF 0xF5 0xDC -> 4095, 1500. 0x25 0x85 0xDC -> 600, 1500.
- Little-endian (R-SRW-27): 0x1A 0xB5 0x18 0x00 0xC5 0x17 -> 1306, 395, 1280, 380 (S11). 0xFF 0xCF 0x5D -> 4095, 1500. 0x58 0xC2 0x5D -> 600, 1500.

### TV-5: V1 samples

- Little-endian (R-SRW-26): 0x21 0x0B 0x70 0x14 0x7D 0x0B 0xD2 0x15 -> 2849, 5232, 2941, 5586 (S12). 0x70 0x17 -> 6000. 0xA0 0x0F -> 4000. 0x60 0x09 -> 2400.
- Big-endian: 0x17 0x70 -> 6000.

### TV-6: classification

- NX10 (S10): Compression 32770, W 4704, H 3124, strip bytes 22042944 = 3 * W * H / 2: V2.
- NX300 (S14): Compression 32770, W 5546, H 3714, strip bytes 18537424, 3 * W * H / 2 = 30896766: X1.
- EX1 (S12): Compression 32769, W 3688, H 2780, strip bytes 20505280 = 2 * W * H: V1.
- NX3000 (S15): Compression 32772: X2. NX500 (S16): Compression 32773: X3.

### TV-7: levels, white balance and matrix

Pixel values 1000 (red), 1500 (green), 600 (blue), black 3, white 4095, levels 9776 4096 4096 7232, matrix 430 -152 -22 -36 352 -60 4 -128 380. Multipliers 2.3877351576 and 1.7661861715. Normalized and balanced values 0.5817625, 0.3658358, 0.2576767. Output linear sRGB for a flat area: 0.7378200, 0.3608209, 0.2086610.

### TV-8: orientation

Stored 3 x 2 image with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a.

### Fixture recipes

The fixtures are complete small SRW files for the decoder tests. All are big-endian like most real files. They share one layout:

- 0x0000: header 4D 4D 00 2A 00 00 00 08.
- 0x0008: IFD0 with 5 entries in ascending order: 0x010F Make ASCII 8 at 0x4A ("SAMSUNG" and a zero byte), 0x0110 Model ASCII 6 at 0x52 ("NX210" and a zero byte), 0x0112 Orientation SHORT 1, 0x014A SubIFDs LONG 1 = 0x184 (raw IFD), 0x8769 LONG 1 = 0x58 (Exif IFD). Next-IFD offset 0.
- 0x0058: Exif IFD with 1 entry: 0x927C UNDEFINED count 0x11A at 0x6A. So P = 0x6A.
- 0x006A: maker-note IFD with 7 entries. Offsets in it count from P. Entry 1 is tag 0x0041 (F1, F4, F5: an unused tag the decoder ignores) or 0x0040 (F2, F3: raw data byte order little-endian), LONG 1 = 0. Then 0xA010 LONG 8 at P + 0x5A = 0xC4: 0 0 8 6 1 1 7 5. 0xA020 LONG 11 at 0xE4: the key of TV-3. 0xA021 LONG 4 at 0x110: stored 10081 4168 4833 7688 (real 9776 4096 4096 7232). 0xA028 SLONG 4 at 0x120: stored 308 75 740 459 (real 3 3 3 3, F3: stored 317 84 749 468, real 12 12 12 12). 0xA031 SLONG 9 at 0x130: stored 125 -224 -759 -492 70 -367 -515 -852 367 (real 430 -152 -22 -36 352 -60 4 -128 380). 0xA048 LONG 12 at 0x154: stored -305 -72 -737 -456 3813 3788 3576 3371 4082 3590 3902 3790 (real 0 0 0 0 4095 x 8, F3: 16383 x 8).
- 0x0184: raw IFD with 11 entries: 0x00FE 0, 0x0100 = 8, 0x0101 = 6, 0x0102 = 12 (F3: 14), 0x0103 = 32770 (F3: 32769), 0x0111 = 0x210, 0x0115 = 3, 0x0116 = 6, 0x0117 = 72 (F3: 96), 0x828D 2 2, 0x828E 1 0 2 1.
- 0x0210: the strip. W = 8, H = 6. The valid area is columns 1 to 6 and rows 1 to 4 (6 x 4 pixels, odd origin). Inside it, every red pixel is 1000, every green pixel 1500 and every blue pixel 600 (F3: 4000, 6000, 2400). Every pixel outside it is 4095 (F3: 16383). A decoder that does not crop, or that uses neighbors outside the valid area, gets other edge values.

F1 (V2, big-endian groups, raw data byte order from the big-endian header), complete file of 600 bytes:

0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 08 00 00 00 4A 01 10 00 02 00 00 00 06 00 00
0020: 00 52 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 01 84 87 69 00 04 00 00
0040: 00 01 00 00 00 58 00 00 00 00 53 41 4D 53 55 4E
0050: 47 00 4E 58 32 31 30 00 00 01 92 7C 00 07 00 00
0060: 01 1A 00 00 00 6A 00 00 00 00 00 07 00 41 00 04
0070: 00 00 00 01 00 00 00 00 A0 10 00 04 00 00 00 08
0080: 00 00 00 5A A0 20 00 04 00 00 00 0B 00 00 00 7A
0090: A0 21 00 04 00 00 00 04 00 00 00 A6 A0 28 00 09
00A0: 00 00 00 04 00 00 00 B6 A0 31 00 09 00 00 00 09
00B0: 00 00 00 C6 A0 48 00 04 00 00 00 0C 00 00 00 EA
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08
00D0: 00 00 00 06 00 00 00 01 00 00 00 01 00 00 00 07
00E0: 00 00 00 05 00 00 01 31 00 00 00 48 00 00 02 E1
00F0: 00 00 01 C8 00 00 01 1A 00 00 01 33 00 00 02 07
0100: 00 00 02 D4 00 00 00 0D 00 00 01 F9 00 00 00 C1
0110: 00 00 27 61 00 00 10 48 00 00 12 E1 00 00 1E 08
0120: 00 00 01 34 00 00 00 4B 00 00 02 E4 00 00 01 CB
0130: 00 00 00 7D FF FF FF 20 FF FF FD 09 FF FF FE 14
0140: 00 00 00 46 FF FF FE 91 FF FF FD FD FF FF FC AC
0150: 00 00 01 6F FF FF FE CF FF FF FF B8 FF FF FD 1F
0160: FF FF FE 38 00 00 0E E5 00 00 0E CC 00 00 0D F8
0170: 00 00 0D 2B 00 00 0F F2 00 00 0E 06 00 00 0F 3E
0180: 00 00 0E CE 00 0B 00 FE 00 04 00 00 00 01 00 00
0190: 00 00 01 00 00 03 00 00 00 01 00 08 00 00 01 01
01A0: 00 03 00 00 00 01 00 06 00 00 01 02 00 03 00 00
01B0: 00 01 00 0C 00 00 01 03 00 03 00 00 00 01 80 02
01C0: 00 00 01 11 00 04 00 00 00 01 00 00 02 10 01 15
01D0: 00 03 00 00 00 01 00 03 00 00 01 16 00 03 00 00
01E0: 00 01 00 06 00 00 01 17 00 04 00 00 00 01 00 00
01F0: 00 48 82 8D 00 03 00 00 00 02 00 02 00 02 82 8E
0200: 00 01 00 00 00 04 01 00 02 01 00 00 00 00 00 00
0210: FF FF FF FF FF FF FF FF FF FF FF FF FF F5 DC 25
0220: 85 DC 25 85 DC 25 8F FF FF F3 E8 5D C3 E8 5D C3
0230: E8 5D CF FF FF F5 DC 25 85 DC 25 85 DC 25 8F FF
0240: FF F3 E8 5D C3 E8 5D C3 E8 5D CF FF FF FF FF FF
0250: FF FF FF FF FF FF FF FF

Expected F1 output: 6 x 4 pixels (orientation 1), every pixel 0.7378200, 0.3608209, 0.2086610 (TV-7).

F2 (V2, little-endian groups): F1 with the byte at 0x006D changed from 41 to 40 (maker-note tag 0x0040 = 0), and the 72 strip bytes at 0x0210 replaced by these 6 rows of 12 bytes:

FF FF FF FF FF FF FF FF FF FF FF FF
FF CF 5D 58 C2 5D 58 C2 5D 58 F2 FF
FF 8F 3E DC 85 3E DC 85 3E DC F5 FF
FF CF 5D 58 C2 5D 58 C2 5D 58 F2 FF
FF 8F 3E DC 85 3E DC 85 3E DC F5 FF
FF FF FF FF FF FF FF FF FF FF FF FF

Expected F2 output: identical to F1. A decoder that reads F2 with big-endian groups gets other values.

F3 (V1, little-endian 16-bit samples), complete file of 624 bytes:

0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 08 00 00 00 4A 01 10 00 02 00 00 00 06 00 00
0020: 00 52 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 01 84 87 69 00 04 00 00
0040: 00 01 00 00 00 58 00 00 00 00 53 41 4D 53 55 4E
0050: 47 00 4E 58 32 31 30 00 00 01 92 7C 00 07 00 00
0060: 01 1A 00 00 00 6A 00 00 00 00 00 07 00 40 00 04
0070: 00 00 00 01 00 00 00 00 A0 10 00 04 00 00 00 08
0080: 00 00 00 5A A0 20 00 04 00 00 00 0B 00 00 00 7A
0090: A0 21 00 04 00 00 00 04 00 00 00 A6 A0 28 00 09
00A0: 00 00 00 04 00 00 00 B6 A0 31 00 09 00 00 00 09
00B0: 00 00 00 C6 A0 48 00 04 00 00 00 0C 00 00 00 EA
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08
00D0: 00 00 00 06 00 00 00 01 00 00 00 01 00 00 00 07
00E0: 00 00 00 05 00 00 01 31 00 00 00 48 00 00 02 E1
00F0: 00 00 01 C8 00 00 01 1A 00 00 01 33 00 00 02 07
0100: 00 00 02 D4 00 00 00 0D 00 00 01 F9 00 00 00 C1
0110: 00 00 27 61 00 00 10 48 00 00 12 E1 00 00 1E 08
0120: 00 00 01 3D 00 00 00 54 00 00 02 ED 00 00 01 D4
0130: 00 00 00 7D FF FF FF 20 FF FF FD 09 FF FF FE 14
0140: 00 00 00 46 FF FF FE 91 FF FF FD FD FF FF FC AC
0150: 00 00 01 6F FF FF FE CF FF FF FF B8 FF FF FD 1F
0160: FF FF FE 38 00 00 3E E5 00 00 3E CC 00 00 3D F8
0170: 00 00 3D 2B 00 00 3F F2 00 00 3E 06 00 00 3F 3E
0180: 00 00 3E CE 00 0B 00 FE 00 04 00 00 00 01 00 00
0190: 00 00 01 00 00 03 00 00 00 01 00 08 00 00 01 01
01A0: 00 03 00 00 00 01 00 06 00 00 01 02 00 03 00 00
01B0: 00 01 00 0E 00 00 01 03 00 03 00 00 00 01 80 01
01C0: 00 00 01 11 00 04 00 00 00 01 00 00 02 10 01 15
01D0: 00 03 00 00 00 01 00 03 00 00 01 16 00 03 00 00
01E0: 00 01 00 06 00 00 01 17 00 04 00 00 00 01 00 00
01F0: 00 60 82 8D 00 03 00 00 00 02 00 02 00 02 82 8E
0200: 00 01 00 00 00 04 01 00 02 01 00 00 00 00 00 00
0210: FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F
0220: FF 3F 70 17 60 09 70 17 60 09 70 17 60 09 FF 3F
0230: FF 3F A0 0F 70 17 A0 0F 70 17 A0 0F 70 17 FF 3F
0240: FF 3F 70 17 60 09 70 17 60 09 70 17 60 09 FF 3F
0250: FF 3F A0 0F 70 17 A0 0F 70 17 A0 0F 70 17 FF 3F
0260: FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F

Expected F3 values: white 16383, black 12, multipliers 9764 / 4084 = 2.3907933399 and 7220 / 4084 = 1.7678746327, normalized and balanced 0.5824008 (red), 0.3657687 (green), 0.2578758 (blue). Output: 6 x 4 pixels, every pixel 0.7389150, 0.3605923, 0.2090000.

F4 (X2): F1 with the byte at 0x01BF changed from 02 to 04 (Compression 32772). Expected: class X2, preview path. F4 has no preview candidate, so the decoder reports an error (R-SRW-48).

F5 (X1): F1 with the byte at 0x01F1 changed from 48 to 47 (StripByteCounts 71, not 3 * 8 * 6 / 2 = 72). Expected: class X1, preview path, error because no preview candidate exists.

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 X1 coding (Compression 32770 with a strip shorter or longer than 3 * W * H / 2: NX300, NX30, NX2000, NX300M, EK-GN120). No permitted source describes it. The raw IFD tags 0xA010 and 0xA011 point to a per-row table (M13). Fallback: preview path (R-SRW-2).
- OQ-2 X2 coding (Compression 32772: NX3000, NX3300, NX mini). No permitted source describes it. Fallback: preview path.
- OQ-3 X3 coding (Compression 32773: NX1, NX500, with lossless and lossy settings and 12 or 14 bits). S5 names the value PackBits, which the data does not match (M13). Fallback: preview path.
- OQ-4 Raw data byte order. Value 1 of tag 0x0040 was not observed. Files without the tag were measured only with a big-endian header. Fallback: R-SRW-25.
- OQ-5 White level. Tag 0xA048 has no documented meaning (S4 marks it "RawData?"). Its use as white level rests on M10. The EX1 value 15881 comes from one file at ISO 80. The NX5, NX10 and NX11 samples have no saturated pixels. Fallback: R-SRW-31.
- OQ-6 Black level. The values of tag 0xA028 (0 to 3) agree with the dark margins within 3 units (M9), on low-ISO files only. Fallback: R-SRW-32.
- OQ-7 White balance with manual or preset settings. Tags 0xA021 and 0xA022 are equal in all files, and no in-scope file has manual white balance (M8). Fallback: R-SRW-33 with tag 0xA021.
- OQ-8 Color matrix. Tag 0xA031 gives sRGB by its S4 name, its row sums (256) and M7. The chromaticity of the white point (D65 or the scene illuminant) is not documented. Only one matrix exists per file, for all illuminants. In X1 to X3 files tag 0xA031 is one matrix for all models and tag 0xA030 varies per file. Fallback: R-SRW-35.
- OQ-9 Valid area against the camera JPEG. The NX210 JPEG starts 10.5 columns right of the valid area (M11). Fallback: R-SRW-22 and R-SRW-41 with tag 0xA010.
- OQ-10 Lens corrections. The camera JPEGs are corrected for distortion (M11). Maker-note tags 0xA050 to 0xA057 (S4: "Distortion?", "ChromaticAberration?", "Vignetting?") have no documented semantics. Fallback: no lens correction.
- OQ-11 Tags 0x0050 (RawDataCFAPattern) and 0xA025 (HighlightLinearityLimit). Their effect on decoding is unknown. M6 shows that tag 0x0050 does not change the pattern of tag 0x828E. Fallback: ignored (R-SRW-21).
- OQ-12 Compression 32769 with packed data (S5 calls 32769 "Packed RAW") and Compression 1 were not observed. Fallback: X4, preview path (R-SRW-3).
- OQ-13 Unmeasured models, for example NX100 (sample only under a non-CC0 license), NX300M, NX2000 variants, EX2F, WB5000. Fallback: classification by structure (R-SRW-3) and the generic table rows of R-SRW-33 and R-SRW-35.
- OQ-14 Several strips. Not observed. Fallback: R-SRW-24.
- OQ-15 Highlights. Values above the white level and balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-SRW-34 and R-SRW-37.
- OQ-16 Orientation of the embedded preview for rotated shots. No rotated sample was measured. Fallback: R-SRW-47.
- OQ-17 Files without a maker note or without tag 0xA010. Not observed. Without tag 0xA010 the junk margins cannot be removed. Fallback: preview path (R-SRW-16).
