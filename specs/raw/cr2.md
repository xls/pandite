# Canon CR2 raw format: decoder specification

Document: specs/raw/cr2.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite CR2 decoder (clean-room step 1 of 2).

This document describes the Canon CR2 raw file format as facts. It is written for an implementer who has only this document, the sibling specification arw.md (for the parts that this document reuses by reference), TIFF 6.0 (S1) and ITU-T T.81 (S2). Every normative statement is traceable to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Conventions

- Requirements have the form R-CR2-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- A reference such as R-ARW-54 points to the requirement with that id in specs/raw/arw.md. Where a CR2 requirement cites an ARW requirement, the cited rule applies to CR2 unchanged, and the CR2 requirement states the rule in short.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are hexadecimal bytes in file order, for example 0x49 0x49 or "49 49".
- Bit 0 is the least significant bit of a value.
- floor(a / b) is integer division rounded toward minus infinity. a mod b is the remainder of that division (0 <= result < b for b > 0).
- Raster coordinates are (x, y): x is the column, y is the row. (0, 0) is the top-left sample of the raster (section Sensor data layout).
- "Index i" of a 16-bit array is the (i + 1)-th 16-bit value, counted from 0. The ExifTool tables (S4, S5) use the same numbering in their column "Index2".
- Ranges "a to b" and rectangles "x from a to b" include both ends.

### Glossary

- CR2: the raw file format of the measured Canon EOS and PowerShot cameras (S21). A CR2 file is a TIFF file with a Canon signature in bytes 8 to 15.
- IFD: image file directory, the TIFF 6.0 tag table (S1 section 2).
- IFD0, IFD1, IFD2, IFD3: the first four IFDs of the main IFD chain.
- Raw IFD: the IFD that describes the sensor data. Its offset is in bytes 12 to 15 of the file (it is IFD3).
- Exif IFD: the IFD at the offset in IFD0 tag 0x8769 (S3).
- MakerNote: the Canon private IFD inside the Exif IFD tag 0x927C (S3, S5).
- SensorInfo: MakerNote tag 0x00E0, a 16-bit array with the raster size and the borders of the visible area (S5).
- ColorData: MakerNote tag 0x4001, a 16-bit array with white balance data (S5).
- CanonModelID: MakerNote tag 0x0010, a 32-bit camera model number (S5).
- LJ92: the lossless JPEG process of ITU-T T.81 Annex H with Huffman coding (frame marker SOF3, 0xFF 0xC3).
- Frame: the image that the LJ92 frame header describes: X samples per line, Y lines, Nf components (S2 B.2.2).
- MCU: minimum coded unit (S2 A.2). In CR2 RAW files one MCU holds one sample of each component.
- Sample sequence: all decoded samples of the frame in coding order (section Compression, R-CR2-22).
- Slice: a vertical band of the raster. The sample sequence fills the slices one after the other (section Sensor data layout).
- Raster: the W x H array of sensor values after the slices are put together.
- Visible area: the part of the raster that holds the photograph. SensorInfo gives its borders.
- Masked area: raster columns or rows outside the visible area that the sensor covers from light. They hold the black level.
- CFA: color filter array, here a 2x2 Bayer pattern (S9).
- CFA position: the index 2 * (y mod 2) + (x mod 2) of a raster sample inside its 2x2 repeat cell (0, 1, 2 or 3).
- Linear sRGB: linear-light RGB with the sRGB primaries and the D65 white point (S10). The decoder output is linear sRGB.
- sRAW, mRAW: reduced-size raw recordings that Canon calls S-RAW and M-RAW (S7). They store YCbCr samples. They are out of scope (X1).
- Preview path: the fallback that shows the embedded JPEG preview in place of the decoded raw data.

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Container layout, Metadata tags, Decoding procedure (orientation), Embedded preview |
| S2 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images, Annex H and Annexes B, C, F | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Compression, Embedded preview, Error handling, Test vectors |
| S3 | Exif Version 2.32, CIPA DC-X008-Translation-2019 (Exif IFD pointer 0x8769, MakerNote 0x927C) | https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf | standard | 2026-11-14 | Container layout (Exif IFD, MakerNote) |
| S4 | ExifTool Tag Names: EXIF tags table (metadata tag reference documentation): 0xC5E0 CR2CFAPattern, 0xC640 RawImageSegmentation, 0xC6C5 SRawType, Compression values | https://exiftool.org/TagNames/EXIF.html | community documentation | 2026-11-14 | Metadata tags, Sensor data layout (slices, CFA) |
| S5 | ExifTool Tag Names: Canon tags table (metadata tag reference documentation): CanonModelID values, SensorInfo, ColorData1 to ColorData12 | https://exiftool.org/TagNames/Canon.html | community documentation | 2026-11-14 | Metadata tags, Levels and white balance, Color conversion |
| S6 | DXOMARK camera sensor Measurements pages, tab Color response (one page per model, URL pattern https://www.dxomark.com/Cameras/Canon/<name>---Measurements, names in the Color conversion table) | https://www.dxomark.com/Cameras/Canon/EOS-5D-Mark-III---Measurements | community documentation | 2026-11-14 | Color conversion (matrices, D50 white balance scales) |
| S7 | Canon EOS 5D Mark III specification page (Canon Vietnam): "RAW (14-bit Canon original)", RAW 5760 x 3840, M-RAW 3960 x 2640, S-RAW 2880 x 1920 | https://vn.canon/vi/support/6200124200 | vendor | 2026-11-14 | Scope and variants (RAW, M-RAW, S-RAW), Sensor data layout (visible area size) |
| S8 | B. K. Gunturk, J. Glotzbach, Y. Altunbasak, R. W. Schafer, R. M. Mersereau, Demosaicking: Color Filter Array Interpolation, IEEE Signal Processing Magazine 22(1), 2005 (peer-reviewed article) | https://www.ece.lsu.edu/ipl/papers/IEEE_SPM2005.pdf | community documentation | 2026-11-14 | Decoding procedure (bilinear demosaicing) |
| S9 | B. E. Bayer, Color imaging array, US Patent 3,971,065 (1976, expired) | https://patents.google.com/patent/US3971065A | patent | 2026-11-14 | Sensor data layout (Bayer arrangement) |
| S10 | M. Stokes, M. Anderson, S. Chandrasekar, R. Motta, A Standard Default Color Space for the Internet - sRGB, version 1.10, 1996 | https://www.w3.org/Graphics/Color/sRGB.html | standard | 2026-11-14 | Color conversion (output primaries and D65 white) |
| S11 | raw.pixls.us repository table (file list with license, checksum and size of each sample) | https://raw.pixls.us/json/getrepository.php?set=all | community documentation | 2026-11-14 | selection of CC0 samples for S12 to S22 |
| S12 | Measurement: EOS 350D sample IMG_1707 (CC0), saved as .testdata/cr2/EOS_350D_RAW.CR2 | https://raw.pixls.us/data/Canon/EOS%20350D/IMG_1707.CR2 | measurement | 2026-11-14 | all sections, records M1 to M6, M9, M15 |
| S13 | Measurement: EOS 20D sample IMG_3893 (CC0), saved as EOS_20D_RAW.CR2 | https://raw.pixls.us/data/Canon/EOS%2020D/IMG_3893.CR2 | measurement | 2026-11-14 | Sensor data layout (no slice tag), records M3, M9, M15 |
| S14 | Measurement: EOS 5D Mark II sample IMG_0051, a color chart scene (CC0), saved as EOS_5D_Mark_II_RAW.CR2 | https://raw.pixls.us/data/Canon/EOS%205D%20Mark%20II/IMG_0051.CR2 | measurement | 2026-11-14 | Compression (4 components), CFA value 3, color, records M2, M7, M8, M9, M15 |
| S15 | Measurement: EOS 5D Mark III samples 5G4A9394 (RAW), 5G4A9395 (sRAW1, M-RAW), 5G4A9396 (sRAW2, S-RAW) (CC0), saved under .testdata/cr2/ | https://raw.pixls.us/data/Canon/EOS%205D%20Mark%20III/ | measurement | 2026-11-14 | Compression, Levels (clipping at 15464), Scope (X1), records M2, M6, M9, M10, M15 |
| S16 | Measurement: EOS 80D sample IMG_0098 (CC0), saved as EOS_80D_RAW.CR2 | https://raw.pixls.us/data/Canon/EOS%2080D/IMG_0098.CR2 | measurement | 2026-11-14 | Levels (clipping at 16383), records M5, M6, M9, M15 |
| S17 | Measurement: EOS 6D Mark II sample 6DII-RAW (CC0), saved as EOS_6D_Mark_II_RAW.CR2 | https://raw.pixls.us/data/Canon/EOS%206D%20Mark%20II/6DII-RAW.CR2 | measurement | 2026-11-14 | Sensor data layout (layout C), records M2, M3, M9, M15 |
| S18 | Measurement: PowerShot G12 sample 2017_09_20_img_1146 (CC0), saved as PowerShot_G12_RAW.CR2 | https://raw.pixls.us/data/Canon/PowerShot%20G12/2017_09_20_img_1146.cr2 | measurement | 2026-11-14 | Metadata tags (ColorData as bytes), records M8, M9, M14, M15 |
| S19 | Measurement: PowerShot S120 sample IMG_1995 (CC0), saved as PowerShot_S120_RAW.CR2 | https://raw.pixls.us/data/Canon/PowerShot%20S120/IMG_1995.CR2 | measurement | 2026-11-14 | Sensor data layout (inconsistent slice tag), records M3, M9, M14, M15 |
| S20 | Measurement: further whole files, not saved: EOS 5D IMG_1428, EOS 40D _MG_0154 (sRAW2), EOS 5D Mark IV B13A0729 (dual pixel), EOS-1D Mark III _K5R7350, PowerShot G11 IMG_3310, PowerShot G16 IMG_0963, PowerShot SX60 HS Canon-SX60HS-Canon-RAW (CC0) | https://raw.pixls.us/data/Canon/ | measurement | 2026-11-14 | records M5, M7, M9, M10, M11, M14 |
| S21 | Measurement: container survey of one CC0 RAW file for each of 96 model folders of the archive (93 parsed; three files of PowerShot A480, SX40 HS and SX150 IS are served through a redirect that the survey tool did not follow): headers, IFDs, MakerNote, lossless JPEG header, and a partial decode of the first slice compared with the embedded preview | https://raw.pixls.us/data/Canon/ | measurement | 2026-11-14 | all sections, records M1 to M8, M12, M13 |
| S22 | Measurement: header survey of 31 CC0 sRAW1 and sRAW2 files and of three EOS 5D Mark IV RAW files (one with dual pixel data) | https://raw.pixls.us/data/Canon/ | measurement | 2026-11-14 | Scope and variants (X1, dual pixel), records M10, M11 |

Sample files are from the public raw sample archive raw.pixls.us. The archive table (S11) marks each used file as CC0 (public domain dedication). The author downloaded the files and inspected them with measurement tools that the author wrote from S1 and S2 for this purpose. The tools are not part of the deliverable. One comparison decoded the camera's embedded JPEG previews with the Pillow imaging library, which is a general image library and contains no raw decoder. The file .testdata/cr2/SOURCES.txt lists the ten saved files with URL, camera, variant, license and checksum.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (container, S12 to S22): all 93 survey files and all whole files start with 49 49 2A 00 10 00 00 00 43 52 02 00, followed by a 4-byte little-endian offset. IFD0 is at offset 16. The main IFD chain has 4 IFDs (5 in the dual pixel file of S20). The offset in bytes 12 to 15 equals the offset of IFD3 in all files. IFD3 has Compression 6, one StripOffsets value and one StripByteCounts value, and the tags 0xC5D8 (value 1, or 0 in EOS 20D and EOS-1D Mark II N), 0xC5E0 and, in 92 of 93 files, 0xC640. 0xC6C5 is 1 in 83 RAW files and absent in 10 older files. 23 files also carry ImageWidth and ImageLength in IFD3, equal to the raster size. IFD0 holds Make "Canon", Model, Orientation, the Exif IFD and the preview JPEG as a strip (0x0111, 0x0117, Compression 6). IFD1 holds a 160 x 120 JPEG thumbnail (0x0201, 0x0202). IFD2 holds an uncompressed RGB image (8 bits per sample in old files, 16 in newer files). The MakerNote is an IFD without header at the value offset of Exif tag 0x927C, and its value offsets count from the file start (for example in S15 RAW the MakerNote starts at 996 with 42 entries, and CanonModelID at its entry reads 0x80000285). The next-IFD field of the MakerNote is not meaningful (65535 in S12).
- M2 (lossless JPEG frames, S21 and whole files): every raw strip is one LJ92 stream with the markers SOI, DHT (two tables in one segment), SOF3, SOS, entropy-coded data, EOI, in this order. No DRI marker occurs. The frame parameters of the 93 survey files are: P = 14 with Nf = 2 (42 files), P = 14 with Nf = 4 (28 files), P = 12 with Nf = 2 (23 files). All components have H = V = 1. The scan has Ns = Nf, Ss = 1, Se = 0, Ah = 0, Al = Pt = 0, and Huffman table selectors 0, 1 (Nf = 2) or 0, 1, 0, 1 (Nf = 4). All 14 whole RAW files (S12 to S20) decode by S2 Annex H with no error. In each of them all X * Y MCUs decode before the EOI marker, and the EOI marker is the last 2 bytes of the strip.
- M3 (slice reassembly, S12, S13, S17, S19, S21): reassembling the sample sequence by R-CR2-17 gives a clean image in every whole file. Mean absolute difference between samples two rows apart: S12 57.3 with slices, 91.1 without. S13 has no tag 0xC640: 14.3 as one slice, 113.8 with two assumed slices of 1798. S19 has tag 0xC640 = 0, 0, 4176 while SensorWidth is 4192: 45.5 with W = 4192, 161.5 with W = 4176. The same tag value appears in PowerShot G16 (S20, also correct with W = 4192). In all 93 survey files SensorWidth x SensorHeight equals X * Y * Nf. Frame layouts: X * Nf = W in 89 files, X * Nf = 2 * W in 4 files (EOS 5D Mark IV, 5DS, 5DS R, 6D Mark II). Tag 0xC640 with first value n >= 1 satisfies n * a + b = SensorWidth in all 66 such files.
- M4 (SensorInfo and visible area, S12 to S21): SensorInfo has 17 values in all files. Index 1 times index 2 equals X * Y * Nf in all files, and the reassembly with index 1 as raster width and index 2 as raster height gives clean images (M3). Index 5 to 8 give the visible area with inclusive borders: in the 63 survey files with a full-size preview, (index 7 - index 5 + 1) x (index 8 - index 6 + 1) equals the preview size (for example S15: 140 to 5899 and 96 to 3935 give 5760 x 3840, the RAW size stated by S7). The other 30 files have smaller previews. Index 9 to 12 are 0 in all EOS files except EOS-1D Mark II N, and give a rectangle in 13 PowerShot files (G9, G11, G12, G15, G16, G1 X, G1 X Mark II, S95, S100, S110, S120, SX50 HS, SX60 HS). They are 0 in the other PowerShot files.
- M5 (black level, S12 to S21): raster columns 0 to floor(L / 2) - 1 (L = SensorInfo index 5) hold dark values in all survey files except PowerShot G11. Between the dark columns and the visible area there is a band of exposed columns: the first exposed column is at 0.6 * L or later. Column profile of S12: columns 0 to 41 have means 256 to 257 (standard deviation 2), columns 42 to 51 have means 358 to 380. In PowerShot G11 (S20) L = 24 and image data starts at column 8, while the SensorInfo rectangle (3732, 28, 3735, 2763) at the right holds black values 128. The SensorInfo rectangle and the left columns agree within 4 units in all other files that have both (S19: 130 and 128, PowerShot G1 X: 510 and 514). EOS-1D Mark III (S20) has columns 0 and 1 at 2149 and 1934 and columns 2 to 5 near 1035, while the other dark columns are at 1024 to 1025: the median is not affected. Measured black levels are near 128, 256, 512, 1024 or 2048. MakerNote values that S5 calls PerChannelBlackLevel agree with the measurement where present (S15: 2047 2047 2047 2048, S16: 512 512 511 511, S17: 513 513 513 513), but S5 gives several possible positions for them in ColorData4 without the rule that selects one.
- M6 (white level, S12 to S21): clipped highlights sit at one value per file, and that value is not always 2^P - 1. Measured clip values with the number of samples: S15 15464 (1899 samples), S16 16383 (622764), S12 4095 (24391), EOS 30D 3398 (3903), EOS 2000D 15303 (625), EOS Rebel T7 15304 (42082), EOS DIGITAL REBEL XSi 16089 (14594), EOS 100D 13583 (54). Files without clipping have a maximum value that only 1 to 3 samples reach. The MakerNote values that S5 calls SpecularWhiteLevel and NormalWhiteLevel do not equal the clip value (S15: 15321 and 14809, clip 15464. S16: 11892 and 11435, clip 16383).
- M7 (CFA, S12 to S21): for each of the four raster positions of the 2x2 cell, the log ratio of that position to the cell mean was correlated with the log ratios of the R, G and B channels of the embedded preview over 40 x 30 or 60 x 90 blocks. Tag 0xC5E0 value 1 gave red at raster (0, 0) and blue at (1, 1) in 65 survey files, and value 3 gave green at (0, 0), blue at (1, 0), red at (0, 1) in 22 survey files (for example S14). In 6 survey files the partial check gave no clear result (too little color in the decoded part, or a raster width taken from tag 0xC640 in the first survey pass for PowerShot S120 and G16). Whole-image checks of EOS 20D, EOS-1D Mark III and PowerShot G11 (S13, S20) confirmed red at raster (0, 0) with value 1, and the full pipeline with value 1 matches the preview for PowerShot G16 and S120 (M9). Values 2 and 4 did not occur.
- M8 (white balance, S12 to S21): ColorData was present in 92 of 93 survey files (absent in PowerShot G9). Its type, length in 16-bit values and first value (version) are listed in the section Levels and white balance. At the index of R-CR2-27 every file holds four positive values R, G1, G2, B with G1 = G2 in 91 files (S12: 2170, 1018, 1020, 1755). In S14 (a color chart under even light) the tag gives R / G = 2.427 and B / G = 1.552, and the median ratios between the embedded preview and the raw values give 2.446 and 1.522.
- M9 (color, S12 to S20): the output of the full pipeline of this document was compared with the embedded preview by the mean chromaticity distance over 60 x 90 blocks (blocks with values from 0.01 to 0.9). Results with the matrix of the table, with the transposed matrix, and with no matrix: S12 0.032, 0.262, 0.072. S13 0.057, 0.439, 0.123. S14 0.020, 0.452, 0.074. S15 0.044, not computable (negative outputs), 0.148. S16 (generic matrix) 0.032, 0.458, 0.062. S17 0.022, 0.543, 0.085. S18 0.052, 0.441, 0.120. S19 0.154 over the whole frame and 0.042 over the central 30 percent (the camera preview corrects lens distortion, M14). Dual pixel file of S20 0.032. Further files of S20: EOS-1D Mark III 0.085, PowerShot G11 0.058, G16 0.027, SX60 HS 0.060. The log luminance correlation with the preview is 0.86 to 0.99 except S19 (0.68, distortion).
- M10 (sRAW and mRAW, S15, S20, S22): all 31 sRAW files have tag 0xC6C5 = 4 and an LJ92 frame with P = 15, Nf = 3, component 1 with H = 2 and V = 1 (sRAW2, S-RAW) or V = 2 (sRAW1, M-RAW), components 2 and 3 with H = V = 1. The S15 sRAW2 frame is 2880 x 1920 and the sRAW1 frame 3960 x 2640, equal to the S-RAW and M-RAW sizes of S7. The luma plane of S15 sRAW2 decodes to a clean image when the MCU sequence fills the slices. The luma plane of S15 sRAW1 (V = 2) shows a mean absolute vertical difference of 20695 with the same rule, so its layout is not established. Conversion of S15 sRAW2 with the YCbCr formulas of S1 section 21 (chroma zero level 16384) gives a chromaticity distance of 0.201 from the preview, against 0.289 for a gray image. Least-squares fits of the preview against Y, Cb, Cr give inconsistent coefficients between files.
- M11 (dual pixel, S20, S22): the EOS 5D Mark IV file B13A0729 has a fifth IFD with the same tags as IFD3 plus tag 0xC6DD (256 SHORT values). Its raw IFD (header offset, IFD3) decodes by this document to an image that matches the preview (M9). The second stream (IFD4) decodes with the same frame parameters to a darker image. The other two 5D Mark IV RAW files have four IFDs.
- M12 (previews, S12 to S21): the IFD0 strip is a baseline JPEG (SOF0, 3 components) in all files, ending with FF D9. Sizes: full visible-area size in 63 survey files (for example 5760 x 3840 in S15), smaller in older and PowerShot files (1536 x 1024 to 2496 x 1664 and 1600 x 1200 to 2304 x 1536). In the whole files IFD0 tags 0x0100 and 0x0101 equal the preview size. The IFD1 thumbnail is a 160 x 120 JPEG. S19 has an APP2 segment in its preview.
- M13 (orientation, S21): 91 survey files have Orientation 1 and 2 files (EOS DIGITAL REBEL XT, EOS Kiss Digital N) have 8. Their previews are stored unrotated (1536 x 1024, landscape), like the raw data.
- M14 (PowerShot files, S18 to S21): ColorData has type 7 (UNDEFINED) with count 5120 bytes (2560 16-bit values) and version -3 or -4 in 24 files (PowerShot and EOS M3, M5, M6, M10, M100). The embedded preview of S19 is corrected for lens distortion, while the raw image shows barrel distortion.
- M15 (raster check values, S12 to S19): MD5 digests of the reassembled rasters and pipeline outputs are listed in test vector TV-12.
- M16 (matrix source checks): the S6 pages of EOS 5D Mark II and EOS 5D Mark III were read twice and agree with the table. The S6 page of EOS 80D prints each matrix row with a leading 0 and only two further values, so the 80D row is left out of the table (OQ-6).

### Sources encountered and not used

No page opened for this document contained decoder source code or stated that its content derives from decoder source code. The archive main page https://raw.pixls.us/ links to decoder projects. These links were not followed. The CR2 description page http://lclevy.free.fr/cr2/ was not opened, because the author could not confirm in advance that it is independent of decoder source code. The ExifTool pages S4 and S5 were read only as tag tables, not the ExifTool source modules.

## Scope and variants

### CR2 generations

CR2 files differ by camera generation in the ColorData layout (S5) and in the frame layout of the sensor data (M2, M3). The decoder does not need the generation: it reads the version from ColorData (R-CR2-27) and the layout from the frame and SensorInfo (R-CR2-15). The table lists the measured models (S21).

| ColorData (S5) | Version (index 0) | Length (16-bit values) | Measured models |
| --- | --- | --- | --- |
| ColorData1 | none (index 0 is 1164) | 582 | EOS 20D, 350D (DIGITAL REBEL XT, Kiss Digital N) |
| ColorData3 | 1 | 796 | EOS 5D, 30D, 400D (DIGITAL REBEL XTi), EOS-1D Mark II N |
| ColorData4 | 2, 3, 5, 6, 7, 9 | 674, 692, 1227, 1250, 1251, 1337, 1338, 1346 | EOS-1D Mark III, 40D, 450D, 1000D, 50D, 5D Mark II, 500D, 7D, EOS-1D Mark IV, 550D, 60D, 1100D |
| ColorData6 | 10 | 1273, 1275 | EOS 600D, 1200D |
| ColorData7 | 10, 11 | 1312, 1313, 1506 | EOS 5D Mark III, 6D, 70D, 100D, 650D, 700D, M, M2, 7D Mark II, 750D, 760D |
| ColorData8 | 12, 13, 14, 15 | 1560, 1592, 1353, 1602 | EOS 5DS, 5DS R, 5D Mark IV, 80D, EOS-1D X Mark II, 1300D, 2000D, 4000D, 6D Mark II, 77D, 200D, 800D |
| ColorData5 | -3, -4 | 2560 (type 7, 5120 bytes) | PowerShot G1 X, G1 X Mark II, G1 X Mark III, G3 X, G5 X, G7 X, G7 X Mark II, G9 X, G9 X Mark II, G11, G12, G15, G16, S95, S100, S110, S120, SX50 HS, SX60 HS, EOS M3, M5, M6, M10, M100 |
| none | tag absent | none | PowerShot G9 |

### Storage variants

S7 names three recording sizes: RAW (14-bit Canon original), M-RAW and S-RAW. The files store them as follows (M1, M2, M10):

| Variant | Camera setting | IFD3 0xC6C5 | LJ92 frame | Status |
| --- | --- | --- | --- | --- |
| V1 | RAW | 1 or absent | P = 12 or 14, Nf = 2 or 4, all H = V = 1, one CFA sample per raster pixel | in scope |
| X1 | M-RAW (sRAW1) and S-RAW (sRAW2) | 4 | P = 15, Nf = 3, component 1 with H = 2 and V = 1 or 2 | out of scope |

V1 has three frame layouts. The decoding procedure is the same for all three (R-CR2-15 to R-CR2-17). The layouts matter for tests.

| Layout | Frame | Measured models (M3) |
| --- | --- | --- |
| A | Nf = 2, X * Nf = W | all measured models not listed under B and C, among them all PowerShot and EOS M models |
| B | Nf = 4, X * Nf = W | EOS 50D, 5D Mark II, 500D, 7D, 550D, 60D, 600D, 1100D, 1200D, 1300D, 2000D, 4000D, EOS-1D Mark IV |
| C | Nf = 4, X * Nf = 2 * W (one frame line holds two raster rows) | EOS 5D Mark IV, 5DS, 5DS R, 6D Mark II |

### Out-of-scope relatives

| Id | Relative | Detection from file contents |
| --- | --- | --- |
| X1 | sRAW1 (M-RAW) and sRAW2 (S-RAW), YCbCr samples (M10) | IFD3 tag 0xC6C5 equals 4, or the LJ92 frame has Nf = 3 or a component with H or V different from 1 |
| X2 | any other LJ92 frame (precision, component count or scan parameters outside R-CR2-21) | R-CR2-21 fails |
| X3 | files that are not CR2: CRW (older Canon raw), CR3 (newer Canon raw), TIFF without the CR2 signature, big-endian files | bytes 0 to 3 differ from 49 49 2A 00, or bytes 8 to 10 differ from 43 52 02 |

The second image of a dual pixel file (IFD4 with tag 0xC6DD, M11) is not decoded. The decoder decodes IFD3 of such a file as V1.

### Requirements

- R-CR2-1: The decoder must decode the sensor data of variant V1 in all three frame layouts as this document specifies.
- R-CR2-2: The decoder must use the preview path (section Embedded preview) for X1 and X2 files and must not interpret their sensor data. For X3 files this document defines no behaviour: the decoder must report that the file is not a CR2 file.
- R-CR2-3: The decoder must classify the file by these rules, in this order:
  1. Bytes 0 to 3 are not 49 49 2A 00, or bytes 8 to 10 are not 43 52 02: X3.
  2. The raw IFD (R-CR2-8) is invalid, has no StripOffsets or StripByteCounts, or Compression is not 6: X2.
  3. IFD3 tag 0xC6C5 is present and its value is not 1: X1.
  4. The LJ92 frame header (R-CR2-21) has Nf = 3 or a component with H or V different from 1: X1.
  5. The LJ92 frame header and scan header meet R-CR2-21: V1.
  6. Otherwise: X2.

## Container layout

### Header

A CR2 file is a little-endian TIFF file (S1 section 2) with a Canon extension of the header (M1).

| Offset | Size | Field | Value in CR2 |
| --- | --- | --- | --- |
| 0 | 2 | byte order | 49 49 ("II", little-endian) |
| 2 | 2 | TIFF magic number | 42 (bytes 2A 00) |
| 4 | 4 | offset of IFD0 | 16 in all measured files |
| 8 | 2 | signature | 43 52 ("CR") |
| 10 | 1 | major version | 2 |
| 11 | 1 | minor version | 0 |
| 12 | 4 | offset of the raw IFD | equals the offset of IFD3 |

- R-CR2-4: The decoder must accept a file as CR2 only if bytes 0 to 3 are 49 49 2A 00 and bytes 8 to 10 are 43 52 02. It must accept any value of byte 11.
- R-CR2-5: The decoder must read every multi-byte TIFF field as little-endian.

### IFD structure

IFDs follow S1 section 2. The entry layout, the field types and the rule for inline values are the same as in arw.md, section Container layout, IFD structure: an IFD is a 2-byte entry count N, N entries of 12 bytes (tag, type, count, value or offset) and a 4-byte next-IFD offset. A value of count * type size <= 4 bytes is inline in the value field. A longer value is at the file offset in the value field.

- R-CR2-6: The decoder must read IFD entries as R-ARW-6 to R-ARW-10 specify: inline values for count * size <= 4 and offset values otherwise (R-ARW-6), tags found by number in any entry order (R-ARW-7), SHORT, LONG and type 13 accepted for integer tags (R-ARW-8), entries with a type outside 1 to 13 skipped (R-ARW-9), and ASCII values compared after trailing zero bytes and spaces are removed (R-ARW-10).
- R-CR2-7: The decoder must treat an IFD as invalid when its offset lies outside the file, when N is 0 or greater than 1024, or when its 2 + 12 * N + 4 bytes do not lie completely inside the file (R-ARW-14).

### IFD chain, raw IFD and MakerNote

Measured layout (M1, values from S15 RAW):

| Structure | Where | Content |
| --- | --- | --- |
| IFD0 | offset 16 | ImageWidth and ImageLength of the preview, Compression 6, Make "Canon", Model, StripOffsets and StripByteCounts of the preview JPEG, Orientation, Exif IFD (0x8769), XMP (0x02BC) in newer files |
| IFD1 | next-IFD offset of IFD0 | thumbnail JPEG (0x0201 offset, 0x0202 length), 160 x 120 |
| IFD2 | next-IFD offset of IFD1 | uncompressed RGB image (8 or 16 bits per sample), not used |
| IFD3 (raw IFD) | header bytes 12 to 15, and next-IFD offset of IFD2 | Compression 6, one strip with the LJ92 stream, tags 0xC5D8, 0xC5E0, 0xC640, 0xC6C5 |
| IFD4 | next-IFD offset of IFD3, dual pixel files only | second raw stream, not used (M11) |
| Exif IFD | IFD0 tag 0x8769 | Exif data and MakerNote (0x927C) |
| MakerNote | value offset of Exif tag 0x927C | Canon IFD: CanonModelID (0x0010), SensorInfo (0x00E0), ColorData (0x4001) and other tags |

Example (S15 RAW): IFD0 at 16 with 18 entries. Exif IFD at 446. MakerNote at 996. IFD1 at 71822. IFD2 at 71852. IFD3 at 72036 (header bytes 12 to 15: 64 19 01 00). Preview JPEG at 93772 (1479160 bytes). Thumbnail at 80324 (13448 bytes). Raw strip at 3309636 (22655373 bytes).

The MakerNote is an IFD without a header. Its entry value offsets count from byte 0 of the file, like all other offsets (M1). Its next-IFD field holds no usable value.

- R-CR2-8: The decoder must use as the raw IFD the IFD at the offset in header bytes 12 to 15, and must not require that this IFD is reachable through the IFD chain.
- R-CR2-9: The decoder must read IFD0 at the offset in header bytes 4 to 7 and must follow the next-IFD offsets from IFD0 for the preview candidates (R-CR2-37), stopping at offset 0, at an invalid IFD, at a repeated offset or after 16 IFDs.
- R-CR2-10: The decoder must find the MakerNote by reading the Exif IFD at the offset in IFD0 tag 0x8769 and reading an IFD at the value offset of its tag 0x927C, and must read all MakerNote value offsets as offsets from byte 0 of the file.
- R-CR2-11: The decoder must ignore the next-IFD field of the MakerNote and of the raw IFD.

## Metadata tags

All offsets in this section are file offsets. "Required" means that the decoder cannot decode V1 without the tag and uses the preview path when it is missing.

### Raw IFD (IFD3)

| Tag | Name (S4) | Type | Count | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- |
| 0x0100 | ImageWidth | SHORT | 1 | raster width W, present in newer files only (M1), not used | ignored | none |
| 0x0101 | ImageLength | SHORT | 1 | raster height H, present in newer files only, not used | ignored | none |
| 0x0103 | Compression | SHORT | 1 | 6 (S4: "JPEG (old-style)"). The strip holds a complete LJ92 stream | required | none |
| 0x0111 | StripOffsets | LONG | 1 | file offset of the LJ92 stream | required | none |
| 0x0117 | StripByteCounts | LONG | 1 | length of the LJ92 stream in bytes | required | none |
| 0xC5D8 | (no public name) | LONG | 1 | 1, or 0 in two old models (M1) | ignored | none |
| 0xC5E0 | CR2CFAPattern | LONG | 1 | CFA arrangement, R-CR2-19 | optional | 1 |
| 0xC640 | RawImageSegmentation | SHORT | 3 | slices: n (number of slices minus one), a (width of each slice except the last), b (width of the last slice) (S4) | optional | one slice |
| 0xC6C5 | SRawType | LONG | 1 | 1 for RAW, 4 for sRAW (M1, M10) | optional | 1 |
| 0xC6DD | (no public name) | SHORT | 256 | only in IFD4 of dual pixel files (M11) | ignored | none |

### IFD0 and Exif IFD

| Tag | Name | IFD | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- |
| 0x010F | Make | IFD0 | "Canon" | optional | empty |
| 0x0110 | Model | IFD0 | for example "Canon EOS 5D Mark III", diagnostic only | optional | empty |
| 0x0111, 0x0117, 0x0103 | StripOffsets, StripByteCounts, Compression 6 | IFD0 | the preview JPEG (section Embedded preview) | optional | none |
| 0x0112 | Orientation | IFD0 | 1 to 8, S1 definition | optional | 1 |
| 0x8769 | Exif IFD pointer (S3) | IFD0 | offset of the Exif IFD | required | none |
| 0x927C | MakerNote (S3) | Exif IFD | type UNDEFINED, the value offset is the MakerNote IFD | required | none |
| 0x0201, 0x0202 | JPEGInterchangeFormat, JPEGInterchangeFormatLength | IFD1 | thumbnail JPEG | optional | none |

### MakerNote

| Tag | Name (S5) | Type | Count | Meaning | Required | Default |
| --- | --- | --- | --- | --- | --- | --- |
| 0x0010 | CanonModelID | LONG | 1 | camera model number, key of the color table (R-CR2-30) | optional | no table row |
| 0x00E0 | SensorInfo | SHORT | 17 (at least 9 used) | raster size and borders, table below | required | none |
| 0x4001 | ColorData | SHORT, or UNDEFINED in PowerShot files | 582 to 2560 values | white balance, R-CR2-27 | optional | R-CR2-28 |

SensorInfo indices (S5 names, M4):

| Index | Name (S5) | Meaning in this document | Example S15 |
| --- | --- | --- | --- |
| 0 | (none) | 34, the byte length of the array | 34 |
| 1 | SensorWidth | raster width W | 5920 |
| 2 | SensorHeight | raster height H | 3950 |
| 5 | SensorLeftBorder | L, first column of the visible area | 140 |
| 6 | SensorTopBorder | T, first row of the visible area | 96 |
| 7 | SensorRightBorder | R, last column of the visible area | 5899 |
| 8 | SensorBottomBorder | B, last row of the visible area | 3935 |
| 9 | BlackMaskLeftBorder | BL, first column of the black rectangle, 0 in EOS files | 0 |
| 10 | BlackMaskTopBorder | BT, first row of the black rectangle | 0 |
| 11 | BlackMaskRightBorder | BR, last column of the black rectangle | 0 |
| 12 | BlackMaskBottomBorder | BB, last row of the black rectangle | 0 |

S5 describes indices 9 to 12 as "coordinates for the area to the left or right of the image used to calculate the average black level". M5 confirms that the rectangle holds black values.

### Tags observed and not used

ColorData holds more values than the white balance (S5: color temperatures, PerChannelBlackLevel, NormalWhiteLevel, SpecularWhiteLevel, ColorCalib). This document does not use them (M5, M6, OQ-3). The MakerNote holds many other tags (S5), for example CameraSettings (0x0001), FileInfo (0x0093) and AspectInfo (0x009A). This document does not use them.

### Requirements

- R-CR2-12: The decoder must use the preview path when the raw IFD lacks Compression, StripOffsets or StripByteCounts, when StripOffsets or StripByteCounts has a count other than 1, when the MakerNote cannot be found, or when SensorInfo is absent or has fewer than 9 values.
- R-CR2-13: The decoder must read ColorData as an array of signed 16-bit little-endian values: count values for type SHORT (3) or SSHORT (8), and floor(count / 2) values for type UNDEFINED (7). For any other type it must treat ColorData as absent.
- R-CR2-14: The decoder must read SensorInfo as unsigned 16-bit values, must use indices 9 to 12 only when SensorInfo has at least 13 values, and must treat them as 0 otherwise.

## Sensor data layout

### Raster size

The raster is the W x H array of sensor values. W and H come from SensorInfo indices 1 and 2 (M4). The LJ92 frame holds exactly W * H samples. The frame line length X * Nf is W in layouts A and B and 2 * W in layout C. When the raster has more than one slice, a frame line is not a raster row in any layout. The decoder therefore places each sample by its index in the sample sequence (R-CR2-17), not by its frame line.

- R-CR2-15: The decoder must take W from SensorInfo index 1 and H from SensorInfo index 2, and must use the preview path when W * H differs from X * Y * Nf of the LJ92 frame.

### Slices

The camera encodes the raster as a sequence of vertical bands, the slices. It writes slice 0 completely, row by row from the top, then slice 1, and so on. The sample sequence of the LJ92 stream (R-CR2-22) is this concatenation (M3).

Tag 0xC640 has three values n, a and b (S4). If n >= 1, there are n + 1 slices: slices 0 to n - 1 have width a and slice n has width b, and n * a + b = W. If n = 0 or the tag is absent, there is one slice of width W. Files with n = 0 hold b = W in 24 measured files and b different from W in 2 files (M3), so the decoder does not use b when n = 0.

Mapping of sample index s (0 to W * H - 1) to raster coordinates, for n >= 1:
- j = min(n, floor(s / (a * H))), the slice number.
- q = s - j * a * H, the index inside the slice.
- w = a if j < n, and w = b if j = n, the slice width.
- x = j * a + (q mod w), y = floor(q / w).

For one slice: x = s mod W, y = floor(s / W).

Worked example (vector TV-5): n = 1, a = 4, b = 2, W = 6, H = 4. Samples 0 to 15 fill slice 0 (columns 0 to 3): sample 5 goes to (1, 1). Samples 16 to 23 fill slice 1 (columns 4 and 5): sample 19 has j = 1, q = 3, w = 2, so x = 4 + 1 = 5 and y = 1.

Example (S14, EOS 5D Mark II): n = 2, a = 1936, b = 1920, W = 5792, H = 3804. Sample 1936 goes to (0, 1). Sample 7364544 = 1936 * 3804 starts slice 1 at (1936, 0). Sample 14729088 starts slice 2 at (3872, 0).

- R-CR2-16: The decoder must use the slice widths a (n times) and b when tag 0xC640 has at least 3 values and n >= 1, and must use one slice of width W when the tag is absent or n = 0. When n >= 1, it must use the preview path unless a >= 1, b >= 1 and n * a + b = W.
- R-CR2-17: The decoder must store sample s of the sample sequence at the raster coordinates (x, y) given by the mapping above.

### Visible area and masked area

SensorInfo gives the visible area as the rectangle from column L to column R and from row T to row B, both ends included (M4). The output image is the visible area: its size is (R - L + 1) x (B - T + 1). The columns left of L and the rows above T are partly masked from light. They hold the black level in the columns 0 to floor(L / 2) - 1. The columns between the masked columns and L hold exposed but unused data (M5). The PowerShot files and EOS-1D Mark II N give a separate black rectangle in SensorInfo indices 9 to 12 (M4, M5).

Measured dimensions (M4):

| File | Layout | W x H | L, T, R, B | Output size |
| --- | --- | --- | --- | --- |
| EOS 350D (S12) | A, P = 12 | 3516 x 2328 | 52, 19, 3507, 2322 | 3456 x 2304 |
| EOS 20D (S13) | A, P = 12, no slice tag | 3596 x 2360 | 84, 19, 3587, 2354 | 3504 x 2336 |
| EOS 5D Mark II (S14) | B, P = 14 | 5792 x 3804 | 168, 56, 5783, 3799 | 5616 x 3744 |
| EOS 5D Mark III (S15) | A, P = 14 | 5920 x 3950 | 140, 96, 5899, 3935 | 5760 x 3840 |
| EOS 80D (S16) | A, P = 14 | 6288 x 4056 | 276, 46, 6275, 4045 | 6000 x 4000 |
| EOS 6D Mark II (S17) | C, P = 14 | 6384 x 4224 | 132, 56, 6371, 4215 | 6240 x 4160 |
| PowerShot G12 (S18) | A, P = 12 | 3744 x 2784 | 72, 24, 3719, 2759 | 3648 x 2736 |
| PowerShot S120 (S19) | A, P = 12 | 4192 x 3062 | 132, 40, 4131, 3039 | 4000 x 3000 |

- R-CR2-18: The decoder must use the preview path unless 0 <= L <= R < W, 0 <= T <= B < H, R - L + 1 >= 2 and B - T + 1 >= 2, and must deliver an image of (R - L + 1) x (B - T + 1) pixels (before orientation) whose pixel (i, j) is raster (L + i, T + j).

### CFA arrangement

The CFA is a 2x2 Bayer pattern (S9) whose origin is raster (0, 0), not the visible area origin (M7). Tag 0xC5E0 selects one of four arrangements (S4). The table lists the colors of the CFA positions 0, 1, 2, 3, that is (x even, y even), (x odd, y even), (x even, y odd), (x odd, y odd).

| 0xC5E0 | S4 text | Position 0 | Position 1 | Position 2 | Position 3 | Measured (M7) |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | [Red,Green][Green,Blue] | R | G | G | B | 65 files |
| 2 | [Blue,Green][Green,Red] | B | G | G | R | not observed |
| 3 | [Green,Blue][Red,Green] | G | B | R | G | 22 files |
| 4 | [Green,Red][Blue,Green] | G | R | B | G | not observed |

Example (S12): L = 52 and T = 19 with value 1. Raster (52, 19) is at position 2, so the top-left pixel of the visible area is green, and the visible area starts with the rows G B / R G.

- R-CR2-19: The decoder must give raster sample (x, y) the color of CFA position 2 * (y mod 2) + (x mod 2) in the row of the table for the value of tag 0xC5E0 (default 1), and must use the preview path for any other value.

## Compression

Every V1 raw strip is one complete LJ92 stream (M2): SOI, DHT, SOF3, SOS, entropy-coded data, EOI. The decoder decodes it by S2 Annex H. This is the same process as arw.md R-ARW-36, without the Sony line-start exception R-ARW-39: all lines use the line-start rule of S2 H.1.2.1.

### LJ92 decoding

The rules of S2 that the decoder applies:
- Huffman tables from DHT segments (S2 B.2.4.2), code generation by S2 Annex C. One DHT segment can define several tables. Lossless scans use the DC-class tables (Tc = 0, S2 H.1.2.2). The measured streams define tables 0 and 1 with Tc = 0 (bytes 00 and 01 before the counts).
- Marker segments other than DHT, SOF3, DRI and SOS that appear before SOS are skipped by their length field (S2 B.1.1.4). None occur in the measured streams.
- Difference decoding by S2 F.2.2.1 with the categories of S2 Table H.2: category SSSS = 0 means difference 0, SSSS = 1 to 15 is followed by SSSS extra bits, SSSS = 16 means difference 32768 with no extra bits.
- Prediction by S2 H.1.2.1: the first sample of the first line (and of each restart interval) is predicted by 2^(P - Pt - 1). The other samples of the first line use Ra (the sample to the left). The first sample of every other line uses Rb (the sample above). All other samples use the predictor of the scan header (S2 Table H.1). In V1 files the predictor is 1 (Ra) and Pt = 0 (M2).
- Reconstruction: value = (prediction + difference) mod 2^16, then shifted left by Pt.
- Each component is predicted only from samples of the same component, in its own plane of X x Y samples.
- Byte stuffing: in the entropy-coded data a 0xFF byte is followed by a 0x00 byte, which the decoder removes (S2 F.1.2.3).

Measured stream headers (M2):

| File | P | X | Y | Nf | Huffman tables | Raster |
| --- | --- | --- | --- | --- | --- | --- |
| EOS 350D (S12) | 12 | 1758 | 2328 | 2 | 0 and 1, identical | 3516 x 2328 |
| EOS 5D Mark II (S14) | 14 | 1448 | 3804 | 4 | 0 and 1, identical | 5792 x 3804 |
| EOS 5D Mark III (S15) | 14 | 2960 | 3950 | 2 | 0 and 1, identical | 5920 x 3950 |
| EOS 6D Mark II (S17) | 14 | 3192 | 2112 | 4 | 0 and 1, identical | 6384 x 4224 |

### Sample sequence

The decoded samples form one sequence in coding order: frame line v from 0 to Y - 1, then MCU u from 0 to X - 1 in the line, then component k from 1 to Nf in scan order. Sample (u, v) of component k has the sequence index s = (v * X + u) * Nf + (k - 1). The slice mapping (R-CR2-17) places index s in the raster.

Example (S17, layout C, X = 3192, Nf = 4, W = 6384, one slice): frame line 0 holds the indices 0 to 12767, which are raster rows 0 and 1. Index 6384 is MCU u = 1596, component 1 of frame line 0, and goes to raster (0, 1).

Worked example (vector TV-3 and TV-10): the first entropy-coded bytes of S15 are FF 00 1F EF FC 77. After removal of the stuffed 00, the bits start 11111111 00011111 11101111. The first code 111111110 is category 13. The 13 extra bits 0011111111011 are 2043. The first bit is 0, so the difference is 2043 - 8191 = -6148. The prediction is 2^(14 - 0 - 1) = 8192, and the value is 2044. This is sample s = 0, raster (0, 0).

### Requirements

- R-CR2-20: The decoder must decode the raw strip as an LJ92 stream by S2 Annex H with the rules listed above (Huffman tables by S2 Annex C, differences by S2 F.2.2.1 and Table H.2, predictors by S2 Table H.1, line-start rule by S2 H.1.2.1 for every line, reconstruction modulo 2^16, output shifted left by Pt).
- R-CR2-21: The decoder must accept as V1 only a stream with all of these properties:
  1. One SOF3 frame header with P equal to 12 or 14, Nf equal to 2 or 4, and H = 1 and V = 1 for every component.
  2. One SOS scan header that contains all Nf components, each component once, with Ss (predictor) = 1, Se = 0, Ah = 0 and Al (Pt) = 0.
  3. X >= 1 and Y >= 1.
- R-CR2-22: The decoder must number the decoded samples in coding order by s = (v * X + u) * Nf + (k - 1), with frame line v, MCU u and component position k (1 to Nf) in the scan header.
- R-CR2-23: The decoder must handle a DRI marker and RST markers as S2 specifies (reset of the predictions at each restart interval), although no measured stream contains them.

## Levels and white balance

The decoder determines, in this order, the black levels, the white level and the white balance multipliers.

### Black level

The black level is measured in the file itself (M5). The decoder selects a black rectangle:
1. If SensorInfo indices 9 to 12 (BL, BT, BR, BB) are not all 0, and 0 <= BL < BR < W, 0 <= BT < BB < H, and the rectangle from column BL to BR and row BT to BB does not overlap the visible area (it overlaps when BL <= R, BR >= L, BT <= B and BB >= T all hold): that rectangle.
2. Otherwise: columns 0 to floor(L / 2) - 1 and rows T to B. This requires L >= 8.

For each CFA position p (0 to 3), the black level b(p) is the lower median of the raster samples inside the black rectangle whose position is p: sort the n values in ascending order and take the value at rank floor((n - 1) / 2), counting from 0. The lower median ignores the few unusual columns of M5.

Example (S12): L = 52, T = 19, B = 2322. The black rectangle is columns 0 to 25 and rows 19 to 2322. The lower median of each position is 257. Example (PowerShot G11, S20): indices 9 to 12 are 3732, 28, 3735, 2763, right of R = 3671, so the black rectangle is that rectangle and the black level is 128.

- R-CR2-24: The decoder must select the black rectangle by rules 1 and 2 above, and must use the preview path when rule 1 does not apply and L < 8.
- R-CR2-25: The decoder must compute the black level of each CFA position as the lower median of the raster samples of that position inside the black rectangle.

### White level

The file carries no reliable white level (M6). The camera clips highlights at one value per file, which is 2^P - 1 or lower. The decoder estimates the clip value from the visible area: let M be the largest raster value in the visible area and C the number of visible samples equal to M.

- R-CR2-26: The decoder must use the white level w = M when C >= 16 and M >= 2^(P - 1), and w = 2^P - 1 otherwise, and must use the preview path when w <= max(b(0), b(1), b(2), b(3)) + 255.

Examples: S15 has M = 15464 and C = 1899, so w = 15464. S16 has M = 16383 and C = 622764, so w = 16383. S17 has M = 12223 and C = 1, so w = 16383. Fixture F1 has M = 6048 with C = 36, below 2^13 = 8192, so w = 16383.

### White balance multipliers

The as-shot white balance is the four ColorData values R, G1, G2, B at an index that depends on the ColorData layout (S5, M8):

| Condition (first rule that matches) | Index of R, G1, G2, B | S5 table |
| --- | --- | --- |
| ColorData has exactly 582 values | 25 | ColorData1 (WB_RGGBLevelsAsShot at 25) |
| value at index 0 is from 1 to 15 and ColorData has at least 67 values | 63 | ColorData3, 4, 6, 7, 8 (WB_RGGBLevelsAsShot at 63, in ColorData4 through ColorCoefs at 63) |
| value at index 0 is -3 or -4 and ColorData has at least 75 values | 71 | ColorData5 (ColorCoefs or ColorCoefs2 at 71, WB_RGGBLevelsAsShot at its index 0) |
| any other ColorData, or no ColorData | none | ColorData2 (EOS-1D Mark II, EOS-1Ds Mark II, not measured) and unknown layouts |

The decoder computes three multipliers mR, mG, mB from the first available source:
1. ColorData by the table above, when R, G1, G2 and B are all greater than 0: G = (G1 + G2) / 2, mR = R / G, mG = 1, mB = B / G.
2. The D50 white balance scales of the CanonModelID row of the color table (section Color conversion): mR and mB from the table, mG = 1.
3. The generic scales mR = 2.073, mG = 1, mB = 1.672 (the mean of the table).

Worked example (vector TV-8, S15): ColorData index 0 is 10, so the index is 63. Bytes 99 06 00 04 00 04 A9 08 give 1689, 1024, 1024, 2217. G = 1024, mR = 1689 / 1024 = 1.6494141, mB = 2217 / 1024 = 2.1650391.

- R-CR2-27: The decoder must select the ColorData index of the as-shot white balance by the first matching row of the table above.
- R-CR2-28: The decoder must take the WB multipliers from the first available source of the list above.

### Normalization

For a raster sample with value s at CFA position p with color c in the visible area, black level b(p), white level w and multiplier m(c):
- v = clip((s - b(p)) / (w - b(p)) * m(c), 0, 1)

- R-CR2-29: The decoder must compute v by the formula above for every sample of the visible area before demosaicing.

Worked example (fixture F1): b = 2048, w = 16383, mR = 2.3720703, mB = 1.7412109. R sample 4048: (4048 - 2048) / 14335 * 2.3720703 = 0.1395187 * 2.3720703 = 0.3309481. G sample 6048: 4000 / 14335 = 0.2790373. B sample 3048: 1000 / 14335 * 1.7412109 = 0.0697593 * 1.7412109 = 0.1214657.

## Color conversion

### Matrix source

CR2 files carry no color matrix that a public document describes. S5 lists ColorCalib coefficients inside ColorData but states no meaning that converts them to a matrix (OQ-3). The decoder therefore uses a table keyed by CanonModelID (MakerNote tag 0x0010, which is the same for the regional names of one camera, S5). S6 (DXOMARK) publishes for each tested camera a "color matrix as defined in ISO standard 17321" for the CIE D50 and CIE A illuminants, together with white balance scales. The matrix converts white-balanced camera RGB to linear sRGB. Each row is one output channel (R, G, B of sRGB). Each column is one camera channel (R, G, B). The rows sum to 1, so a white-balanced neutral stays neutral, and M9 confirms the orientation: the transposed matrix gives chromaticity errors from 0.26 to more than 0.5, the table matrix 0.020 to 0.057.

How to read S6: open https://www.dxomark.com/Cameras/Canon/<name>---Measurements, tab "Color response". The page shows two blocks. In all 70 pages of the table the first block (tab CIE-D50) has the larger ratio of the R scale to the B scale, which identifies the D50 block. The table gives the D50 block, read on 2026-11-14, rounded to 2 decimals by S6. Column "Model (S5 name)" is the CanonModelID name of S5.

| CanonModelID | Model (S5 name) | S6 page name | WB R | WB B | m11 | m12 | m13 | m21 | m22 | m23 | m31 | m32 | m33 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0x02230000 | PowerShot G9 | PowerShot-G9 | 2.58 | 1.77 | 1.93 | -1.00 | 0.07 | -0.15 | 1.47 | -0.31 | 0.12 | -0.61 | 1.48 |
| 0x02490000 | PowerShot G10 | PowerShot-G10 | 1.97 | 1.68 | 1.67 | -0.72 | 0.05 | -0.16 | 1.61 | -0.45 | 0.06 | -0.69 | 1.63 |
| 0x02700000 | PowerShot G11 | PowerShot-G11 | 1.96 | 1.83 | 1.81 | -0.74 | -0.07 | -0.19 | 1.56 | -0.36 | 0.13 | -0.63 | 1.50 |
| 0x02720000 | PowerShot S90 | PowerShot-S90 | 1.99 | 1.84 | 1.84 | -0.77 | -0.07 | -0.20 | 1.56 | -0.36 | 0.12 | -0.61 | 1.49 |
| 0x02920000 | PowerShot G12 | PowerShot-G12 | 1.73 | 2.30 | 1.88 | -0.80 | -0.08 | -0.23 | 1.56 | -0.33 | 0.18 | -0.80 | 1.62 |
| 0x02950000 | PowerShot S95 | PowerShot-S95 | 1.70 | 2.32 | 1.79 | -0.71 | -0.09 | -0.20 | 1.51 | -0.31 | 0.18 | -0.72 | 1.55 |
| 0x03080000 | PowerShot G1 X | PowerShot-G1-X | 2.01 | 1.73 | 2.09 | -1.21 | 0.12 | -0.20 | 1.54 | -0.35 | 0.08 | -0.65 | 1.57 |
| 0x03110000 | PowerShot S100 (new) | PowerShot-S100 | 2.23 | 2.48 | 1.80 | -0.70 | -0.10 | -0.16 | 1.61 | -0.45 | -0.03 | -0.50 | 1.52 |
| 0x03330000 | PowerShot G15 | PowerShot-G15 | 2.34 | 2.49 | 1.85 | -0.85 | 0.00 | -0.13 | 1.53 | -0.40 | 0.03 | -0.54 | 1.51 |
| 0x03340000 | PowerShot SX50 HS | PowerShot-SX50-HS | 1.67 | 1.92 | 1.75 | -0.65 | -0.10 | -0.24 | 1.53 | -0.29 | 0.10 | -0.74 | 1.64 |
| 0x03360000 | PowerShot S110 (new) | PowerShot-S110 | 2.24 | 2.62 | 1.85 | -0.83 | -0.01 | -0.15 | 1.58 | -0.43 | 0.02 | -0.53 | 1.51 |
| 0x03540000 | PowerShot G16 | PowerShot-G16 | 2.22 | 1.80 | 1.79 | -0.75 | -0.05 | -0.18 | 1.45 | -0.27 | 0.09 | -0.74 | 1.65 |
| 0x03550000 | PowerShot S120 | PowerShot-S120 | 2.26 | 1.78 | 1.83 | -0.80 | -0.03 | -0.17 | 1.45 | -0.28 | 0.10 | -0.76 | 1.66 |
| 0x03640000 | PowerShot G1 X Mark II | PowerShot-G1-X-Mark-II | 1.82 | 1.71 | 1.93 | -1.07 | 0.15 | -0.23 | 1.52 | -0.29 | 0.05 | -0.68 | 1.63 |
| 0x03740000 | EOS M3 | EOS-M3 | 2.01 | 1.81 | 2.13 | -1.35 | 0.23 | -0.24 | 1.70 | -0.45 | 0.02 | -0.75 | 1.73 |
| 0x03750000 | PowerShot SX60 HS | PowerShot-SX60-HS | 1.63 | 1.87 | 1.75 | -0.70 | -0.05 | -0.23 | 1.52 | -0.29 | 0.08 | -0.72 | 1.64 |
| 0x03780000 | PowerShot G7 X | PowerShot-G7-X | 2.14 | 1.90 | 1.81 | -0.75 | -0.06 | -0.24 | 1.54 | -0.30 | 0.07 | -0.77 | 1.70 |
| 0x03840000 | EOS M10 | EOS-M10 | 1.97 | 1.66 | 2.08 | -1.21 | 0.13 | -0.24 | 1.59 | -0.35 | 0.05 | -0.69 | 1.64 |
| 0x03850000 | PowerShot G3 X | PowerShot-G3-X | 2.11 | 1.86 | 1.87 | -0.78 | -0.09 | -0.22 | 1.56 | -0.34 | 0.10 | -0.74 | 1.64 |
| 0x03930000 | PowerShot G9 X | PowerShot-G9-X | 2.12 | 1.82 | 1.89 | -0.80 | -0.09 | -0.21 | 1.55 | -0.34 | 0.09 | -0.72 | 1.63 |
| 0x03940000 | EOS M5 | EOS-M5 | 1.45 | 1.36 | 2.20 | -1.46 | 0.26 | -0.22 | 1.65 | -0.43 | 0.02 | -0.69 | 1.67 |
| 0x03950000 | PowerShot G5 X | PowerShot-G5-X | 2.15 | 1.85 | 1.78 | -0.73 | -0.05 | -0.23 | 1.53 | -0.30 | 0.08 | -0.74 | 1.67 |
| 0x03980000 | EOS M100 | EOS-M100 | 1.49 | 1.38 | 2.15 | -1.41 | 0.27 | -0.22 | 1.66 | -0.44 | 0.02 | -0.71 | 1.69 |
| 0x04070000 | EOS M6 | EOS-M6 | 1.46 | 1.35 | 2.18 | -1.43 | 0.25 | -0.22 | 1.66 | -0.44 | 0.02 | -0.70 | 1.68 |
| 0x04100000 | PowerShot G9 X Mark II | PowerShot-G9-X-Mark-II | 2.06 | 1.79 | 1.85 | -0.74 | -0.11 | -0.21 | 1.52 | -0.30 | 0.08 | -0.68 | 1.60 |
| 0x80000169 | EOS-1D Mark III | EOS-1D-Mark-III | 2.34 | 1.37 | 1.96 | -1.07 | 0.10 | -0.18 | 1.50 | -0.32 | 0.01 | -0.32 | 1.31 |
| 0x80000174 | EOS-1D Mark II | EOS-1D-Mark-II | 2.24 | 1.18 | 2.11 | -1.11 | 0.00 | -0.16 | 1.49 | -0.33 | 0.00 | -0.30 | 1.30 |
| 0x80000175 | EOS 20D | EOS-20D | 1.80 | 1.50 | 1.83 | -0.89 | 0.05 | -0.16 | 1.50 | -0.34 | 0.08 | -0.55 | 1.47 |
| 0x80000176 | EOS Digital Rebel XSi / 450D / Kiss X2 | EOS-450D | 2.08 | 1.44 | 1.91 | -1.00 | 0.09 | -0.19 | 1.65 | -0.46 | 0.07 | -0.70 | 1.64 |
| 0x80000188 | EOS-1Ds Mark II | EOS-1Ds-Mark-II | 2.24 | 1.32 | 1.99 | -1.06 | 0.07 | -0.16 | 1.46 | -0.30 | 0.02 | -0.37 | 1.35 |
| 0x80000189 | EOS Digital Rebel XT / 350D / Kiss Digital N | EOS-350D | 2.23 | 1.54 | 1.62 | -0.64 | 0.02 | -0.15 | 1.59 | -0.44 | 0.02 | -0.45 | 1.44 |
| 0x80000190 | EOS 40D | EOS-40D | 2.07 | 1.49 | 1.80 | -0.84 | 0.05 | -0.16 | 1.54 | -0.38 | 0.08 | -0.67 | 1.58 |
| 0x80000213 | EOS 5D | EOS-5D | 2.40 | 1.29 | 1.96 | -1.09 | 0.13 | -0.18 | 1.53 | -0.35 | 0.00 | -0.32 | 1.31 |
| 0x80000215 | EOS-1Ds Mark III | EOS-1Ds-Mark-III | 2.36 | 1.33 | 2.02 | -1.13 | 0.11 | -0.18 | 1.50 | -0.32 | 0.01 | -0.32 | 1.31 |
| 0x80000218 | EOS 5D Mark II | EOS-5D-Mark-II | 2.57 | 1.44 | 2.25 | -1.50 | 0.25 | -0.26 | 1.62 | -0.36 | -0.04 | -0.40 | 1.43 |
| 0x80000232 | EOS-1D Mark II N | EOS-1D-Mark-II-N | 1.97 | 1.46 | 1.81 | -0.77 | -0.04 | -0.19 | 1.59 | -0.40 | 0.05 | -0.45 | 1.40 |
| 0x80000234 | EOS 30D | EOS-30D | 1.99 | 1.56 | 1.82 | -0.90 | 0.08 | -0.18 | 1.56 | -0.37 | 0.06 | -0.50 | 1.43 |
| 0x80000236 | EOS Digital Rebel XTi / 400D / Kiss Digital X | EOS-400D | 2.00 | 1.58 | 1.57 | -0.57 | 0.00 | -0.18 | 1.60 | -0.41 | 0.02 | -0.45 | 1.43 |
| 0x80000250 | EOS 7D | EOS-7D | 2.44 | 1.43 | 2.14 | -1.40 | 0.26 | -0.25 | 1.64 | -0.40 | 0.00 | -0.66 | 1.66 |
| 0x80000252 | EOS Rebel T1i / 500D / Kiss X3 | EOS-500D | 2.31 | 1.24 | 2.62 | -1.91 | 0.29 | -0.32 | 1.88 | -0.55 | -0.04 | -0.65 | 1.68 |
| 0x80000254 | EOS Rebel XS / 1000D / Kiss F | EOS-1000D | 2.51 | 1.37 | 1.80 | -0.93 | 0.13 | -0.18 | 1.59 | -0.41 | 0.00 | -0.32 | 1.32 |
| 0x80000261 | EOS 50D | EOS-50D | 2.33 | 1.25 | 2.51 | -1.80 | 0.29 | -0.30 | 1.83 | -0.53 | -0.01 | -0.63 | 1.64 |
| 0x80000269 | EOS-1D X | EOS-1D-X | 1.97 | 1.50 | 2.22 | -1.36 | 0.15 | -0.26 | 1.65 | -0.39 | 0.05 | -0.75 | 1.70 |
| 0x80000270 | EOS Rebel T2i / 550D / Kiss X4 | EOS-550D | 2.54 | 1.38 | 2.26 | -1.48 | 0.22 | -0.22 | 1.66 | -0.44 | 0.01 | -0.53 | 1.51 |
| 0x80000281 | EOS-1D Mark IV | EOS-1D-Mark-IV | 2.32 | 1.43 | 2.29 | -1.54 | 0.25 | -0.30 | 1.74 | -0.44 | -0.03 | -0.58 | 1.62 |
| 0x80000285 | EOS 5D Mark III | EOS-5D-Mark-III | 2.02 | 1.73 | 2.27 | -1.42 | 0.15 | -0.28 | 1.67 | -0.39 | 0.04 | -0.80 | 1.75 |
| 0x80000286 | EOS Rebel T3i / 600D / Kiss X5 | EOS-600D | 2.27 | 1.67 | 2.12 | -1.28 | 0.16 | -0.24 | 1.63 | -0.38 | 0.04 | -0.69 | 1.65 |
| 0x80000287 | EOS 60D | EOS-60D | 2.40 | 1.43 | 2.17 | -1.38 | 0.21 | -0.24 | 1.64 | -0.39 | 0.00 | -0.58 | 1.58 |
| 0x80000288 | EOS Rebel T3 / 1100D / Kiss X50 | EOS-1100D | 2.33 | 1.51 | 2.05 | -1.18 | 0.14 | -0.24 | 1.64 | -0.41 | 0.02 | -0.59 | 1.58 |
| 0x80000289 | EOS 7D Mark II | EOS-7D-Mark-II | 1.91 | 1.67 | 2.16 | -1.42 | 0.25 | -0.27 | 1.83 | -0.55 | 0.04 | -0.82 | 1.78 |
| 0x80000301 | EOS Rebel T4i / 650D / Kiss X6i | EOS-650D | 2.12 | 1.76 | 2.20 | -1.37 | 0.17 | -0.27 | 1.67 | -0.40 | 0.05 | -0.78 | 1.73 |
| 0x80000302 | EOS 6D | EOS-6D | 1.95 | 1.78 | 2.29 | -1.44 | 0.15 | -0.27 | 1.63 | -0.36 | 0.05 | -0.75 | 1.70 |
| 0x80000325 | EOS 70D | EOS-70D | 2.04 | 1.73 | 2.15 | -1.37 | 0.22 | -0.24 | 1.71 | -0.47 | 0.03 | -0.78 | 1.75 |
| 0x80000326 | EOS Rebel T5i / 700D / Kiss X7i | EOS-700D | 2.03 | 1.75 | 2.26 | -1.45 | 0.20 | -0.29 | 1.71 | -0.42 | 0.04 | -0.75 | 1.70 |
| 0x80000327 | EOS Rebel T5 / 1200D / Kiss X70 / Hi | EOS-1200D | 2.04 | 1.61 | 2.24 | -1.37 | 0.13 | -0.27 | 1.68 | -0.41 | 0.02 | -0.65 | 1.62 |
| 0x80000328 | EOS-1D X Mark II | EOS-1D-X-Mark-II | 1.80 | 1.63 | 2.19 | -1.33 | 0.14 | -0.21 | 1.71 | -0.49 | 0.04 | -0.62 | 1.57 |
| 0x80000331 | EOS M | EOS-M | 1.97 | 1.65 | 2.11 | -1.18 | 0.07 | -0.24 | 1.60 | -0.36 | 0.04 | -0.65 | 1.61 |
| 0x80000346 | EOS Rebel SL1 / 100D / Kiss X7 | EOS-100D | 1.97 | 1.66 | 2.11 | -1.19 | 0.08 | -0.24 | 1.62 | -0.38 | 0.04 | -0.68 | 1.64 |
| 0x80000347 | EOS Rebel T6s / 760D / 8000D | EOS-760D | 2.16 | 1.77 | 2.21 | -1.42 | 0.21 | -0.27 | 1.80 | -0.53 | 0.01 | -0.73 | 1.72 |
| 0x80000349 | EOS 5D Mark IV | EOS-5D-Mark-IV | 1.95 | 1.46 | 2.25 | -1.49 | 0.25 | -0.23 | 1.71 | -0.48 | 0.02 | -0.69 | 1.68 |
| 0x80000355 | EOS M2 | EOS-M2 | 1.96 | 1.67 | 2.08 | -1.12 | 0.04 | -0.20 | 1.59 | -0.39 | 0.05 | -0.56 | 1.51 |
| 0x80000382 | EOS 5DS | EOS-5DS | 2.25 | 1.80 | 2.23 | -1.47 | 0.24 | -0.21 | 1.69 | -0.48 | 0.04 | -0.66 | 1.61 |
| 0x80000393 | EOS Rebel T6i / 750D / Kiss X8i | EOS-750D | 2.14 | 1.76 | 2.19 | -1.39 | 0.20 | -0.26 | 1.79 | -0.53 | 0.02 | -0.73 | 1.71 |
| 0x80000401 | EOS 5DS R | EOS-5DS-R | 2.21 | 1.77 | 2.22 | -1.47 | 0.25 | -0.26 | 1.75 | -0.49 | 0.01 | -0.69 | 1.68 |
| 0x80000404 | EOS Rebel T6 / 1300D / Kiss X80 | EOS-1300D | 1.97 | 1.64 | 2.25 | -1.33 | 0.08 | -0.25 | 1.66 | -0.41 | 0.04 | -0.63 | 1.59 |
| 0x80000406 | EOS 6D Mark II | EOS-6D-Mark-II | 1.98 | 1.73 | 2.12 | -1.27 | 0.15 | -0.21 | 1.65 | -0.44 | 0.01 | -0.57 | 1.56 |
| 0x80000408 | EOS 77D / 9000D | EOS-77D | 1.75 | 1.74 | 2.17 | -1.41 | 0.24 | -0.21 | 1.64 | -0.43 | 0.02 | -0.69 | 1.67 |
| 0x80000417 | EOS Rebel SL2 / 200D / Kiss X9 | EOS-200D | 1.76 | 1.74 | 2.21 | -1.47 | 0.26 | -0.22 | 1.67 | -0.45 | 0.01 | -0.70 | 1.69 |
| 0x80000422 | EOS Rebel T100 / 4000D / 3000D | EOS-4000D | 2.01 | 1.58 | 2.24 | -1.35 | 0.11 | -0.27 | 1.68 | -0.41 | 0.02 | -0.68 | 1.65 |
| 0x80000432 | EOS Rebel T7 / 2000D / 1500D / Kiss X90 | EOS-2000D | 2.07 | 1.75 | 2.22 | -1.40 | 0.18 | -0.26 | 1.79 | -0.52 | 0.01 | -0.74 | 1.73 |

Generic matrix for models that are not in the table: the element-wise mean of the 70 matrices above, rounded to 3 decimals.

| | camera R | camera G | camera B |
| --- | --- | --- | --- |
| sRGB R | 2.039 | -1.142 | 0.104 |
| sRGB G | -0.220 | 1.618 | -0.397 |
| sRGB B | 0.042 | -0.632 | 1.589 |

The generic WB scales are the means of the table: R 2.073, B 1.672.

### Procedure

- R-CR2-30: The decoder must select the table row whose CanonModelID equals the value of MakerNote tag 0x0010, and must use the generic matrix when the tag is absent or no row matches.
- R-CR2-31: The decoder must divide each row of the selected matrix by the sum of that row before use, so that every row sums to 1.
- R-CR2-32: The decoder must compute the output (r, g, b) from the demosaiced, normalized camera values (v_R, v_G, v_B) as r = m11 * v_R + m12 * v_G + m13 * v_B, g = m21 * v_R + m22 * v_G + m23 * v_B, b = m31 * v_R + m32 * v_G + m33 * v_B with the normalized matrix, and must clip r, g and b to the range 0 to 1.
- R-CR2-33: The decoder must deliver linear-light RGB values from 0 to 1 with the sRGB primaries and the D65 white point (S10), without the sRGB transfer function.

Worked example (fixture F1, EOS 5D Mark III, CanonModelID 0x80000285): the row sums are 1.00, 1.00 and 0.99, so row 3 becomes 0.0404040, -0.8080808, 1.7676768. With (v_R, v_G, v_B) = (0.3309481, 0.2790373, 0.1214657):
- r = 2.27 * 0.3309481 - 1.42 * 0.2790373 + 0.15 * 0.1214657 = 0.7512522 - 0.3962330 + 0.0182199 = 0.3732391.
- g = -0.28 * 0.3309481 + 1.67 * 0.2790373 - 0.39 * 0.1214657 = -0.0926655 + 0.4659923 - 0.0473716 = 0.3259552.
- b = 0.0404040 * 0.3309481 - 0.8080808 * 0.2790373 + 1.7676768 * 0.1214657 = 0.0133716 - 0.2254847 + 0.2147121 = 0.0025990.

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and the header (R-CR2-43, R-CR2-4, R-CR2-5).
2. Read IFD0, the IFD chain, the Exif IFD and the MakerNote. Validate every IFD (R-CR2-6, R-CR2-7, R-CR2-9, R-CR2-10, R-CR2-11).
3. Collect the preview candidates now, so that every later failure can fall back (R-CR2-37, R-CR2-38).
4. Read the raw IFD (R-CR2-8) and the required tags (R-CR2-12, R-CR2-13, R-CR2-14).
5. Parse the LJ92 headers and classify the file (R-CR2-3, R-CR2-21). For X1 and X2 use the preview path (R-CR2-2).
6. Determine W, H, the slices, the visible area and the CFA (R-CR2-15, R-CR2-16, R-CR2-18, R-CR2-19).
7. Check all sizes against the file and the limits before allocating memory (section Error handling).
8. Decode the LJ92 stream and place each sample in the raster (R-CR2-20, R-CR2-22, R-CR2-23, R-CR2-17).
9. Determine the black levels, the white level and the WB multipliers (R-CR2-24 to R-CR2-28).
10. Normalize the visible area (R-CR2-29).
11. Demosaic inside the visible area (R-CR2-34, R-CR2-35).
12. Convert to linear sRGB (R-CR2-30 to R-CR2-33).
13. Apply the orientation (R-CR2-36).
14. Deliver the image (R-CR2-33). If any step from 4 to 13 fails, deliver the preview instead (R-CR2-41).

### Demosaicing

The decoder uses the bilinear interpolation of arw.md section Decoding procedure (S8 there and here): for each pixel and each color that the pixel lacks, the missing value is the mean of the pixels of that color among the 8 neighbors of the pixel. Green at a red or blue pixel is the mean of the 4 orthogonal neighbors. Red or blue at a green pixel is the mean of the 2 row or 2 column neighbors of that color. Red at a blue pixel and blue at a red pixel is the mean of the 4 diagonal neighbors. The area for CR2 is the visible area: the decoder ignores neighbors outside it, and the masked and exposed border columns never enter the image.

- R-CR2-34: For each pixel of the visible area, the decoder must set its own color to its normalized value and each missing color to the arithmetic mean of the normalized values of the pixels of that color among its 8 neighbors inside the visible area (R-ARW-54), and must use 0 when no such neighbor exists (R-ARW-55).
- R-CR2-35: The decoder must use the CFA colors of R-CR2-19 at raster coordinates for the demosaicing, so that the phase of the pattern inside the visible area follows from L and T.

Worked example: with value 1 and the S12 borders (L = 52, T = 19), output pixel (0, 0) is raster (52, 19), position 2, green. Its red neighbors are raster (52, 18) and (52, 20). Row 18 lies above the visible area, so the red value is raster (52, 20) alone. Its blue neighbors are raster (51, 19) and (53, 19). Column 51 lies left of the visible area, so the blue value is raster (53, 19) alone.

### Orientation

- R-CR2-36: The decoder must read Orientation from IFD0 tag 0x0112, must use 1 when the tag is absent or outside 1 to 8, and must apply it to the output image with the mappings of R-ARW-58 (values 5 to 8 swap width and height). M13 shows that the raw data and the preview are stored unrotated.

## Embedded preview

### Locations

| Location | Tags | Content (M12) |
| --- | --- | --- |
| IFD0 | StripOffsets 0x0111, StripByteCounts 0x0117, Compression 6 | baseline JPEG preview. Full visible-area size in newer files (for example 5760 x 3840 in S15). 1536 x 1024 to 2496 x 1664 in older EOS files, 1600 x 1200 to 2304 x 1536 in PowerShot files |
| IFD1 | 0x0201 offset, 0x0202 length | 160 x 120 JPEG thumbnail |
| IFD2 | strip | uncompressed RGB image, not a JPEG, not used |

All preview streams are baseline JPEG (SOF0, 8-bit, 3 components), decoded by S2 (baseline sequential DCT process).

### Requirements

- R-CR2-37: The decoder must collect as preview candidates the IFD0 strip when IFD0 has Compression 6 and exactly one StripOffsets and one StripByteCounts value, and the pair (0x0201, 0x0202) of every IFD of the chain, and must keep a candidate only if offset + length lies inside the file, length >= 4 and the first two bytes are FF D8.
- R-CR2-38: The decoder must determine each candidate's size from its first SOF0, SOF1 or SOF2 marker segment and must choose the candidate with the largest width * height.
- R-CR2-39: The decoder must accept a preview stream that has bytes after its EOI marker, or that ends without an EOI marker after its last scan (R-ARW-61).
- R-CR2-40: The decoder must apply the IFD0 Orientation (R-CR2-36) to the preview.
- R-CR2-41: The decoder must deliver the chosen preview when raw decoding fails or the file is X1 or X2, must try the next-largest candidate when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-CR2-42: The decoder must report with every delivered image whether it is the decoded raw image (linear sRGB, R-CR2-33) or a preview (sRGB-encoded JPEG values, not linear).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow and large allocations. "Fail" means: stop raw decoding and use the preview path (R-CR2-41).

### Limits

- R-CR2-43: The decoder must fail for a file shorter than 16 bytes or longer than 2^32 - 1 bytes, and must compute every offset + length in at least 64-bit arithmetic.
- R-CR2-44: The decoder must fail when W or H is 0, when W or H is greater than 65535, or when W * H is greater than 2^27 (134217728).
- R-CR2-45: The decoder must fail when count * type size of a tag that it reads exceeds the file size, before it allocates memory for the values (R-ARW-67).
- R-CR2-46: The decoder must check, before reading, that every value, strip and preview byte range lies completely inside the file.

### Stream checks

- R-CR2-47: The decoder must fail on a raw stream with any of these faults:
  1. The stream does not start with FF D8.
  2. SOF3, DHT or SOS is missing before the entropy-coded data, or a frame or scan header is shorter than its length field states.
  3. The frame or the scan differs from R-CR2-21.
  4. The scan refers to an undefined Huffman table.
- R-CR2-48: The decoder must fail on a DHT whose 16 counts sum to more than 256 or to more than the bytes left in the segment, or whose codes overflow the code space of their length (S2 Annex C, R-ARW-73).
- R-CR2-49: The decoder must fail when a Huffman code is not in the table, when SSSS is greater than 16, or when a marker other than RST appears inside the entropy-coded data before all X * Y MCUs are decoded (R-ARW-74).
- R-CR2-50: The decoder must not read bytes outside the strip, must fail when a Huffman code or its extra bits need bits beyond the entropy-coded data (at a marker or at the end of the strip), and must not fail because of look-ahead bits that it does not consume (R-ARW-75).
- R-CR2-51: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and every sample is clipped to 0 to 65535 in storage and every normalized value to 0 to 1.

### Behaviour on error

- R-CR2-52: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-CR2-53: The decoder must discard a partly decoded raw image on failure and must not deliver it.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order.

### TV-1: header

Bytes 0x49 0x49 0x2A 0x00 0x10 0x00 0x00 0x00 0x43 0x52 0x02 0x00 0x64 0x19 0x01 0x00 (S15): little-endian TIFF, IFD0 at 16, signature "CR", version 2.0, raw IFD at 0x00011964 = 72036 (R-CR2-4, R-CR2-8). Bytes 0x49 0x49 0x2A 0x00 0x08 0x00 0x00 0x00 0x00 ...: no CR2 signature, X3.

### TV-2: IFD entries

- Raw IFD entry 03 01 03 00 01 00 00 00 06 00 00 00: Compression (0x0103), SHORT, count 1, value 6.
- Raw IFD entry E0 C5 04 00 01 00 00 00 03 00 00 00 (S14): CR2CFAPattern, LONG, value 3: G B / R G at raster (0, 0).
- Raw IFD entry 40 C6 03 00 03 00 00 00 A4 AD 00 00 (S14): RawImageSegmentation, SHORT, count 3 (6 bytes > 4), values at offset 0xADA4 = 44452: 2, 1936, 1920.
- MakerNote entry 10 00 04 00 01 00 00 00 85 02 00 80 (S15): CanonModelID 0x80000285 (EOS 5D Mark III).
- MakerNote entry 01 40 07 00 00 14 00 00 92 0C 00 00 (S18): ColorData, UNDEFINED, count 5120 bytes = 2560 signed 16-bit values at offset 0x0C92 = 3218. Its first bytes FD FF give version -3.

### TV-3: LJ92 stream with 2 components (layout A rules)

A complete stream with P = 14, X = 2, Y = 2, Nf = 2, and the Huffman table of the 14-bit Canon streams (M2, the same bytes as in S15):

FF D8
FF C4 00 42 00 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E
FF C3 00 0E 0E 00 02 00 02 02 01 11 00 02 11 00
FF DA 00 0A 02 01 00 02 10 01 00 00
FF 00 20 07 FC 80 0F 5E 3C 7A 5B E7
FF D9

Huffman table (both tables 0 and 1): BITS = 0, 1, 4, 2, 3, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, HUFFVAL = 6, 4, 8, 5, 7, 3, 9, 0, 10, 2, 1, 12, 11, 13, 14. Codes (S2 Annex C): SSSS 6 = 00, 4 = 010, 8 = 011, 5 = 100, 7 = 101, 3 = 1100, 9 = 1101, 0 = 11100, 10 = 11101, 2 = 11110, 1 = 111110, 12 = 1111110, 11 = 11111110, 13 = 111111110, 14 = 1111111110.

| MCU (u, v) | Component | Prediction | Code | Extra bits | Difference | Value |
| --- | --- | --- | --- | --- | --- | --- |
| (0, 0) | 1 | 8192 (first sample) | 111111110 | 0100000000001 | -6142 | 2050 |
| (0, 0) | 2 | 8192 (first sample) | 111111110 | 0100000000000 | -6143 | 2049 |
| (1, 0) | 1 | 2050 (left) | 11110 | 10 | +2 | 2052 |
| (1, 0) | 2 | 2049 (left) | 11110 | 00 | -3 | 2046 |
| (0, 1) | 1 | 2050 (above) | 11110 | 00 | -3 | 2047 |
| (0, 1) | 2 | 2049 (above) | 11110 | 10 | +2 | 2051 |
| (1, 1) | 1 | 2047 (left) | 010 | 1101 | +13 | 2060 |
| (1, 1) | 2 | 2051 (left) | 11110 | 01 | -2 | 2049 |

The 88 bits fill 11 bytes exactly, so no padding bits follow. The first byte is FF and is followed by the stuffed byte 00. Sample sequence (R-CR2-22): 2050 2049 2052 2046 2047 2051 2060 2049.

### TV-4: LJ92 stream with 4 components and the 12-bit Canon table

P = 12, X = 1, Y = 2, Nf = 4, tables 0 and 1 with BITS = 0, 2, 2, 2, 3, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0 and HUFFVAL = 4, 5, 3, 6, 7, 2, 8, 0, 1, 9, 10, 12, 11 (the table of S12):

FF D8
FF C4 00 3E 00 00 02 02 02 03 01 01 01 01 00 00 00 00 00 00 00 04 05 03 06 07 02 08 00 01 09 0A 0C 0B 01 00 02 02 02 03 01 01 01 01 00 00 00 00 00 00 00 04 05 03 06 07 02 08 00 01 09 0A 0C 0B
FF C3 00 14 0C 00 02 00 01 04 01 11 00 02 11 00 03 11 00 04 11 00
FF DA 00 0E 04 01 00 02 10 03 00 04 10 01 00 00
FF 00 12 BF F1 01 FF 00 10 5F F0 FE 28 72 3D
FF D9

Codes: SSSS 4 = 00, 5 = 01, 3 = 100, 6 = 101, 7 = 1100, 2 = 1101, 8 = 11100, 0 = 11101, 1 = 11110, 9 = 111110, 10 = 1111110, 12 = 11111110, 11 = 111111110. Line 0 predicts every component from 2^11 = 2048 (each component is the first sample of its own plane): differences -1748, -1790, -1786, -1793 (category 11). Line 1 predicts from the sample above: differences +10, -8, +8, +1. Sample sequence: 300 258 262 255 310 250 270 256. With W = 2 and one slice this is the raster 300 258 / 262 255 / 310 250 / 270 256 (rows), that is H = 4 for X * Y * Nf = 8.

### TV-5: slice mapping

n = 1, a = 4, b = 2, W = 6, H = 4 (24 samples). Sample index s and raster position (x, y):

| s | 0 | 3 | 4 | 5 | 15 | 16 | 17 | 18 | 19 | 23 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| (x, y) | (0, 0) | (3, 0) | (0, 1) | (1, 1) | (3, 3) | (4, 0) | (5, 0) | (4, 1) | (5, 1) | (5, 3) |

S14 (n = 2, a = 1936, b = 1920, H = 3804): s = 1935 -> (1935, 0). s = 1936 -> (0, 1). s = 7364543 -> (1935, 3803). s = 7364544 -> (1936, 0). s = 14729088 -> (3872, 0). s = 14731008 -> (3872, 1). s = 22032767 -> (5791, 3803).

### TV-6: layout C

S17: X = 3192, Y = 2112, Nf = 4, tag 0xC640 = 0, 6384, 6384, SensorInfo W = 6384, H = 4224. One slice. Frame line v, MCU u, component k -> s -> raster: (0, 0, 1) -> 0 -> (0, 0). (0, 1595, 4) -> 6383 -> (6383, 0). (0, 1596, 1) -> 6384 -> (0, 1). (0, 3191, 4) -> 12767 -> (6383, 1). (1, 0, 1) -> 12768 -> (0, 2).

S19: tag 0xC640 = 0, 0, 4176 and SensorInfo W = 4192: one slice of width 4192 (R-CR2-16).

### TV-7: SensorInfo

S15 SensorInfo bytes: 22 00 20 17 6E 0F 01 00 01 00 8C 00 60 00 0B 17 5F 0F 00 00 00 00 00 00 00 00 ... give 34, 5920, 3950, 1, 1, 140, 96, 5899, 3935, 0, 0, 0, 0: W = 5920, H = 3950, visible area columns 140 to 5899 and rows 96 to 3935, output 5760 x 3840, black rectangle columns 0 to 69 and rows 96 to 3935.

S18 SensorInfo bytes: 22 00 A0 0E E0 0A 01 00 01 00 48 00 18 00 87 0E C7 0A 28 00 18 00 2B 00 C7 0A give 34, 3744, 2784, 1, 1, 72, 24, 3719, 2759, 40, 24, 43, 2759: black rectangle columns 40 to 43 and rows 24 to 2759 (rule 1, left of L = 72).

### TV-8: white balance from ColorData

- S15: version 10 at index 0 (bytes 0A 00), index 63 bytes 99 06 00 04 00 04 A9 08: 1689, 1024, 1024, 2217. mR = 1.6494141, mB = 2.1650391.
- S12: 582 values, index 25 bytes 7A 08 FA 03 FC 03 DB 06: 2170, 1018, 1020, 1755. G = 1019, mR = 2.1295388, mB = 1.7222767.
- S18: type UNDEFINED, 2560 values, version -3, index 71 bytes CB 05 79 03 79 03 82 07: 1483, 889, 889, 1922. mR = 1.6681665, mB = 2.1619798.
- Version 99 or no ColorData, CanonModelID 0x80000285: table scales mR = 2.02, mB = 1.73. Unknown CanonModelID: mR = 2.073, mB = 1.672.

### TV-9: black and white level

- Black: the values 1024, 1030, 1022, 2149, 1025, 1023 of one CFA position give the sorted list 1022 1023 1024 1025 1030 2149, n = 6, rank floor(5 / 2) = 2, black 1024.
- White, P = 14: M = 15464, C = 1899: w = 15464. M = 12223, C = 1: w = 16383. M = 6048, C = 36: M < 8192, w = 16383. M = 16383, C = 622764: w = 16383.
- White, P = 12: M = 3398, C = 3903: w = 3398. M = 1756, C = 24: M < 2048, w = 4095.

### TV-10: first samples of a camera stream

S15, entropy-coded data at file offset 3309734: FF 00 1F EF FC 77 3A 2A 4A 61 76 C6.

| Sample s | Component | Prediction | Code | SSSS | Extra bits | Difference | Value | Raster |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 1 | 8192 | 111111110 | 13 | 0011111111011 | -6148 | 2044 | (0, 0) |
| 1 | 2 | 8192 | 111111110 | 13 | 0011101110011 | -6284 | 1908 | (1, 0) |
| 2 | 1 | 2044 | 101 | 7 | 0001010 | -117 | 1927 | (2, 0) |
| 3 | 2 | 1908 | 100 | 5 | 10010 | +18 | 1926 | (3, 0) |
| 4 | 1 | 1927 | 100 | 5 | 11000 | +24 | 1951 | (4, 0) |
| 5 | 2 | 1926 | 010 | 4 | 1110 | +14 | 1940 | (5, 0) |

### TV-11: color matrix

EOS 5D Mark III row of the table, normalized: (2.27, -1.42, 0.15), (-0.28, 1.67, -0.39), (0.0404040, -0.8080808, 1.7676768). Input (0.3309481, 0.2790373, 0.1214657), output (0.3732391, 0.3259552, 0.0025990). A neutral input (0.25, 0.25, 0.25) gives (0.25, 0.25, 0.25) for every normalized matrix.

### TV-12: camera files

Check values for the saved files (.testdata/cr2/). "Raster MD5" is the MD5 digest of the reassembled raster as W * H unsigned 16-bit little-endian values, row by row. "Mean output" is the mean linear sRGB value over the output image before orientation (bilinear demosaicing, R-CR2-34). An implementation must reproduce the raster digest exactly and the means within 0.0005.

| File | W x H | Raster MD5 | Black b(0..3) | w | mR, mB | Mean output r, g, b |
| --- | --- | --- | --- | --- | --- | --- |
| EOS_350D_RAW.CR2 | 3516 x 2328 | 8fe9f0449e5fd5946b297dbaac580e43 | 257 257 257 257 | 4095 | 2.1295388, 1.7222767 | 0.08807, 0.08978, 0.08126 |
| EOS_20D_RAW.CR2 | 3596 x 2360 | c45df124a2211bc8576b8aec394341b0 | 128 128 128 128 | 4095 | 1.5506993, 2.1739510 | 0.12976, 0.07774, 0.04426 |
| EOS_5D_Mark_II_RAW.CR2 | 5792 x 3804 | 6d66209bcefed3d77eb723e3371826e6 | 1024 1024 1024 1024 | 16383 | 2.4267578, 1.5517578 | 0.07773, 0.06996, 0.06497 |
| EOS_5D_Mark_III_RAW.CR2 | 5920 x 3950 | a18235d6abced4df9948b6fbe91aba7c | 2048 2048 2048 2047 | 15464 | 1.6494141, 2.1650391 | 0.15664, 0.08921, 0.04530 |
| EOS_80D_RAW.CR2 | 6288 x 4056 | c939b7ef677c1e137d0f6ff9e9cfda35 | 512 512 513 513 | 16383 | 1.7294922, 1.7841797 | 0.11926, 0.11899, 0.11609 |
| EOS_6D_Mark_II_RAW.CR2 | 6384 x 4224 | d5c08b5bfdc1a2d88034fefc0a467b16 | 513 513 513 513 | 16383 | 2.0361328, 1.7744141 | 0.06500, 0.06222, 0.04481 |
| PowerShot_G12_RAW.CR2 | 3744 x 2784 | bcead3d6a125a5ea5c4f3596fced0023 | 128 128 128 128 | 4095 | 1.6681665, 2.1619798 | 0.06161, 0.09369, 0.14029 |
| PowerShot_S120_RAW.CR2 | 4192 x 3062 | 649bf40b6cd80ea03b4d2f08ef8f06b4 | 130 130 130 130 | 4095 | 2.1617647, 1.6323529 | 0.22366, 0.12664, 0.07817 |

Raster spot values: S12 (0, 0) = 137, (1758, 0) = 136, (1758, 1164) = 1288. S14 (2896, 1902) = 2418. S15 row 0 starts 2044 1908 1927 1926 1951 1940 2231 2098. S17 (3192, 2112) = 1041, (6383, 4223) = 562.

The two sRAW files (X1) must take the preview path and deliver the 5760 x 3840 preview.

### Fixture recipes

The fixtures are complete small CR2 files for the decoder tests. All multi-byte numbers are little-endian. Each file has this layout: the 16-byte header, IFD0 at 16 with 4 entries (Make, Model, Orientation, Exif IFD pointer) and next-IFD offset 0, the Exif IFD with one entry 0x927C, the MakerNote IFD with 3 entries (0x0010 LONG, 0x00E0 SHORT 17, 0x4001), the raw IFD with Compression 6, StripOffsets, StripByteCounts, 0xC5E0 and (except F4) 0xC640, and the LJ92 stream. The files have no preview. All streams use the Huffman table of TV-3 for both tables. ColorData has 68 values (F1 to F3, type SHORT) or 76 values (F4, type UNDEFINED, count 152), all 0 except index 0 (version), the four WB values and the following temperature value. Each raster holds the black value outside the visible area and a flat CFA image inside it, so every output pixel has the same value.

| Fixture | Layout | W x H | Frame X, Y, Nf, P | 0xC640 | 0xC5E0 | L, T, R, B | Black rectangle | Black | R, G, B samples | CanonModelID | ColorData version, index, WB | Orientation |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| F1 | A | 20 x 8 | 10, 8, 2, 14 | 1, 10, 10 | 1 | 8, 2, 19, 7 | columns 0 to 3, rows 2 to 7 | 2048 | 4048, 6048, 3048 | 0x80000285 | 10, 63, 2429 1024 1024 1783 | 1 |
| F2 | C | 16 x 8 | 8, 4, 4, 14 | 0, 16, 16 | 1 | 8, 2, 15, 7 | columns 0 to 3, rows 2 to 7 | 512 | 6000, 15000, 7000 | 0x80000406 | 15, 63, 2085 1024 1024 1817 | 1 |
| F3 | B | 20 x 6 | 5, 6, 4, 12 | 2, 7, 6 | 3 | 8, 2, 19, 5 | columns 0 to 3, rows 2 to 5 | 256 | 1256, 1756, 756 | 0x80009999 (not in the table) | 99 (unknown), none | 1 |
| F4 | A, no slice tag | 12 x 6 | 6, 6, 2, 12 | absent | 4 | 2, 2, 9, 5 | SensorInfo 9 to 12 = 10, 2, 11, 5 | 128 | 1128, 1628, 628 | 0x02920000 | -3, 71, 1483 889 889 1922 | 6 |

Expected results:

| Fixture | w | mR, mB (source) | Normalized v_R, v_G, v_B | Output size | Every output pixel (linear sRGB) |
| --- | --- | --- | --- | --- | --- |
| F1 | 16383 (M = 6048 < 8192) | 2.3720703, 1.7412109 (ColorData) | 0.3309481, 0.2790373, 0.1214657 | 12 x 6 | 0.3732391, 0.3259552, 0.0025990 |
| F2 | 15000 (M = 15000, C = 24) | 2.0361328, 1.7744141 (ColorData) | 0.7712795, 1.0000000, 0.7946161 | 8 x 6 | 0.4843049, 1.0000000, 0.6773140 |
| F3 | 4095 (M = 1756 < 2048) | 2.073, 1.672 (generic) | 0.5399844, 0.3907268, 0.2177650 | 12 x 4 | 0.6767890, 0.4265201, 0.1218906 |
| F4 | 4095 | 1.6681665, 2.1619798 (ColorData) | 0.4205108, 0.3781195, 0.2724956 | 4 x 8 after orientation 6 (8 x 4 before) | 0.4662651, 0.4032254, 0.2146392 |

F1, complete file of 652 bytes. The raw IFD is at 0x148 (header bytes 12 to 15), the LJ92 stream at 0x190 (252 bytes).

0000: 49 49 2A 00 10 00 00 00 43 52 02 00 48 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 16 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 01 00 00 00 69 87 04 00 01 00 00 00 62 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 45 4F 53 20 35 44 20 4D 61 72 6B 20 49 49
0060: 49 00 01 00 7C 92 07 00 D4 00 00 00 74 00 00 00
0070: 00 00 00 00 03 00 10 00 04 00 01 00 00 00 85 02
0080: 00 80 E0 00 03 00 11 00 00 00 9E 00 00 00 01 40
0090: 03 00 44 00 00 00 C0 00 00 00 00 00 00 00 22 00
00A0: 14 00 08 00 01 00 01 00 08 00 02 00 13 00 07 00
00B0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00C0: 0A 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 7D 09
0140: 00 04 00 04 F7 06 50 14 05 00 03 01 03 00 01 00
0150: 00 00 06 00 00 00 11 01 04 00 01 00 00 00 90 01
0160: 00 00 17 01 04 00 01 00 00 00 FC 00 00 00 E0 C5
0170: 04 00 01 00 00 00 01 00 00 00 40 C6 03 00 03 00
0180: 00 00 8A 01 00 00 00 00 00 00 01 00 0A 00 0A 00
0190: FF D8 FF C4 00 42 00 00 01 04 02 03 01 01 01 01
01A0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01B0: 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01
01C0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01D0: 02 01 0C 0B 0D 0E FF C3 00 0E 0E 00 08 00 0A 02
01E0: 01 11 00 02 11 00 FF DA 00 0A 02 01 00 02 10 01
01F0: 00 00 FF 00 1F FF 00 FC 7F FE 73 9C E7 39 CE 73
0200: 9C E7 39 CE 73 9C E7 39 CE 73 FB E8 7E FA 0F E0
0210: 5F F8 17 F9 CE 73 9C FD F4 1D FA 39 CE 73 9C E7
0220: 3F BE 87 EF A0 FE 05 FF 00 81 7F 9C E7 39 CF DF
0230: 41 DF A3 9C E7 39 CE 73 FB E8 7E FA 0F E0 5F F8
0240: 17 F9 CE 73 9C FD F4 1D FA 39 CE 73 9C E7 39 CE
0250: 73 9C E7 39 CE 73 FB E8 7E FA 0E 73 9C E7 39 CF
0260: EF A1 F9 11 F9 CE 73 9C E7 39 CE 73 9C E7 39 CF
0270: EF A1 F9 11 F9 CE 73 9C E7 39 CE 73 9C E7 39 CF
0280: EF A1 F9 11 F9 CE 73 9C E7 3F FF D9

F2, complete file of 697 bytes (layout C: each frame line of 32 samples holds raster rows 2v and 2v + 1):

0000: 49 49 2A 00 10 00 00 00 43 52 02 00 48 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 15 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 01 00 00 00 69 87 04 00 01 00 00 00 62 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 45 4F 53 20 36 44 20 4D 61 72 6B 20 49 49
0060: 00 00 01 00 7C 92 07 00 D4 00 00 00 74 00 00 00
0070: 00 00 00 00 03 00 10 00 04 00 01 00 00 00 06 04
0080: 00 80 E0 00 03 00 11 00 00 00 9E 00 00 00 01 40
0090: 03 00 44 00 00 00 C0 00 00 00 00 00 00 00 22 00
00A0: 10 00 08 00 01 00 01 00 08 00 02 00 0F 00 07 00
00B0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00C0: 0F 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 25 08
0140: 00 04 00 04 19 07 B4 12 05 00 03 01 03 00 01 00
0150: 00 00 06 00 00 00 11 01 04 00 01 00 00 00 90 01
0160: 00 00 17 01 04 00 01 00 00 00 29 01 00 00 E0 C5
0170: 04 00 01 00 00 00 01 00 00 00 40 C6 03 00 03 00
0180: 00 00 8A 01 00 00 00 00 00 00 00 00 10 00 10 00
0190: FF D8 FF C4 00 42 00 00 01 04 02 03 01 01 01 01
01A0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01B0: 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01
01C0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01D0: 02 01 0C 0B 0D 0E FF C3 00 14 0E 00 04 00 08 04
01E0: 01 11 00 02 11 00 03 11 00 04 11 00 FF DA 00 0E
01F0: 04 01 00 02 10 03 00 04 10 01 00 00 FF 00 07 FF
0200: 00 FC 1F FF 00 F0 7F FF 00 C1 FF 00 E7 39 CE 73
0210: 9C E7 39 CE 73 9C E7 39 CE 73 9C E7 39 CE 73 9C
0220: E7 39 CF F5 5C 3F EE 26 3F D5 70 FF 00 B8 98 E7
0230: 39 CF F2 A3 FF 00 E1 D9 FF 00 CA 8F FF 00 87 67
0240: E7 39 CF FB 89 8F F6 56 3F EE 26 3F D9 58 E7 39
0250: CE 73 9C E7 39 CF F5 5C 3F EE 26 3F D5 70 FF 00
0260: B8 98 E7 39 CF F2 A3 FF 00 E1 D9 FF 00 CA 8F FF
0270: 00 87 67 E7 39 CF FB 89 8F F6 56 3F EE 26 3F D9
0280: 58 E7 39 CE 73 9C E7 39 CF F5 5C 3F EE 26 3F D5
0290: 70 FF 00 B8 98 E7 39 CF F2 A3 FF 00 E1 D9 FF 00
02A0: CA 8F FF 00 87 67 E7 39 CF FB 89 8F F6 56 3F EE
02B0: 26 3F D9 58 E7 39 CF FF D9

F3, complete file of 638 bytes:

0000: 49 49 2A 00 10 00 00 00 43 52 02 00 42 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 0F 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 01 00 00 00 69 87 04 00 01 00 00 00 5C 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 45 4F 53 20 54 45 53 54 00 00 01 00 7C 92
0060: 07 00 D4 00 00 00 6E 00 00 00 00 00 00 00 03 00
0070: 10 00 04 00 01 00 00 00 99 99 00 80 E0 00 03 00
0080: 11 00 00 00 98 00 00 00 01 40 03 00 44 00 00 00
0090: BA 00 00 00 00 00 00 00 22 00 14 00 06 00 01 00
00A0: 01 00 08 00 02 00 13 00 05 00 00 00 00 00 00 00
00B0: 00 00 00 00 00 00 00 00 00 00 63 00 00 00 00 00
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 7D 09 00 04 00 04 F7 06
0140: 50 14 05 00 03 01 03 00 01 00 00 00 06 00 00 00
0150: 11 01 04 00 01 00 00 00 8A 01 00 00 17 01 04 00
0160: 01 00 00 00 F4 00 00 00 E0 C5 04 00 01 00 00 00
0170: 03 00 00 00 40 C6 03 00 03 00 00 00 84 01 00 00
0180: 00 00 00 00 02 00 07 00 06 00 FF D8 FF C4 00 42
0190: 00 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00
01A0: 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E
01B0: 01 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00
01C0: 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E
01D0: FF C3 00 14 0C 00 06 00 05 04 01 11 00 02 11 00
01E0: 03 11 00 04 11 00 FF DA 00 0E 04 01 00 02 10 03
01F0: 00 04 10 01 00 00 FE 1F FF 00 C3 FF 00 F8 7F FF
0200: 00 0F FE 73 9C E7 39 CE 73 9C E7 39 CE 73 9C E7
0210: 39 CE 73 9C E7 39 CE 73 9C E7 39 CE 73 9C E7 39
0220: CE 73 9C E7 F5 DC DF A7 F5 DC DF A7 F5 DC DF A7
0230: 37 E9 CD FA 7F 5D CE 73 A0 BF 34 17 CD FA 73 9F
0240: C8 8F 7E 9C DF A7 37 E9 FD 77 3A 0B FF 00 22 3E
0250: 82 FF 00 C8 8F 9C E7 39 CE 73 FA EE 6F D3 FA EE
0260: 6F D3 7E 9D 05 F9 CD 05 F7 E8 E7 37 E9 D0 5F 7E
0270: 9D 05 F9 CD 05 F7 E8 D0 5F 7E 8E 73 FF D9

F4, complete file of 571 bytes (ColorData type 7 at 0xBE, black rectangle right of the visible area):

0000: 49 49 2A 00 10 00 00 00 43 52 02 00 56 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 14 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 06 00 00 00 69 87 04 00 01 00 00 00 60 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 50 6F 77 65 72 53 68 6F 74 20 47 31 32 00
0060: 01 00 7C 92 07 00 E4 00 00 00 72 00 00 00 00 00
0070: 00 00 03 00 10 00 04 00 01 00 00 00 00 00 92 02
0080: E0 00 03 00 11 00 00 00 9C 00 00 00 01 40 07 00
0090: 98 00 00 00 BE 00 00 00 00 00 00 00 22 00 0C 00
00A0: 06 00 01 00 01 00 02 00 02 00 09 00 05 00 0A 00
00B0: 02 00 0B 00 05 00 00 00 00 00 00 00 00 00 FD FF
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0140: 00 00 00 00 00 00 00 00 00 00 00 00 CB 05 79 03
0150: 79 03 82 07 50 14 04 00 03 01 03 00 01 00 00 00
0160: 06 00 00 00 11 01 04 00 01 00 00 00 8C 01 00 00
0170: 17 01 04 00 01 00 00 00 AF 00 00 00 E0 C5 04 00
0180: 01 00 00 00 04 00 00 00 00 00 00 00 FF D8 FF C4
0190: 00 42 00 00 01 04 02 03 01 01 01 01 01 00 00 00
01A0: 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B
01B0: 0D 0E 01 00 01 04 02 03 01 01 01 01 01 00 00 00
01C0: 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B
01D0: 0D 0E FF C3 00 0E 0C 00 06 00 06 02 01 11 00 02
01E0: 11 00 FF DA 00 0A 02 01 00 02 10 01 00 00 FE 0F
01F0: FF 00 C1 FF 00 9C E7 39 CE 73 9C E7 39 CE 73 9C
0200: E7 39 CE 73 FA EE 77 E8 E7 39 CE 73 F9 11 F4 17
0210: E7 37 E9 FD 77 39 CE 73 9C D0 5F F2 23 E7 3F AE
0220: E7 7E 8E 73 9C E7 3F 91 1F 41 7E 73 7E 9F D7 73
0230: 9C E7 39 CD 05 FF 00 22 3F FF D9

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behaviour the decoder must take.

- OQ-1 sRAW and mRAW (X1). The YCbCr to RGB conversion, the white balance of the YCbCr data and the slice layout of 4:2:0 data (mRAW) are not established (M10). Fallback: preview path. The previews of the two S15 sRAW files are full size (5760 x 3840).
- OQ-2 White level. The file carries no white level that matches the measured clip values (M6). R-CR2-26 estimates it from the data. An image whose clipped area is smaller than 16 samples keeps w = 2^P - 1, so clipped highlights can show a tint. Fallback: R-CR2-26.
- OQ-3 ColorData black level, white level and calibration values (PerChannelBlackLevel, NormalWhiteLevel, SpecularWhiteLevel, LinearityUpperMargin, ColorCalib). S5 lists several positions in ColorData4 without the selecting rule, and the meaning of ColorCalib is not documented. Fallback: not used. Black from the masked area (R-CR2-25), white by R-CR2-26, matrix from the table.
- OQ-4 ColorData2 (EOS-1D Mark II, EOS-1Ds Mark II). S5 places the as-shot white balance at index 34, but the only archive samples are not CC0 and were not measured, so the length that identifies this layout is not known. Fallback: R-CR2-28 source 2 or 3.
- OQ-5 CR2CFAPattern values 2 and 4. Taken from S4, not observed. Fallback: R-CR2-19 as written.
- OQ-6 Color matrices. S6 gives D50 matrices with 2 decimals for 70 models. EOS 800D, PowerShot G1 X Mark III and G7 X Mark II have no color data on S6, PowerShot SX40 HS has no page, and the EOS 80D page is malformed (M16). The D50 matrix is used for every scene illuminant. Fallback: R-CR2-30 with the generic matrix (M9: EOS 80D with the generic matrix 0.032).
- OQ-7 Lens corrections. PowerShot cameras correct lens distortion and vignetting in their JPEG (M14). The parameters are not documented. Fallback: no lens correction.
- OQ-8 Aspect-ratio crops. The camera can record an aspect ratio other than the sensor ratio (S5 tag AspectInfo, 0x009A). Its use for the raw crop is not established. Fallback: the whole visible area (R-CR2-18).
- OQ-9 Big-endian CR2 files, other header versions and files without tag 0xC640 other than EOS 20D. Not observed except as stated. Fallback: R-CR2-4 and R-CR2-16.
- OQ-10 Restart markers, predictors other than 1 and Pt other than 0. Not observed (M2). Fallback: X2 for predictors and Pt (R-CR2-21), S2 behaviour for restart markers (R-CR2-23).
- OQ-11 Dual pixel data (IFD4, tag 0xC6DD). Its meaning is not documented. Fallback: ignored (M11).
- OQ-12 Black level of models whose masked columns hold unusual values. The lower median handles the measured cases (M5). A model whose columns 0 to floor(L / 2) - 1 hold image data and that has no SensorInfo rectangle would get a wrong black level. None was observed in 93 models.
- OQ-13 Tags 0xC5D8 and IFD2 tag 0xC6DC. Meaning unknown. Fallback: ignored.
- OQ-14 Highlights. White-balanced values above 1 are clipped. No highlight reconstruction is specified. Fallback: R-CR2-29.
- OQ-15 PowerShot A480, SX40 HS and SX150 IS were not measured (S21). Fallback: classification as usual. A file that fails a check takes the preview path.
