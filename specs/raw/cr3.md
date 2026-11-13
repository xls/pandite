# Canon CR3 raw format: decoder specification

Document: specs/raw/cr3.md. Version 1.0. Date 2026-11-14.
Status: normative input for the pandite CR3 decoder (clean-room step 1 of 2).

This document describes the Canon CR3 raw file format as facts. It is written for an implementer who has only this document and the public standards it cites: the QuickTime File Format specification (S1, S2), TIFF 6.0 (S4), ITU-T T.81 (S5) and JFIF 1.02 (S6). Every normative statement traces to a source in the Sources section or to a measurement in the measurement record. The document contains no code. It describes algorithms in prose, tables and formulas.

### Outcome of this version

This version does not specify the decoding of CR3 sensor data. All 110 measured CR3 files store their sensor data with the Canon CRX codec (M3, M13), in a lossless mode (camera setting RAW) or a lossy mode (camera setting C-RAW, Compact RAW). No permitted public source describes CRX bit-exactly, and the measurements of this document establish only the bytes that identify the mode. Both modes are therefore out of scope (section Scope and variants).

The document specifies in full: the container, the metadata, the exact location of the black levels, white levels and as-shot white balance, and the route to the embedded previews. The decoder delivers the largest embedded JPEG preview for every CR3 file that has one. In 104 of the 110 public sample files measured (M1, M8), that preview is a full-resolution JPEG of the same size as the camera JPEG (for example 6000 x 4000 for the EOS M50). The other 6 files (EOS R5 Mark II and EOS R8) hold all their previews HEVC coded. HEVC decoding is also out of scope, so the decoder reports an error for these files (variant X5).

### Conventions

- Requirements have the form R-CR3-n. Each requirement is one testable statement with "must". Text outside requirements explains or gives examples.
- "The decoder" is the program that implements this document.
- All byte offsets count from byte 0 of the file, unless a section states another origin.
- Numbers with the prefix 0x are hexadecimal. Byte sequences are written as hexadecimal bytes in file order, for example 0x63 0x72 or "63 72".
- u8, u16, u32 and u64 are unsigned integers of 1, 2, 4 and 8 bytes. In boxes they are big-endian (most significant byte first, S1). Fields marked "LE" are little-endian (least significant byte first). TIFF blocks use the byte order of their own TIFF header (S4).
- A four-character code in double quotes, for example "moov", means the 4 ASCII bytes of the code (6D 6F 6F 76).
- floor(a / b) is integer division rounded toward minus infinity.
- W x H means width x height in pixels.

### Glossary

- CR3: the raw file format of current Canon cameras, for example the EOS M50, the EOS R series and the PowerShot V1 (M17). The file extension is .CR3 (S10).
- Box: the basic data unit of the file. S2 calls it an atom and states that an atom is functionally identical to a box of the ISO specifications for MPEG-4. A box has a header (size and type) and a payload.
- Payload: the bytes of a box after its header.
- Full box: a box whose payload starts with a 1-byte version and 3 bytes of flags (S2: "full atom").
- uuid box: a box of type "uuid". Its payload starts with a 16-byte extended type that identifies a private extension (S3, M1).
- Container box: a box whose payload is a sequence of boxes.
- Track: a "trak" box and the media data it describes.
- Sample: one unit of media data of a track. In CR3 one sample is one image or one block of metadata. Samples are stored in the "mdat" box.
- Sample entry: the first entry of the "stsd" box of a track. It names the format of the samples.
- CRAW: the format code of the sample entry of every image track of a CR3 file (M3).
- JPEG track, CRX track, HEVC track: an image track whose CRAW sample entry contains a child box "JPEG", "CMP1" or "HEVC" (M3).
- Metadata track: a track with the handler type "meta" and the sample entry format "CTMD" (Canon timed metadata, S7, S8).
- CRX: the Canon codec of the sensor data. The file brand is "crx " (M1).
- Canon metadata box: the uuid box inside "moov" with the extended type 85 C0 B6 87 82 0F 11 E0 81 11 F4 CE 46 2B 6A 48 (M2). S7 lists its children CNCV, CMT1 to CMT4 and THMB.
- Preview holder box: the top-level uuid box with the extended type EA F4 2B 5E 1C 98 4B 88 B9 FB B7 DC 40 6E 4D 16. It holds the "PRVW" box (M6).
- CMT1, CMT2, CMT3, CMT4: boxes that hold a TIFF structure with IFD0, the Exif IFD, the Canon maker note and the GPS IFD (S7).
- THMB, PRVW: boxes that hold the 160 x 120 thumbnail and the 1620 x 1080 (or 1440 x 1080) preview (S7, S8, M5, M6).
- ColorData: the Canon maker-note tag 0x4001 (S7). It holds the white balance levels and the sensor levels.
- IFD: image file directory, the TIFF 6.0 tag table (S4 section 2).
- Preview path: the procedure that shows an embedded preview in place of decoded sensor data (section Embedded preview).
- RAW, C-RAW: the two camera settings for raw images. "Compact RAW produces RAW images with smaller file sizes than RAW" (S10).
- Dual Pixel RAW: a camera setting that "produces special Dual Pixel RAW images containing dual pixel information from the image sensor" (S11).

## Sources

Every row below names a public document or a measurement made by the author for this document. The column "Used in" names the sections that rely on the row.

| Id | Title | URL | Kind | Accessed | Used in |
| --- | --- | --- | --- | --- | --- |
| S1 | Apple Computer, QuickTime File Format, 2001-03-01 (PDF) | https://developer.apple.com/standards/qtff-2001.pdf | vendor | 2026-11-14 | Container layout (atom header, size values 0 and 1, sample description, video sample description, hdlr, stsz, stco, co64), Error handling |
| S2 | Apple, QuickTime File Format documentation: pages Atoms, File type compatibility atom, Sample description atom, Sample size atom, Chunk offset atom | https://developer.apple.com/documentation/quicktime-file-format | vendor | 2026-11-14 | Glossary (atom = box, full atom), Container layout (co64 replaces stco, file offsets, unknown atoms are skipped) |
| S3 | Library of Congress, Sustainability of Digital Formats: ISO Base Media File Format (fdd000079) | https://www.loc.gov/preservation/digital/formats/fdd/fdd000079.shtml | community documentation | 2026-11-14 | Container layout (all data in boxes, unrecognized boxes are skipped, private extensions use the uuid box type) |
| S4 | TIFF Revision 6.0, Final, June 3 1992, Adobe (Aldus) | https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf | official | 2026-11-14 | Metadata tags (IFD, field types, Orientation), Embedded preview (orientation) |
| S5 | ITU-T Recommendation T.81 (1992), Digital compression and coding of continuous-tone still images | https://www.w3.org/Graphics/JPEG/itu-t81.pdf | standard | 2026-11-14 | Compression (JPEG previews), Embedded preview, Error handling |
| S6 | E. Hamilton, JPEG File Interchange Format, Version 1.02, C-Cube Microsystems, 1992 | https://www.w3.org/Graphics/JPEG/jfif3.pdf | official | 2026-11-14 | Compression (YCbCr to RGB of the previews), Color conversion |
| S7 | ExifTool Tag Names: Canon tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/Canon.html | community documentation | 2026-11-14 | Glossary, Container layout (Canon uuid tags), Metadata tags (CMT1 to CMT4, CMP1, CTMD record types, ExifInfo, SensorInfo, ColorData9 to ColorData12), Levels and white balance |
| S8 | ExifTool Tag Names: QuickTime tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/QuickTime.html | community documentation | 2026-11-14 | Container layout (CTMD timed metadata, children CMP1, CDI1 and JPEG of the visual sample description) |
| S9 | ExifTool Tag Names: EXIF tags table (metadata tag reference documentation) | https://exiftool.org/TagNames/EXIF.html | community documentation | 2026-11-14 | Metadata tags (Make, Model, Orientation, ImageWidth, ImageHeight, ColorSpace) |
| S10 | Canon, EOS R6 Product Manual, Image Quality | https://cam.start.canon/en/C004/manual/html/UG-03_Shooting-1_0030.html | vendor | 2026-11-14 | Scope and variants (RAW and Compact RAW, .CR3 extension, HEIF with HDR PQ) |
| S11 | Canon, EOS R7 Product Manual, Dual Pixel RAW | https://cam.start.canon/en/C005/manual/html/UG-04_Shooting-1_0040.html | vendor | 2026-11-14 | Scope and variants (Dual Pixel RAW) |
| S12 | D. Sakamoto (Canon Inc.), Image encoding apparatus, control method thereof, and storage medium, US Patent 10,097,840 B2, 2018 | https://patents.google.com/patent/US10097840B2/en | patent | 2026-11-14 | Compression (Canon RAW coding described in general terms only) |
| S13 | Measurement: EOS M50 samples Canon_EOS_M50_RAW_ISO_100_nocrop_nodual and Canon_EOS_M50_CRAW_ISO_100_nocrop_nodual, public domain (CC0) files | https://raw.pixls.us/data/Canon/EOS%20M50/ | measurement | 2026-11-14 | all sections, records M1 to M13 |
| S14 | Measurement: EOS R6 samples Canon_EOS_R6_RAW_ISO_100_crop_nodual and Canon_EOS_R6_CRAW_ISO_100_crop_nodual (CC0) | https://raw.pixls.us/data/Canon/EOS%20R6/ | measurement | 2026-11-14 | all sections, test vectors TV-1 to TV-13 |
| S15 | Measurement: EOS R5 sample Canon_EOS_R5_CRAW_ISO_100_crop_dual (CC0) | https://raw.pixls.us/data/Canon/EOS%20R5/Canon_EOS_R5_CRAW_ISO_100_crop_dual.CR3 | measurement | 2026-11-14 | Scope and variants (X3), records M3, M14 |
| S16 | Measurement: EOS R6 Mark II sample canon-r6m2-img_0349-raw-dual-crop (CC0) | https://raw.pixls.us/data/Canon/EOS%20R6%20Mark%20II/canon-r6m2-img_0349-raw-dual-crop.cr3 | measurement | 2026-11-14 | Scope and variants (X3), Levels and white balance, records M3, M11, M14 |
| S17 | Measurement: EOS R6 Mark III samples IMG_4243 (RAW) and IMG_4244 (C-RAW) (CC0) | https://raw.pixls.us/data/Canon/Canon%20EOS%20R6%20Mark%20III/ | measurement | 2026-11-14 | Levels and white balance, records M7, M10, M11 |
| S18 | Measurement: EOS-1D X Mark III sample Canon_EOS-1D_X_Mark_III_CRAW_ISO_100_nocrop_nodual (CC0) | https://raw.pixls.us/data/Canon/EOS-1D%20X%20Mark%20III/Canon_EOS-1D_X_Mark_III_CRAW_ISO_100_nocrop_nodual.CR3 | measurement | 2026-11-14 | Levels and white balance, records M7, M11 |
| S19 | Measurement: PowerShot V1 sample IMG_1225 (CC0) | https://raw.pixls.us/data/Canon/Canon%20PowerShot%20V1/IMG_1225.CR3 | measurement | 2026-11-14 | Scope and variants, Levels and white balance, records M2, M7, M11 |
| S20 | Measurement: EOS R5 Mark II sample CRAW.CR3, a file with HEVC previews (CC0) | https://raw.pixls.us/data/Canon/Canon%20EOS%20R5m2/CRAW.CR3 | measurement | 2026-11-14 | Scope and variants (X5), record M15 |
| S21 | Measurement: container survey of all 110 CR3 files of the archive (first 196608 bytes, first 64 bytes of each image sample, and the whole CTMD sample of each file, read with HTTP range requests) (CC0 archive) | https://raw.pixls.us/data/Canon/ | measurement | 2026-11-14 | Scope and variants, Container layout, Metadata tags, Levels and white balance, records M1 to M18 |

Sample files are from the public raw sample archive raw.pixls.us. The archive marks every file used here as public domain (CC0). The author downloaded the files and inspected them with measurement tools that the author wrote from S1, S2, S4, S5 and the S7 tag tables for this purpose. One comparison decoded the embedded JPEG previews with the Pillow imaging library, a general image library that contains no raw decoder. The tools are not part of the deliverable.

### Measurement record

Each record states what was measured, on which files, and the result. Requirements cite the record number.

- M1 (top level, S13 to S21): all 110 files start with the 24-byte box 00 00 00 18 66 74 79 70 63 72 78 20 00 00 00 01 63 72 78 20 69 73 6F 6D: type "ftyp", major brand "crx ", minor version 1, compatible brands "crx " and "isom". The top-level boxes are, in this order: "ftyp", "moov", a uuid box with the extended type BE 7A CF CB 97 A9 42 E8 9C 71 99 94 91 E3 AF AC, the preview holder box, in some files one "free" box, and "mdat". The payload of the first uuid box starts with the text "<?xpacket begin=" (an XMP packet). The "mdat" box has the size field 1 and a 64-bit size, so its header is 16 bytes, and it ends at the end of the file. Box sizes, read as in S1, tile every container exactly.
- M2 (Canon metadata box, S13 to S21): the first child of "moov" is the Canon metadata box. Its payload after the 16-byte extended type holds, in this order, the boxes "CNCV", "CCTP", "CTBO", "free", "CMT1", "CMT2", "CMT3", "CMT4" and "THMB". "CNCV" holds a 30-byte ASCII text, for example "CanonCR3_001/00.10.00/00.00.00". Values seen: "CanonCR3_001/00.09.00/00.00.00" (43 files), "CanonCR3_001/00.10.00/00.00.00" (21), "CanonCR3_001/00.11.00/00.00.00" (21), "CanonCR3_001/01.09.00/00.00.00" (18), "CanonCR3_001/01.11.00/00.00.00" (1), "CanonCR3_003/00.10.00/00.00.00" (2) and "CanonCR3_003/00.11.00/00.00.00" (4). The text starts with "CanonCR3_003" exactly in the 6 files with HEVC previews (M15).
- M3 (tracks, S13 to S21): after "mvhd", "moov" holds 4 "trak" boxes (94 files), or 5 in Dual Pixel RAW files (16 files). Track 1 (track_id 1 in "tkhd") is a JPEG track, or an HEVC track in the 6 files of M15. Tracks 2 and 3 are CRX tracks. Track 4 is the metadata track. Track 5, when present, is a CRX track. Image tracks have the handler type "vide". The metadata track has the handler type "meta". Every "stsd" has 1 entry. Every track has 1 sample, 1 chunk, the "stsc" table (1, 1, 1), and a "co64" box. No file has an "stco" box.
- M4 (CRAW sample entry, S13 to S21): the entry has the 8-byte header, the 78 bytes of the video sample description of S1 (width at entry payload offset 24, height at 26, depth 24, color table ID -1), then 4 bytes that S1 does not define (00 03 00 00 in JPEG tracks, 00 01 00 01 in CRX tracks, 00 04 00 00 in HEVC tracks), then child boxes up to the end of the entry: "JPEG" (12 bytes, payload 00 00 00 00) and "free" in JPEG tracks, "CMP1", "CDI1" and "free" in CRX tracks, "HEVC" and "free" in HEVC tracks. The width and height of the track-1 entry equal the size of the track-1 image. The width and height of the track-3 entry equal the CMP1 size in the 63 files with CNCV "CanonCR3_001/00.09.00" or "CanonCR3_001/01.09.00", and the track-1 image size in the 47 other files. They are therefore not the size of the CRX image.
- M5 (THMB, S13 to S21): in 104 files the THMB payload is: 4 zero bytes, width 160 (u16), height 120 (u16), JPEG length L (u32), 2 bytes with the value 1 or 2, 2 zero bytes, then L bytes of JPEG. After the JPEG, 0 to 6 bytes fill the box. These bytes are not always zero. In the 10 files S13 to S19 the thumbnail shows the image in rows 6 to 113, and rows 0 to 5 and 114 to 119 are black (mean below 2 on the 0 to 255 scale). In the 6 files of M15 the payload starts with 01 00 00 00 and holds HEVC data.
- M6 (PRVW, S13 to S21): the payload of the preview holder box is: the 16-byte extended type, 8 bytes 00 00 00 00 00 00 00 01, and the box "PRVW". In 104 files the PRVW payload is: 4 zero bytes, the u16 value 1, the width (u16), the height (u16), the u16 value 1, JPEG length L (u32), then L bytes of JPEG and 0 to 6 bytes of fill. The size is 1620 x 1080, or 1440 x 1080 in the 8 PowerShot SX70 HS files (a 4:3 sensor). The PRVW box ends exactly at the end of the preview holder box. In the 6 files of M15 the PRVW payload starts with 01 00 00 00, the u16 values at payload offsets 4 and 10 are 2 and 0xFFFF, and the data is HEVC.
- M7 (JPEG streams, S13 to S19): every JPEG of the JPEG tracks, PRVW boxes and THMB boxes of the 10 files S13 to S19 starts with FF D8 FF DB, has no APP segment, and has one SOF0 frame (baseline DCT, S5) with P = 8, Nf = 3, component 1 with H = 2 and V = 1 and quantization table 0, components 2 and 3 with H = 1, V = 1 and table 1. Every stream ends with FF D9 exactly at its stated length. The Pillow library decodes all 30 streams. The track-1 JPEG scaled to 1620 x 1080 differs from the PRVW JPEG by a mean absolute difference of 1.06 to 2.67 on the 0 to 255 scale, so both show the same rendering of the scene. Rows 6 to 112 of the THMB JPEG differ from the PRVW JPEG scaled to 160 x 107 by 4.1 to 10.6.
- M8 (largest preview, S13 to S21): in all 104 files with a JPEG track, the first 1024 bytes of the track-1 sample start with FF D8 FF DB and hold an SOF0 segment with 3 components and the sampling factors of M7. The JPEG of track 1 is the largest JPEG of the file. Its size equals CMT1 ImageWidth and ImageLength, the sample entry width and height of track 1, and the crop rectangle of SensorInfo (M12). Examples: EOS M50 6000 x 4000, EOS R6 in 1.6x crop mode 3408 x 2272, EOS R5 8192 x 5464, PowerShot V1 5760 x 3840, PowerShot SX70 HS 5184 x 3888. Files recorded with the aspect ratio settings 1:1, 4:3 and 16:9 still have a track-1 JPEG of the full sensor area in the native aspect ratio (EOS 250D, EOS M6 Mark II, PowerShot G5 X Mark II and SX70 HS sets).
- M9 (TIFF blocks, S13 to S21): the payload of CMT1, CMT2, CMT3 and CMT4 is a TIFF structure with the header 49 49 2A 00 08 00 00 00 (little-endian, IFD0 at 8). Offsets in the IFD entries count from the first byte of the payload. CMT1 holds the tags 0x0100, 0x0101, 0x0102, 0x0103, 0x010F, 0x0110, 0x0112, 0x011A, 0x011B, 0x0128, 0x0132, 0x013B and 0x8298, and its next-IFD offset is 0. CMT2 holds Exif tags, among them 0xA001 ColorSpace with the value 1 in all 10 local files. In no file does CMT3 hold the tag 0x4001.
- M10 (CTMD, S13 to S21): the CTMD sample entry and the sample of the metadata track use these structures. The sample is a sequence of records that fills the sample exactly. Each record starts with its size (u32 LE, header included) and its type (u16 LE), followed by 6 bytes. Record types seen: 1, 3, 4, 5, 7, 8, 9, and in newer files 14. The payload of records 7, 8 and 9 after the 12-byte record header is a sequence of blocks that fills the record exactly. Each block starts with its size (u32 LE, 8-byte header included) and a tag (u32 LE): 0x8769 (a 22-byte TIFF with one entry, without the 4-byte next-IFD offset) or 0x927C (a Canon maker note as a TIFF with the header 49 49 2A 00 08 00 00 00). The next-IFD offset of the maker-note IFDs holds values such as 64225313 that are not valid offsets.
- M11 (ColorData, S13 to S21): in all 110 files the maker note of the record of type 8 holds tag 0x4001 with type SHORT (3). Its value 0 (the ColorDataVersion of S7) and its count are: 16 and 1820 (EOS M50, Kiss M), 17 and 1824 (EOS R), 18 and 1816 (EOS RP, 250D, Rebel SL3, 200D Mark II, PowerShot SX70 HS), 19 and 1816 (EOS 90D, 850D, M6 Mark II, M200, M50 Mark II, R100, PowerShot G5 X Mark II, G7 X Mark III), 32 and 2024 (EOS-1D X Mark III), 33 and 3656 (EOS R5, R6), 34 and 3973 (EOS R3), 48 and 3778 (EOS R6 Mark II, R7, R8, R10, R50), 64 and 4528 (EOS R5 Mark II), 65 and 3778 (EOS R50 V, PowerShot V1), 66 and 3778 (EOS R6 Mark III). Read at the S7 indices of that version, the second and third white balance levels are 1024 in every file, the black levels are 509 to 513 or 2042 to 2049, and NormalWhiteLevel and SpecularWhiteLevel are between 10336 and 14888.
- M12 (SensorInfo, S13 to S21): CMT3 tag 0x00E0 (SensorInfo, S7) holds 17 SHORT values. Values 1 and 2 (SensorWidth, SensorHeight) equal the CMP1 width and height of track 3. Values 5 to 8 (left, top, right and bottom border) describe a rectangle of (right - left + 1) x (bottom - top + 1) pixels, which equals the track-1 sample entry size in all 110 files and the track-1 JPEG size in all 104 files with a JPEG track. The same rectangle appears as the first rectangle of the IAD1 box of track 3 in all 110 files.
- M13 (CRX tracks, S13 to S21): CMP1 payload bytes 8 to 11 and 12 to 15 hold the width and height (u32) of the CRX image. The first two payload bytes are FF 00 (88 files) or FF 10 (22 files). FF 10 occurs exactly in the C-RAW files of the EOS-1D X Mark III, R5, R5 Mark II, R6, R6 Mark II, R6 Mark III, R7, R8, R50 and PowerShot V1. The C-RAW files of the bodies with CNCV "CanonCR3_001/00.09.00" and "CanonCR3_001/01.09.00" start with FF 00, like all RAW files. Payload byte 26 is 0x00 in all 49 files that the archive labels RAW (by file name or description) and 0x03 in all 47 files labelled C-RAW or CRAW. The other 14 files carry no label. The CRX samples of tracks 2 and 3 start with the bytes FF 01 00 08 when CMP1 starts with FF 00, and with FF 11 00 10 when CMP1 starts with FF 10.
- M14 (Dual Pixel RAW, S15, S16, S21): the 16 files with a track 5 are exactly the files that the archive labels "dual" or "Dual Pixel" (EOS R, R5, R6 Mark II, R7, R8). In all 16, track 5 is a CRX track with the same CMP1 width and height as track 3. Track 3 has the same structure in files with and without track 5.
- M15 (HEVC preview files, S20, S21): 6 files: EOS R5 Mark II RAW.CR3 and CRAW.CR3 (CNCV "CanonCR3_003/00.10.00/00.00.00") and the 4 EOS R8 files (CNCV "CanonCR3_003/00.11.00/00.00.00"). Track 1 is an HEVC track: its CRAW entry holds the child boxes "HEVC" and "free", and "HEVC" holds boxes named "GRID", "CISZ" and "hvcC". The THMB and PRVW payloads start with 01 00 00 00 and the u16 value 2. THMB has the u16 values 320 and 214 at payload offsets 6 and 8, PRVW the values 1620 and 1080. Where version-0 boxes have a JPEG stream, both hold boxes named "CISZ" and "hvcC". The whole 27527840-byte file S20 contains no byte sequence FF D8 FF.
- M16 (orientation, S21): CMT1 Orientation is 1 in 109 files and 8 in one file, EOS R10 IMG_4361.CR3 (https://raw.pixls.us/data/Canon/EOS%20R10/IMG_4361.CR3). In that file the track-1 JPEG is 6000 x 4000 and the PRVW JPEG 1620 x 1080, both wider than tall. The PRVW JPEG shows a standing person lying on the side. After the mapping of the value 8 (R-CR3-39) the head of the person is at the top. The camera therefore stores the previews unrotated, and the decoder applies the orientation.
- M17 (survey models, S21): the 110 files cover these models (CMT1 Model without "Canon "): EOS M50, EOS KISS M, EOS M50m2, EOS M6 Mark II, EOS M200, EOS 250D, EOS Rebel SL3, EOS 200D II, EOS 850D, EOS 90D, EOS R, EOS RP, EOS R3, EOS R5, EOS R5m2, EOS R6, EOS R6m2, EOS R6 Mark III, EOS R7, EOS R8, EOS R10, EOS R50, EOS R50 V, EOS R100, EOS-1D X Mark III, PowerShot G5 X Mark II, PowerShot G7 X Mark III, PowerShot SX70 HS and PowerShot V1.
- M18 (CTBO, S13 to S21): CTBO holds a u32 count N and N entries of 20 bytes: index (u32), offset (u64), size (u64). Entry 1 gives the offset and size of the XMP uuid box, entry 2 those of the preview holder box, entry 3 those of "mdat". Entries 4 and 5 are 0.
- M19 (fixtures): the author built the fixtures F1, F2 and F3 of section Test vectors with the measurement tools, parsed them with the same tools as the camera files, and decoded their JPEG streams with Pillow. The results are the expected values of section Test vectors.

### Sources encountered and not used

The readme of https://github.com/lclevy/canon_cr3 was opened as a format description. The author read its first part (title, contributor list, introduction and box overview). The introduction cites the LibRaw decoder source file crx.cpp as the basis of one statement, and the contributor list names the author of an open-source CRX decoder as the source of its CMP1 description. The author stopped reading at that point, deleted the local copy, and used nothing from it. Every container fact in this document comes from S1 to S9 and from the measurements M1 to M18. Search results that pointed to decoder documentation inside source repositories or source browsers (libopenraw doc/cr3.md on a crate source browser, a fork of the same readme) and to a book with metadata-library code listings were seen only as titles and were not opened.

## Scope and variants

### File generations

The CNCV text (M2) names a container generation. The decoder does not need it: R-CR3-2 classifies a file by its structure.

| CNCV | Measured models (M2, M17) | Track 1 |
| --- | --- | --- |
| CanonCR3_001/00.09.00/00.00.00 | EOS M50, Kiss M, M50 Mark II, M6 Mark II, M200, 250D, Rebel SL3, 200D Mark II, 850D, 90D, R, RP, R100 | JPEG |
| CanonCR3_001/01.09.00/00.00.00 | PowerShot G5 X Mark II, G7 X Mark III, SX70 HS | JPEG |
| CanonCR3_001/00.10.00/00.00.00 | EOS-1D X Mark III, R3, R5, R5 Mark II (files with JPEG previews), R6, R6 Mark III | JPEG |
| CanonCR3_001/00.11.00/00.00.00 | EOS R6 Mark II, R7, R10, R50, R50 V | JPEG |
| CanonCR3_001/01.11.00/00.00.00 | PowerShot V1 | JPEG |
| CanonCR3_003/00.10.00/00.00.00 | EOS R5 Mark II (files with HEVC previews) | HEVC |
| CanonCR3_003/00.11.00/00.00.00 | EOS R8 | HEVC |

### Content of a CR3 file

S10 names two user settings for raw images, RAW and Compact RAW (C-RAW), and states that HEIF output needs the HDR PQ setting. S11 describes Dual Pixel RAW. The measured files map these settings to the content below.

| Id | Content | Where (M3) | Status |
| --- | --- | --- | --- |
| P1 | full-size JPEG preview | first sample of the JPEG track (track 1) | in scope (preview path) |
| P2 | 1620 x 1080 or 1440 x 1080 JPEG preview | PRVW box in the preview holder box | in scope (preview path) |
| P3 | 160 x 120 JPEG thumbnail | THMB box in the Canon metadata box | in scope (preview path) |
| X1 | sensor data, CRX lossless (camera setting RAW) | first sample of the main CRX track (track 3), CMP1 byte 26 = 0 | out of scope |
| X2 | sensor data, CRX lossy (camera setting C-RAW) | first sample of the main CRX track, CMP1 byte 26 not 0 | out of scope |
| X3 | Dual Pixel RAW data | an additional CRX track with the size of the main CRX track (track 5) | out of scope |
| X4 | several images per file (S7 lists the maker-note tags RawBurstImageNum and RawBurstImageCount) | image tracks with more than one sample | not measured, out of scope |
| X5 | files with HEVC previews | HEVC track, THMB and PRVW with version 1 | out of scope |
| X6 | reduced-size CRX image (1444 to 2304 pixels wide) | track 2 | out of scope |

Out-of-scope content has a measured detection rule (M13 to M15) and the fallback "show the embedded JPEG preview". No CR3 file holds sensor data in any other form (M3: 110 of 110 files).

### Requirements

- R-CR3-1: The decoder must not decode or interpret the samples of CRX tracks and HEVC tracks, and must deliver an embedded JPEG preview (section Embedded preview) for every file that passes R-CR3-4.
- R-CR3-2: The decoder must report the variant of the file by these rules (for information only, the preview path is the same for all):
  1. X5 when the file has an HEVC track (R-CR3-15) or a THMB or PRVW box with version 1 (R-CR3-20).
  2. X4 when the "stsz" number of entries of an image track is greater than 1.
  3. X1 when the main CRX track (R-CR3-16) exists and byte 26 of its CMP1 payload is 0.
  4. X2 when the main CRX track exists and byte 26 of its CMP1 payload is not 0.
  5. Unknown in every other case.
  The decoder must also report "Dual Pixel RAW" when a second CRX track has the same CMP1 width and height as the main CRX track (X3).

## Container layout

### Boxes

A CR3 file is a sequence of boxes (S3: "All data is encapsulated in boxes"). S1 defines the header of a box (S1 calls it an atom):

| Offset in box | Size | Field | Meaning (S1) |
| --- | --- | --- | --- |
| 0 | 4 | size | u32, size of the box in bytes, header and payload included |
| 4 | 4 | type | four-character code |
| 8 | 8 | extended size | u64, present only when size = 1: the size of the box |

Header size H is 8, or 16 when the size field is 1. S1: the size value 0 "is allowed only for a top-level atom" and means that the box extends to the end of the file. "The actual size of an atom cannot be less than 8 bytes" (S2). A uuid box has 16 more bytes after the header: the extended type (S3 and M1, M2, M6). Its own data follow these 16 bytes.

S1 and S2 state that a reader skips a box of an unknown type with its size. S3: "Extensibility is supported by allowing unrecognized box types to be skipped".

- R-CR3-3: The decoder must read every multi-byte field of a box, of the sample tables and of the Canon boxes CMP1, THMB, PRVW and CTBO as big-endian.
- R-CR3-4: The decoder must accept a file only if bytes 4 to 7 are "ftyp" (66 74 79 70), the size of that first box is at least 16, and the major brand at bytes 8 to 11 is "crx " (63 72 78 20). For any other file it must report "not a CR3 file".
- R-CR3-5: The decoder must compute the size of a box from its header: the u32 size; the u64 extended size when the u32 size is 1; and, when the u32 size is 0 and the box is a top-level box, the distance from the box start to the end of the file.
- R-CR3-6: The decoder must treat a box as invalid when its size is smaller than its header size (8, 16 for an extended size, plus 16 for a uuid box), when it extends past the end of its parent box (past the end of the file for a top-level box), or when its u32 size is 0 and it is not a top-level box. It must stop reading the container at the first invalid box and keep the boxes before it.
- R-CR3-7: The decoder must skip a box of a type that this document does not name, by its size, without reading its payload.
- R-CR3-8: The decoder must identify a uuid box by the 16 bytes that follow its header, and must compare all 16 bytes.

### Top-level layout

Measured layout (M1), values of S14 EOS R6 RAW crop:

| Box | Offset | Size | Content |
| --- | --- | --- | --- |
| "ftyp" | 0 | 24 | brand "crx ", minor version 1, compatible brands "crx " and "isom" |
| "moov" | 24 | 36368 | Canon metadata box and tracks |
| uuid BE 7A CF CB ... | 36392 | 65560 | XMP packet (not used) |
| uuid EA F4 2B 5E ... | 101952 | 320856 | preview holder box with PRVW |
| "free" | 422808 | 88 | fill (not used) |
| "mdat" | 422896 | 9392742 (16-byte header) | samples of all tracks |

### Box tree

The table lists every box that this document uses, in measured order (M1 to M4). The decoder finds boxes by type and does not depend on their order (R-CR3-9).

| Path | Box | Content |
| --- | --- | --- |
| top | "ftyp" | file type |
| top | "moov" | container |
| moov | uuid 85 C0 B6 87 82 0F 11 E0 81 11 F4 CE 46 2B 6A 48 | Canon metadata box: after the 16-byte extended type, a sequence of boxes |
| moov / Canon metadata box | "CNCV" | ASCII text, the container generation |
| moov / Canon metadata box | "CCTP", "CTBO", "free" | not used (M18 describes CTBO) |
| moov / Canon metadata box | "CMT1", "CMT2", "CMT3", "CMT4" | TIFF structures (section Metadata tags) |
| moov / Canon metadata box | "THMB" | thumbnail (P3) |
| moov | "mvhd" | movie header, not used |
| moov | "trak" | one per track, container |
| moov / trak | "tkhd" | track header, not used |
| moov / trak | "mdia" | container |
| moov / trak / mdia | "mdhd" | not used |
| moov / trak / mdia | "hdlr" | handler type |
| moov / trak / mdia | "minf" | container |
| moov / trak / mdia / minf | "vmhd", "nmhd", "dinf" | not used |
| moov / trak / mdia / minf | "stbl" | container |
| moov / trak / mdia / minf / stbl | "stsd" | sample entry |
| moov / trak / mdia / minf / stbl | "stts", "stsc", "free" | not used |
| moov / trak / mdia / minf / stbl | "stsz" | sample sizes |
| moov / trak / mdia / minf / stbl | "co64" or "stco" | chunk offsets |
| top | uuid BE 7A CF CB 97 A9 42 E8 9C 71 99 94 91 E3 AF AC | XMP packet, not used |
| top | uuid EA F4 2B 5E 1C 98 4B 88 B9 FB B7 DC 40 6E 4D 16 | preview holder box: 16-byte extended type, 8 bytes, then a sequence of boxes |
| top / preview holder box | "PRVW" | preview (P2) |
| top | "mdat" | samples |

- R-CR3-9: The decoder must find "moov" among the top-level boxes, the Canon metadata box among the children of "moov", and the preview holder box among the top-level boxes, by type and extended type, in any order.
- R-CR3-10: The decoder must read the payload of the Canon metadata box, after its 16-byte extended type, as a sequence of boxes.
- R-CR3-11: The decoder must read the payload of the preview holder box from byte 24 of the payload (after the 16-byte extended type and 8 more bytes) as a sequence of boxes.

### Tracks

Each "trak" box holds "mdia", which holds "hdlr" and "minf". "minf" holds "stbl", which holds "stsd", "stsz" and "co64" or "stco". All four are full boxes.

"hdlr" (S1, handler reference atom): payload offset 0 version and flags (4 bytes), offset 4 component type (4 bytes, 0 in CR3), offset 8 component subtype (4 bytes). The component subtype is the handler type: "vide" for image tracks, "meta" for the metadata track (M3).

"stsd" (S1, sample description atom): payload offset 0 version and flags, offset 4 number of entries (u32), offset 8 the first entry. S2: the first four fields of every sample description are its size (u32), its data format (four-character code), 6 reserved bytes and a data reference index (u16). The CRAW entry (M4) continues with the video sample description fields of S1:

| Offset in entry | Size | Field | CR3 value (M4) |
| --- | --- | --- | --- |
| 0 | 4 | entry size | for example 112 (JPEG track), 212 and 228 (CRX tracks) in S14 |
| 4 | 4 | data format | "CRAW" |
| 8 | 6 | reserved | 0 |
| 14 | 2 | data reference index | 1 |
| 16 | 16 | version, revision, vendor, temporal and spatial quality | 0 |
| 32 | 2 | width | track 1: width of its image. CRX tracks: not reliable (M4) |
| 34 | 2 | height | track 1: height of its image. CRX tracks: not reliable (M4) |
| 36 | 4 | horizontal resolution (16.16 fixed point) | 0x00480000 (72.0) |
| 40 | 4 | vertical resolution | 0x00480000 |
| 44 | 4 | data size | 0 |
| 48 | 2 | frame count | 1 |
| 50 | 32 | compressor name | 0 |
| 82 | 2 | depth | 24 |
| 84 | 2 | color table ID | 0xFFFF (-1) |
| 86 | 4 | Canon field, not in S1 | 00 03 00 00 (JPEG), 00 01 00 01 (CRX), 00 04 00 00 (HEVC) |
| 90 | to entry end | child boxes | "JPEG" or "CMP1" and "CDI1" or "HEVC", then "free" |

S1 states that video sample descriptions "can be extended by appending other atoms". S8 lists "CMP1", "CDI1" and "JPEG" as children of the visual sample description.

- R-CR3-12: The decoder must read the handler type of a track from bytes 8 to 11 of the "hdlr" payload.
- R-CR3-13: The decoder must read the first sample entry of "stsd" at payload offset 8, must require that its size is at least 8 and lies inside the "stsd" payload, and must read its data format at entry offset 4.
- R-CR3-14: For a sample entry with the data format "CRAW", the decoder must read the child boxes from entry offset 90 to the end of the entry, and must treat the entry as having no children when the entry is shorter than 90 bytes.
- R-CR3-15: The decoder must classify each track by its content and not by its track number or position:
  1. JPEG track: handler type "vide", data format "CRAW", a child box "JPEG".
  2. HEVC track: handler type "vide", data format "CRAW", a child box "HEVC".
  3. CRX track: handler type "vide", data format "CRAW", a child box "CMP1".
  4. Metadata track: handler type "meta", data format "CTMD".
  5. Any other track is ignored.
- R-CR3-16: The decoder must take as the main CRX track the CRX track with the largest product of the CMP1 width and height (CMP1 payload bytes 8 to 11 and 12 to 15, u32), and the first such track in "moov" order when two tracks have the same product.

### Sample location

"stsz" (S1, sample size atom): payload offset 0 version and flags, offset 4 sample size (u32), offset 8 number of entries (u32), offset 12 a table of u32 sizes. When the sample size is not 0, all samples have that size and no table follows. "stco" (S1, chunk offset atom): payload offset 0 version and flags, offset 4 number of entries (u32), offset 8 a table of u32 offsets. S2: the 64-bit variant "co64" is "used in place of" "stco" and holds 64-bit offsets. M3 confirms 8-byte entries. S2: the "offsets are file offsets, not the offset into any atom within the file". The first sample of a track starts at the first chunk offset (S1: a chunk holds one or more samples; M3: one sample per chunk).

- R-CR3-17: The decoder must locate the first sample of a track at the first entry of "co64" (u64) or, when the track has no "co64", of "stco" (u32), and must take its size from the "stsz" sample size when that is not 0 and from the first table entry otherwise.
- R-CR3-18: The decoder must treat a track as having no usable sample when "stsz" or the chunk offset box is missing, when a number of entries is 0, or when the sample does not lie completely inside the file.

### THMB box (P3)

| Offset in THMB payload | Size | Field (M5) |
| --- | --- | --- |
| 0 | 1 | version: 0 (JPEG), 1 (HEVC, M15) |
| 1 | 3 | flags, 0 |
| 4 | 2 | width (u16), 160 |
| 6 | 2 | height (u16), 120 |
| 8 | 4 | JPEG length L (u32) |
| 12 | 2 | 1 or 2, not used |
| 14 | 2 | 0, not used |
| 16 | L | JPEG stream |
| 16 + L | to box end | fill, not used |

### PRVW box (P2)

| Offset in PRVW payload | Size | Field (M6) |
| --- | --- | --- |
| 0 | 1 | version: 0 (JPEG), 1 (HEVC, M15) |
| 1 | 3 | flags, 0 |
| 4 | 2 | 1, not used |
| 6 | 2 | width (u16), 1620 or 1440 |
| 8 | 2 | height (u16), 1080 |
| 10 | 2 | 1, not used |
| 12 | 4 | JPEG length L (u32) |
| 16 | L | JPEG stream |
| 16 + L | to box end | fill, not used |

- R-CR3-19: The decoder must read the JPEG of a THMB box from payload offset 16 with the length L at payload offset 8, and the JPEG of a PRVW box from payload offset 16 with the length L at payload offset 12, and must require 16 + L to be at most the payload size.
- R-CR3-20: The decoder must use a THMB or PRVW box as a JPEG source only when its version byte (payload offset 0) is 0.

### Other Canon boxes

CTBO (M18) lists the offsets and sizes of the XMP box, the preview holder box and "mdat". It duplicates what the box walk finds.

- R-CR3-21: The decoder must not use CTBO, CCTP, CNCV or the XMP box to locate any data.

## Metadata tags

### TIFF blocks CMT1 to CMT4

S7 names the four boxes: CMT1 is IFD0 (EXIF tags), CMT2 the Exif IFD, CMT3 the Canon maker note (Canon tags) and CMT4 the GPS IFD. Each payload is a complete TIFF structure (M9): a TIFF header (S4 section 2) and IFD0. Offsets in its entries count from the first byte of the payload, which is the first byte of the TIFF header.

The rules for IFD entries are those of S4 section 2: an entry has a tag (2 bytes), a field type (2 bytes), a count (4 bytes) and a value or an offset (4 bytes). The value bytes occupy count times the size of the field type (BYTE, ASCII, SBYTE and UNDEFINED 1 byte; SHORT and SSHORT 2; LONG, SLONG and FLOAT 4; RATIONAL, SRATIONAL and DOUBLE 8). When that total is 4 or less, the value is in the value field. Otherwise the value field holds the offset of the value bytes.

- R-CR3-22: The decoder must read each TIFF block (CMT1 to CMT4 and the CTMD blocks of R-CR3-26) with the byte order of its header (49 49 little-endian, 4D 4D big-endian), must require the value 42 at header bytes 2 and 3, and must take all offsets inside the block relative to the first byte of the block.
- R-CR3-23: The decoder must read a value from the value field when count times type size is 4 or less, and from the offset in the value field otherwise, and must ignore an entry whose value bytes do not lie completely inside the block or whose field type is outside 1 to 12.
- R-CR3-24: The decoder must find tags by tag number in any entry order, must compare ASCII values after it removes trailing zero bytes and spaces, and must read only IFD0 of each TIFF block (it must ignore every next-IFD offset, M10).

### Tags used

| Block | Tag | Name (S7, S9) | Type | Count | Use in this document | Default |
| --- | --- | --- | --- | --- | --- | --- |
| CMT1 | 0x010F | Make | ASCII | any | report, "Canon" in all files | empty |
| CMT1 | 0x0110 | Model | ASCII | any | report, for example "Canon EOS R6" | empty |
| CMT1 | 0x0112 | Orientation | SHORT | 1 | orientation of the preview (R-CR3-38) | 1 |
| CMT1 | 0x0100 | ImageWidth | SHORT or LONG | 1 | equals the track-1 JPEG width (M8), not used | none |
| CMT1 | 0x0101 | ImageHeight (S4: ImageLength) | SHORT or LONG | 1 | equals the track-1 JPEG height (M8), not used | none |
| CMT2 | 0xA001 | ColorSpace | SHORT | 1 | color space of the previews (R-CR3-41) | 1 |
| CMT3 | 0x00E0 | SensorInfo | SHORT | 17 | report: sensor size and camera crop (R-CR3-34) | none |
| CTMD record 8 maker note | 0x4001 | ColorData | SHORT | 1816 to 4528 | levels and white balance (section Levels and white balance) | none |

S9 lists the ColorSpace values 1 (sRGB) and 0xFFFF (Uncalibrated), and states that an Adobe RGB image is indicated by "Uncalibrated" with an InteropIndex of "R03".

### CTMD: the metadata track

The sample of the metadata track holds Canon timed metadata. S8 lists CTMD among the timed metadata tags. S7 lists the CTMD record types 1 (TimeStamp), 4 (FocalInfo), 5 (ExposureInfo), 7, 8 and 9 (ExifInfo7, ExifInfo8, ExifInfo9) and 14 (LensCorrectionInfo), and states that the ExifInfo records hold the tags 0x8769 (ExifIFD) and 0x927C (MakerNoteCanon). M10 gives the byte layout:

| Offset in record | Size | Field (M10) |
| --- | --- | --- |
| 0 | 4 | record size RS (u32 LE), 12-byte header included |
| 4 | 2 | record type (u16 LE) |
| 6 | 6 | not used |
| 12 | RS - 12 | record payload |

The payload of records 7, 8 and 9 is a sequence of blocks:

| Offset in block | Size | Field (M10) |
| --- | --- | --- |
| 0 | 4 | block size BS (u32 LE), 8-byte header included |
| 4 | 4 | block tag (u32 LE): 0x8769 or 0x927C |
| 8 | BS - 8 | TIFF block |

The maker note with ColorData is the block with the tag 0x927C in the record of type 8 (M10, M11). CMT3 does not hold ColorData (M9).

- R-CR3-25: The decoder must read the first sample of the metadata track (R-CR3-17) as a sequence of records, each starting at the end of the previous one, and must stop at the end of the sample or at the first record whose size is less than 12 or extends past the end of the sample.
- R-CR3-26: The decoder must read the payload of a record of type 7, 8 or 9 as a sequence of blocks, each starting at the end of the previous one, and must stop at the end of the record or at the first block whose size is less than 8 or extends past the end of the record.
- R-CR3-27: The decoder must take ColorData from tag 0x4001 of the first block with the tag 0x927C in the first record of type 8, and must accept it only with the field type SHORT (3) or SSHORT (8).

### CMP1, CDI1 and IAD1 (informative)

These boxes describe the CRX image of a CRX track. The decoder uses only the CMP1 width, height and byte 26 (R-CR3-2, R-CR3-16). S7 names CMP1 values ImageWidth and ImageHeight and lists no IAD1 tags.

| Box | Payload bytes | Measured meaning (M12, M13) |
| --- | --- | --- |
| CMP1 | 0 to 1 | FF 00 or FF 10 |
| CMP1 | 8 to 11, 12 to 15 | width and height of the CRX image (u32), equal to SensorInfo values 1 and 2 for track 3 |
| CMP1 | 26 | 0 for RAW, 3 for C-RAW |
| CDI1 | 0 to 3 | 0, then the box "IAD1" |
| IAD1 | 4 to 7 | width and height of the CRX image (u16 each) |
| IAD1 | 16 to 23 (track 3) | left, top, right, bottom of the camera crop (u16 each), equal to SensorInfo values 5 to 8 |

## Sensor data layout

This section is informative. The decoder does not read sensor data (R-CR3-1).

The sensor data of a file is the first sample of the main CRX track (track 3). It is a CRX image of CMP1 width x height pixels (M13), for example 6288 x 4056 for the EOS M50 and 3584 x 2386 for the EOS R6 in 1.6x crop mode. This size equals SensorInfo SensorWidth and SensorHeight (S7, M12). The camera JPEG covers the SensorInfo border rectangle: columns SensorLeftBorder to SensorRightBorder and rows SensorTopBorder to SensorBottomBorder (M12). For the EOS R6 crop sample: columns 156 to 3563 and rows 108 to 2379, which is 3408 x 2272 pixels, the size of its track-1 JPEG.

Track 2 holds a smaller CRX image (1444 to 2304 pixels wide, S21). Track 5 of Dual Pixel RAW files holds a second CRX image of the main size (M14, S11). The color filter arrangement, the bit depth and the sample order of the CRX images are not established (Open questions OQ-1, OQ-9).

- R-CR3-28: The decoder must not read the bytes of any CRX sample or HEVC sample, and must not allocate memory for them.

## Compression

### CRX sensor data (out of scope)

S12, a Canon patent for the encoding of Bayer RAW data, describes in general terms a transformation of the RAW data into four color planes or into a luminance plane and three color difference planes, followed by a wavelet transform, quantization and entropy coding. It does not define a bitstream: no header fields, no code tables and no update rules. S10 states only that Compact RAW files are smaller than RAW files. No other permitted source describes CRX. The measurements establish the detection bytes (M13) but not the coding. Both X1 and X2 are therefore out of scope. The decoder detects them by R-CR3-15 and R-CR3-2 and uses the preview path (OQ-1, OQ-2).

### HEVC previews (out of scope)

In the files of variant X5 the track-1 sample, THMB and PRVW hold HEVC data with Canon boxes "CISZ", "GRID" and "hvcC" (M15). This document does not describe them (OQ-5). These files have no JPEG source, so the decoder reports an error for them (R-CR3-37).

### JPEG previews (in scope)

The previews are JPEG streams by S5. All measured streams (M7) have the marker segments SOI, DQT, SOF0, DHT, SOS in this order: the baseline process, 8-bit samples, three components with the identifiers 1, 2, 3 and the sampling factors 2x1, 1x1, 1x1, Huffman coding, no DRI segment and no restart markers. They have no APP segment, so they carry no orientation and no color space of their own. The three components are Y, Cb and Cr in the convention of S6: the decoded images of all three previews of a file agree (M7). S6 gives the conversion to RGB for 8-bit values:

- R = Y + 1.402 * (Cr - 128)
- G = Y - 0.34414 * (Cb - 128) - 0.71414 * (Cr - 128)
- B = Y + 1.772 * (Cb - 128)

Each result is clipped to 0 to 255. A one-component stream is gray: R = G = B = Y.

- R-CR3-29: The decoder must decode a preview stream by the baseline sequential DCT process of S5 (SOF0) and must also accept SOF1 (extended sequential, Huffman, 8-bit), with any sampling factors from 1 to 4 that S5 allows, with one or three components.
- R-CR3-30: The decoder must convert three-component preview data to RGB with the formulas of S6 above and must treat one-component data as gray.

Worked example (track-1 JPEG of S14, vector TV-9): the SOF0 segment FF C0 00 11 08 08 E0 0D 50 03 01 21 00 02 11 01 03 11 01 gives P = 8, height 0x08E0 = 2272, width 0x0D50 = 3408, Nf = 3, component 1 with H = 2, V = 1 and table 0, components 2 and 3 with H = 1, V = 1 and table 1.

Worked example of the color conversion: Y = 120, Cb = 100, Cr = 160.
- R = 120 + 1.402 * 32 = 164.864.
- G = 120 - 0.34414 * (-28) - 0.71414 * 32 = 120 + 9.63592 - 22.85248 = 106.78344.
- B = 120 + 1.772 * (-28) = 70.384.

## Levels and white balance

No in-scope content uses these values: the previews are rendered by the camera. This section states where a raw decoder finds them, so that a later version can use them, and the decoder reports them as metadata.

### Location

1. Find the metadata track (R-CR3-15) and its first sample (R-CR3-17).
2. Find the first record of type 8 (R-CR3-25), and in it the first block with the tag 0x927C (R-CR3-26).
3. Read IFD0 of the TIFF block and its tag 0x4001 ColorData (R-CR3-27).
4. Read the ColorData values as signed 16-bit numbers c[0], c[1], ... in the byte order of the TIFF block. c[0] is ColorDataVersion (S7).
5. Read the values at the indices of the version in the table below.

S7 gives one table per ColorData layout. Indices count 16-bit values from c[0]. "4 values" means the value at the index and the three that follow.

| ColorDataVersion (c[0]) | S7 table | WB_RGGBLevelsAsShot (4 values) | ColorTempAsShot | PerChannelBlackLevel (4 values) | NormalWhiteLevel | SpecularWhiteLevel | LinearityUpperMargin |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 16, 17, 18, 19 | ColorData9 | 71 | 75 | 329 | 796 | 797 | 798 |
| 32, 33 | ColorData10 | 85 | 89 | 343 | 810 | 811 | 812 |
| 34, 48 | ColorData11 | 105 | 109 | 363 | 640 | 641 | 642 |
| 64, 65, 66, 67 | ColorData12 | 105 | 109 | 383 | 660 | 661 | 662 |

S7 assigns the versions as follows: 16 EOS M50, 17 EOS R, 18 EOS RP and 250D, 19 EOS 90D, 850D, M6 Mark II and M200, 32 EOS-1D X Mark III, 33 EOS R5 and R6, 34 EOS R3, 48 EOS R7, R10, R50 and R6 Mark II, 64 EOS R1 and R5 Mark II, 65 EOS R50 V, 66 EOS R6 Mark III, 67 EOS R8 Mark II. M11 confirms the versions of all measured models that S7 lists, and adds 18 for the PowerShot SX70 HS, 19 for the EOS M50 Mark II, R100, PowerShot G5 X Mark II and G7 X Mark III, 48 for the EOS R8, and 65 for the PowerShot V1. PerChannelBlackLevel, NormalWhiteLevel, SpecularWhiteLevel and LinearityUpperMargin are unsigned 16-bit values (S7: int16u). The white balance levels are signed 16-bit values (S7: int16s).

### Measured values

| Model (CMT1 Model) | Version | WB_RGGBLevelsAsShot | ColorTempAsShot | PerChannelBlackLevel | NormalWhiteLevel | SpecularWhiteLevel |
| --- | --- | --- | --- | --- | --- | --- |
| Canon EOS M50 (S13 RAW) | 16 | 1103 1024 1024 1945 | 3200 | 510 511 511 511 | 11435 | 11892 |
| Canon EOS R6 (S14 RAW) | 33 | 1856 1024 1024 1670 | 4965 | 512 512 512 512 | 12735 | 14008 |
| Canon EOS R5 (S15) | 33 | 1579 1024 1024 2491 | 3737 | 512 512 511 512 | 12735 | 14008 |
| Canon EOS-1D X Mark III (S18) | 32 | 1633 1024 1024 1824 | 4164 | 512 512 512 512 | 12735 | 14008 |
| Canon EOS R6m2 (S16) | 48 | 2144 1024 1024 1362 | 6189 | 512 512 512 512 | 12735 | 14008 |
| Canon EOS R6 Mark III (S17) | 66 | 2320 1024 1024 1454 | 7392 | 512 512 512 512 | 13200 | 13995 |
| Canon PowerShot V1 (S19) | 65 | 1935 1024 1024 1535 | 5600 | 512 512 512 512 | 12535 | 13036 |
| Canon EOS R (S21, Canon_EOS_R_RAW_ISO_100) | 17 | 2001 1024 1024 1582 | 4833 | 511 510 511 510 | 12735 | 14008 |
| Canon EOS RP (S21, RAW nocrop) | 18 | 1789 1024 1024 2097 | 4030 | 513 513 513 513 | 13235 | 14558 |
| Canon PowerShot SX70 HS (S21, IMG_1000) | 18 | 1584 1024 1024 1805 | 5406 | 512 512 512 512 | 12500 | 12500 |
| Canon EOS M6 Mark II (S21, IMG_RAW_3_2) | 19 | 1606 1024 1024 2060 | 3966 | 2048 2048 2048 2048 | 10435 | 10852 |
| Canon EOS 90D (S21, RAW) | 19 | 2097 1024 1024 1376 | 6868 | 512 512 512 512 | 12535 | 13036 |
| Canon PowerShot G5 X Mark II (S21, RAW 3:2) | 19 | 2570 1024 1024 1702 | 6274 | 2048 2048 2048 2048 | 13035 | 14335 |
| Canon EOS R3 (S21, _C3A1200) | 34 | 2072 1024 1024 1742 | 5692 | 2047 2047 2047 2047 | 13685 | 14351 |
| Canon EOS R10 (S21, IMG_4276) | 48 | 1830 1024 1024 1711 | 5475 | 2048 2048 2048 2048 | 13035 | 14338 |
| Canon EOS R5m2 (S21, APS-C_RAW) | 64 | 1780 1024 1024 1971 | 4192 | 512 512 512 512 | 13535 | 14888 |
| Canon EOS R50 V (S21, IMG_0034) | 65 | 1833 1024 1024 1683 | 5572 | 512 512 512 512 | 11535 | 11708 |

Within one model the black levels differ between files: near 512 or near 2048 for the EOS M50, R10 and R50 (M11, S21). The values must therefore come from each file and not from a table per model.

The meaning of the two white levels and the assignment of the four black levels to color filter positions are not established, because no CRX data was decoded (OQ-7, OQ-8).

### Requirements

- R-CR3-31: The decoder must report the as-shot white balance only when ColorDataVersion is in the table above and the 4 WB_RGGBLevelsAsShot values lie inside the ColorData count, and must otherwise report it as unknown.
- R-CR3-32: The decoder must compute the as-shot white balance multipliers from the levels R, G1, G2, B as G = (G1 + G2) / 2, mR = R / G, mG = 1, mB = B / G, and must report them as unknown when R, G1, G2 or B is 0 or negative.
- R-CR3-33: The decoder must report PerChannelBlackLevel (4 values), NormalWhiteLevel and SpecularWhiteLevel as unsigned 16-bit values read at the indices of the table above, each only when its index lies inside the ColorData count, and must not use them to modify any delivered image.

Worked example (S14 EOS R6 RAW, vector TV-11): c[0] = 33, so the ColorData10 indices apply. c[85] to c[88] = 1856, 1024, 1024, 1670. G = 1024. mR = 1856 / 1024 = 1.8125. mB = 1670 / 1024 = 1.630859375. c[89] = 4965 (K). c[343] to c[346] = 512, 512, 512, 512. c[810] = 12735 and c[811] = 14008.

### SensorInfo

- R-CR3-34: The decoder must report the sensor size as SensorInfo values 1 and 2 and the camera crop as the rectangle from (value 5, value 6) to (value 7, value 8) inclusive, only when CMT3 holds tag 0x00E0 with at least 9 values, value 7 >= value 5 and value 8 >= value 6.

## Color conversion

The file carries no color matrix that a permitted source documents. S7 lists ColorCalib tables inside ColorData only with unknown names ("CameraColorCalibration01?" and similar). No in-scope content needs a matrix: the previews are rendered by the camera to RGB. A later version that decodes CRX data needs a camera-to-sRGB matrix per model. The fallback for that case is the method of the Sony specification arw.md (section Color conversion): a per-model D50 matrix from DXOMARK measurements with a generic mean matrix for other models (OQ-10).

The previews are 8-bit RGB values after the conversion of R-CR3-30. CMT2 tag 0xA001 ColorSpace (S9) is 1 (sRGB) in every measured file (M9). The value 0xFFFF (Uncalibrated) was not measured (OQ-11).

- R-CR3-35: The decoder must deliver a decoded preview as 8-bit RGB values in the sRGB encoding (S6 conversion, no linearization, no color matrix), and must report the CMT2 ColorSpace value with it (1 when the tag is absent).

## Decoding procedure

The pipeline from file bytes to image. Each step names the requirements it applies.

1. Check the file size and the file type box (R-CR3-49, R-CR3-4).
2. Walk the top-level boxes. Find "moov", the Canon metadata box and the preview holder box (R-CR3-3, R-CR3-5 to R-CR3-11).
3. Read the children of the Canon metadata box: CMT1 to CMT4 and THMB (R-CR3-10, R-CR3-22 to R-CR3-24).
4. Read every "trak" of "moov": handler type, sample entry, children of the sample entry, first sample location (R-CR3-12 to R-CR3-18). Classify the tracks (R-CR3-15) and choose the main CRX track (R-CR3-16).
5. Report the variant (R-CR3-2). Do not read CRX or HEVC samples (R-CR3-1, R-CR3-28).
6. Optional for display, required for metadata reporting: read the metadata track, ColorData and SensorInfo (R-CR3-25 to R-CR3-27, R-CR3-31 to R-CR3-34).
7. Collect the JPEG candidates and choose the largest (R-CR3-19, R-CR3-20, R-CR3-36, R-CR3-37).
8. Decode the chosen JPEG (R-CR3-29, R-CR3-30). If it fails, try the next candidate (R-CR3-40).
9. Apply the orientation (R-CR3-38, R-CR3-39).
10. Deliver the image with its report (R-CR3-35, R-CR3-41). If no candidate decodes, report an error (R-CR3-40).

## Embedded preview

### Locations

| Id | Location | Size (M5 to M8) | Codec |
| --- | --- | --- | --- |
| P1 | first sample of each JPEG track (track 1) | full size, equal to the camera JPEG size, for example 6000 x 4000 | JPEG (SOF0) |
| P2 | PRVW box in the preview holder box | 1620 x 1080, or 1440 x 1080 for a 4:3 sensor | JPEG (SOF0) |
| P3 | THMB box in the Canon metadata box | 160 x 120, a 3:2 picture with black bands | JPEG (SOF0) |

All three show the same camera rendering of the scene (M7). P1 is the largest in every measured file with JPEG previews (M8). In the files of variant X5 the three sources are HEVC (M15) and there is no JPEG candidate.

### Requirements

- R-CR3-36: The decoder must collect as JPEG candidates, in this order: the first sample of each JPEG track in "moov" order (R-CR3-17), the JPEG of each PRVW box with version 0, and the JPEG of the THMB box with version 0 (R-CR3-19, R-CR3-20). It must keep a candidate only if it lies completely inside the file, has a length of at least 4 bytes, and starts with FF D8.
- R-CR3-37: The decoder must determine the size of each candidate from its first SOF0, SOF1 or SOF2 marker segment (S5, number of lines Y and samples per line X), must choose the candidate with the largest product X * Y, and must choose the earlier candidate of the collection order when two products are equal. When no candidate remains, the decoder must report the error "no supported preview" and must report the variant (X5 for files with HEVC previews).
- R-CR3-38: The decoder must read Orientation from CMT1 tag 0x0112, must use 1 when the tag is absent or outside 1 to 8, and must apply it to the decoded preview with the mapping of the table below, except that it must not apply the values 5 to 8 when the decoded preview is taller than wide.
- R-CR3-39: The decoder must map pixel (x, y) of a W' x H' decoded preview to the output pixel of the table below, with an output of W' x H' pixels for the values 1 to 4 and H' x W' pixels for the values 5 to 8.
- R-CR3-40: The decoder must try the next candidate in the order of decreasing size when a candidate fails to decode, and must report an error only when no candidate decodes.
- R-CR3-41: The decoder must report with the delivered image that it is an embedded preview (sRGB-encoded JPEG values, not decoded sensor data), its size, its source (P1, P2 or P3), and the CMT2 ColorSpace value.
- R-CR3-42: The decoder must accept a preview stream that has bytes after its EOI marker or that ends without an EOI marker after its last scan.

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

These mappings follow the definitions of S4. For example, S4 defines the value 6 as "the 0th row represents the visual right-hand side of the image, and the 0th column represents the visual top". Stored row 0 therefore goes to the rightmost output column, and stored column 0 goes to the top output row. All measured previews are wider than tall (M5 to M8). In the one measured file with Orientation 8, the previews are stored unrotated and the mapping of the value 8 makes the image upright (M16). The exception for previews taller than wide protects against a preview that a camera has already rotated. No such preview was measured (OQ-6).

## Error handling

The decoder reads untrusted files. The rules below prevent out-of-range access, integer overflow, unbounded loops and large allocations. "Fail" for one candidate means: drop that candidate and continue with the next (R-CR3-40). "Fail" for the file means: report an error and deliver no image.

### Limits

- R-CR3-43: The decoder must compute every offset, size and sum of offset and size in at least 64-bit unsigned arithmetic, and must treat a sum that overflows as a value outside the file.
- R-CR3-44: The decoder must read at most 16 levels of nested boxes, at most 4096 boxes in one container, and at most 64 "trak" boxes, and must ignore the boxes beyond these limits.
- R-CR3-45: The decoder must check that a byte range lies completely inside its parent box and inside the file before it reads the range or allocates memory for it.
- R-CR3-46: The decoder must reject a JPEG candidate whose length exceeds 268435456 bytes (256 MiB), or whose SOF gives X or Y equal to 0, X or Y above 65535, or X * Y above 2^28 (268435456), before it allocates the image.
- R-CR3-47: The decoder must ignore a metadata track sample longer than 16777216 bytes (16 MiB), a TIFF block with more than 4096 entries in IFD0, and a ColorData value whose index lies outside the count.
- R-CR3-48: The decoder must stop the record and block walks of R-CR3-25 and R-CR3-26 after 65536 records or blocks.

### Structure checks

- R-CR3-49: The decoder must fail for a file shorter than 24 bytes.
- R-CR3-50: The decoder must fail for the file when "moov" is missing or invalid and the preview holder box holds no valid PRVW candidate.
- R-CR3-51: The decoder must fail for a candidate when its JPEG stream has no SOF0, SOF1 or SOF2 segment before the first SOS, when a marker segment length runs past the end of the candidate, or when the stream decoding of S5 fails (undefined Huffman table, code not in the table, quantization table not defined).
- R-CR3-52: The decoder must treat a ColorData, SensorInfo or Orientation value that fails a check of this document as absent and use the stated default or report it as unknown.

### Behavior on error

- R-CR3-53: The decoder must not terminate the process, must not read or write memory outside its buffers, and must report a decoding failure as a recoverable error value.
- R-CR3-54: The decoder must discard a partly decoded preview on failure and must not deliver it.
- R-CR3-55: The decoder must not produce NaN or infinite values: every division in this document has a divisor checked to be greater than 0, and a failed check makes the quantity unknown.

## Test vectors

All vectors were computed by the author from the definitions in this document and checked with the measurement tools (section Sources). Hex bytes are in file order. Vectors TV-1 to TV-13 are bytes of the camera file S14 EOS R6 RAW crop (Canon_EOS_R6_RAW_ISO_100_crop_nodual.CR3, 9815638 bytes) at the stated offsets.

### TV-1: file type box

Offset 0: 0x00 0x00 0x00 0x18 0x66 0x74 0x79 0x70 0x63 0x72 0x78 0x20 0x00 0x00 0x00 0x01 0x63 0x72 0x78 0x20 0x69 0x73 0x6F 0x6D. Size 24, type "ftyp", major brand "crx ", minor version 1, compatible brands "crx " and "isom". R-CR3-4 accepts the file. The same bytes with major brand "isom" (69 73 6F 6D at bytes 8 to 11) give "not a CR3 file".

### TV-2: box headers

- Offset 24: 00 00 8E 10 6D 6F 6F 76. Size 0x8E10 = 36368, type "moov", header 8, payload from 32 to 36391.
- Offset 422896: 00 00 00 01 6D 64 61 74 00 00 00 00 00 8F 52 66. Size field 1, type "mdat", extended size 0x8F5266 = 9392742, header 16, payload from 422912. The box ends at 422896 + 9392742 = 9815638, the file size.
- Synthetic: 00 00 00 00 6D 64 61 74 as the last top-level box at offset 900 of a 1000-byte file: size 100 (R-CR3-5). The same header inside "moov": invalid (R-CR3-6).
- Synthetic: 00 00 00 04 66 72 65 65: size 4 is less than 8, invalid (R-CR3-6).

### TV-3: Canon metadata box

Offset 32: 00 00 85 30 75 75 69 64 85 C0 B6 87 82 0F 11 E0 81 11 F4 CE 46 2B 6A 48. Size 0x8530 = 34096, type "uuid", extended type of the Canon metadata box. The first child starts at 32 + 8 + 16 = 56: 00 00 00 26 43 4E 43 56 then the 30 bytes "CanonCR3_001/00.10.00/00.00.00" (CNCV, size 38).

### TV-4: handler box of track 1

Offset 34376: 00 00 00 21 68 64 6C 72 00 00 00 00 00 00 00 00 76 69 64 65 followed by 13 bytes 00. Size 33, type "hdlr", version and flags 0, component type 0, handler type "vide" at payload bytes 8 to 11.

### TV-5: sample description of track 1

Offset 34481 (128 bytes):

00 00 00 80 73 74 73 64 00 00 00 00 00 00 00 01
00 00 00 70 43 52 41 57 00 00 00 00 00 00 00 01
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0D 50 08 E0 00 48 00 00 00 48 00 00 00 00 00 00
00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 18 FF FF 00 03 00 00 00 00 00 0C 4A 50
45 47 00 00 00 00 00 00 00 0A 66 72 65 65 00 00

"stsd" of size 128 with 1 entry. The entry at 34497 has size 0x70 = 112, data format "CRAW", data reference index 1, width 0x0D50 = 3408, height 0x08E0 = 2272, resolutions 72.0, frame count 1, depth 24, color table ID 0xFFFF, Canon field 00 03 00 00. Its children start at entry offset 90 (file offset 34587): "JPEG" (size 12) and "free" (size 10). The track is a JPEG track (R-CR3-15).

### TV-6: sample location of track 1

- Offset 34676: 00 00 00 18 73 74 73 7A 00 00 00 00 00 00 00 00 00 00 00 01 00 0B 32 44. "stsz": sample size 0, 1 entry, first size 0x0B3244 = 733764.
- Offset 34712: 00 00 00 18 63 6F 36 34 00 00 00 00 00 00 00 01 00 00 00 00 00 06 74 00. "co64": 1 entry, first offset 0x067400 = 422912.
- The sample (P1) occupies offsets 422912 to 1156675. It starts with FF D8 FF DB and ends with FF D9.

### TV-7: THMB box

Offset 18264: 00 00 3D F8 54 48 4D 42 00 00 00 00 00 A0 00 78 00 00 3D DF 00 01 00 00 FF D8 FF DB. Size 0x3DF8 = 15864, type "THMB", version 0, width 160, height 120, L = 0x3DDF = 15839. The JPEG (P3) occupies 18288 to 34126. The box ends at 34128, so 1 fill byte follows the JPEG.

### TV-8: preview holder box and PRVW

Offset 101952: 00 04 E5 58 75 75 69 64 EA F4 2B 5E 1C 98 4B 88 B9 FB B7 DC 40 6E 4D 16 00 00 00 00 00 00 00 01 00 04 E5 38 50 52 56 57 00 00 00 00 00 01 06 54 04 38 00 01 00 04 E5 1F FF D8 FF DB.
- uuid box, size 0x04E558 = 320856, extended type of the preview holder box.
- At 101952 + 32 = 101984 the box "PRVW", size 0x04E538 = 320824.
- PRVW payload: version 0, 1, width 0x0654 = 1620, height 0x0438 = 1080, 1, L = 0x04E51F = 320799.
- The JPEG (P2) occupies 102008 to 422806.

### TV-9: SOF0 of the track-1 JPEG

At byte 136 of the P1 stream: 0xFF 0xC0 0x00 0x11 0x08 0x08 0xE0 0x0D 0x50 0x03 0x01 0x21 0x00 0x02 0x11 0x01 0x03 0x11 0x01. Lf = 17, P = 8, Y = 2272, X = 3408, Nf = 3. Components: (1, H 2, V 1, Tq 0), (2, H 1, V 1, Tq 1), (3, H 1, V 1, Tq 1). Candidate sizes of the file: P1 3408 x 2272 = 7742976, P2 1620 x 1080 = 1749600, P3 160 x 120 = 19200. R-CR3-37 chooses P1.

### TV-10: CTMD records and blocks

- Offset 9696768 (first byte of the metadata sample, 118870 bytes): 18 00 00 00 01 00 00 00 00 01 00 00. Record size 0x18 = 24 (LE), type 1.
- Offset 9706254: 16 A0 01 00 08 00 01 01 01 01 FF FF 0A A0 01 00 7C 92 00 00 49 49 2A 00 08 00 00 00 04 00. Record size 0x01A016 = 106518, type 8. First block: size 0x01A00A = 106506, tag 0x927C. TIFF block at 9706274: little-endian, IFD0 at 8, 4 entries.
- The ColorData entry at TIFF offset 10: 01 40 03 00 48 0E 00 00 3A 00 00 00. Tag 0x4001, type 3, count 0x0E48 = 3656, values at TIFF offset 0x3A = 58, file offset 9706274 + 58 = 9706332.

### TV-11: ColorData values

- File offset 9706332: 21 00. c[0] = 33, table ColorData10.
- File offset 9706502 (c[85] to c[89]): 40 07 00 04 00 04 86 06 65 13. Values 1856, 1024, 1024, 1670, 4965.
- File offset 9707018 (c[343] to c[346]): 00 02 00 02 00 02 00 02. Values 512, 512, 512, 512.
- File offset 9707952 (c[810] to c[812]): BF 31 B8 36 BF 31. Values 12735, 14008, 12735.
- Multipliers (R-CR3-32): mR = 1.8125, mG = 1, mB = 1.630859375.

### TV-12: SensorInfo

CMT3 payload starts at 2384. The entry E0 00 03 00 11 00 00 00 22 2F 00 00: tag 0x00E0, SHORT, count 17, values at CMT3 offset 0x2F22 = 12066 (file offset 14450). The first 9 values: 22 00 00 0E 52 09 01 00 01 00 9C 00 6C 00 EB 0D 4B 09 = 34, 3584, 2386, 1, 1, 156, 108, 3563, 2379. Sensor size 3584 x 2386. Camera crop from (156, 108) to (3563, 2379), which is 3408 x 2272 pixels.

### TV-13: CMP1 and variant

- Offset 35687 (track 3 of S14, RAW): 00 00 00 3C 43 4D 50 31 FF 00 00 30 01 00 00 00 00 00 0E 00 00 00 09 52 00 00 0E 00 00 00 09 52 0E 40 00 00. Payload bytes 8 to 11 = 0x0E00 = 3584, 12 to 15 = 0x0952 = 2386, byte 26 = 0x00: variant X1 (R-CR3-2).
- The CMP1 payload of track 3 of S14 C-RAW starts FF 10 00 30 02 00 00 00 00 00 0E 00 00 00 09 52 00 00 0E 00 00 00 09 52 0E 40 03 00. Byte 26 = 0x03: variant X2.

### TV-14: minimal JPEG

The 141-byte stream below is a baseline JPEG (S5) of 8 x 8 pixels with one component. All 64 quantization values are 1. The DC table has one code, the 1-bit code 0 for category 0. The AC table has one code, the 1-bit code 0 for the value 0x00 (end of block). The entropy data is the single byte 0x3F: the bits 0 (DC difference 0) and 0 (end of block), then six 1-bits of padding. The stream decodes to 64 gray pixels of the value 128.

FF D8 FF DB 00 43 00, then 64 bytes 01,
FF C0 00 0B 08 00 08 00 08 01 01 11 00,
FF C4 00 14 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00,
FF C4 00 14 10 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00,
FF DA 00 08 01 01 00 00 3F 00 3F FF D9.

### TV-15: orientation

Decoded preview 3 x 2 with pixel values a b c / d e f (rows). Orientation 6 gives the 2 x 3 output d a / e b / f c. Orientation 8 gives c f / b e / a d. Orientation 3 gives f e d / c b a. A decoded preview of 2 x 3 (taller than wide) with orientation 6 stays 2 x 3 (R-CR3-38).

### Fixture recipes

The fixtures are complete small files in the CR3 structure for the decoder tests. The JPEG streams in them are one-component baseline streams in the form of TV-14 (DQT with all values 1, a DC table with the code 0 for category 0 and, for a gray value other than 128, the code 10 for one category c, and an AC table with only the end-of-block code). Only the first 8 x 8 block codes a DC difference, so every pixel has the stated gray value.

F1 (928 bytes): CMT1 Orientation 6, P1 32 x 16 gray 200, P2 16 x 8 gray 96, P3 8 x 8 gray 128. No "mvhd", "tkhd" or metadata track.

0000: 00 00 00 18 66 74 79 70 63 72 78 20 00 00 00 01
0010: 63 72 78 20 69 73 6F 6D 00 00 02 20 6D 6F 6F 76
0020: 00 00 01 31 75 75 69 64 85 C0 B6 87 82 0F 11 E0
0030: 81 11 F4 CE 46 2B 6A 48 00 00 00 26 43 4E 43 56
0040: 43 61 6E 6F 6E 43 52 33 5F 30 30 31 2F 30 30 2E
0050: 31 30 2E 30 30 2F 30 30 2E 30 30 2E 30 30 00 00
0060: 00 4E 43 4D 54 31 49 49 2A 00 08 00 00 00 03 00
0070: 0F 01 02 00 06 00 00 00 32 00 00 00 10 01 02 00
0080: 0D 00 00 00 38 00 00 00 12 01 03 00 01 00 00 00
0090: 06 00 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61
00A0: 6E 6F 6E 20 45 4F 53 20 52 36 00 00 00 00 00 A5
00B0: 54 48 4D 42 00 00 00 00 00 08 00 08 00 00 00 8D
00C0: 00 01 00 00 FF D8 FF DB 00 43 00 01 01 01 01 01
00D0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
00E0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
00F0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0100: 01 01 01 01 01 01 01 01 01 01 01 FF C0 00 0B 08
0110: 00 08 00 08 01 01 11 00 FF C4 00 14 00 01 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 FF C4
0130: 00 14 10 01 00 00 00 00 00 00 00 00 00 00 00 00
0140: 00 00 00 00 FF DA 00 08 01 01 00 00 3F 00 3F FF
0150: D9 00 00 00 E7 74 72 61 6B 00 00 00 DF 6D 64 69
0160: 61 00 00 00 21 68 64 6C 72 00 00 00 00 00 00 00
0170: 00 76 69 64 65 00 00 00 00 00 00 00 00 00 00 00
0180: 00 00 00 00 00 B6 6D 69 6E 66 00 00 00 AE 73 74
0190: 62 6C 00 00 00 76 73 74 73 64 00 00 00 00 00 00
01A0: 00 01 00 00 00 66 43 52 41 57 00 00 00 00 00 00
01B0: 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00
01C0: 00 00 00 20 00 10 00 48 00 00 00 48 00 00 00 00
01D0: 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00
01E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
01F0: 00 00 00 00 00 18 FF FF 00 03 00 00 00 00 00 0C
0200: 4A 50 45 47 00 00 00 00 00 00 00 18 73 74 73 7A
0210: 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00 91
0220: 00 00 00 18 63 6F 36 34 00 00 00 00 00 00 00 01
0230: 00 00 00 00 00 00 03 0F 00 00 00 C7 75 75 69 64
0240: EA F4 2B 5E 1C 98 4B 88 B9 FB B7 DC 40 6E 4D 16
0250: 00 00 00 00 00 00 00 01 00 00 00 A7 50 52 56 57
0260: 00 00 00 00 00 01 00 10 00 08 00 01 00 00 00 8F
0270: FF D8 FF DB 00 43 00 01 01 01 01 01 01 01 01 01
0280: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0290: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
02A0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
02B0: 01 01 01 01 01 01 01 FF C0 00 0B 08 00 08 00 10
02C0: 01 01 11 00 FF C4 00 15 00 01 01 00 00 00 00 00
02D0: 00 00 00 00 00 00 00 00 00 00 09 FF C4 00 14 10
02E0: 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
02F0: 00 FF DA 00 08 01 01 00 00 3F 00 9F E3 FF D9 00
0300: 00 00 01 6D 64 61 74 00 00 00 00 00 00 00 A1 FF
0310: D8 FF DB 00 43 00 01 01 01 01 01 01 01 01 01 01
0320: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0330: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0340: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0350: 01 01 01 01 01 01 FF C0 00 0B 08 00 10 00 20 01
0360: 01 11 00 FF C4 00 15 00 01 01 00 00 00 00 00 00
0370: 00 00 00 00 00 00 00 00 00 0A FF C4 00 14 10 01
0380: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0390: FF DA 00 08 01 01 00 00 3F 00 A4 00 00 1F FF D9

Annotations:
- 0x000 "ftyp". 0x018 "moov" (544 bytes). 0x020 Canon metadata box (305 bytes). 0x038 CNCV. 0x05E CMT1 (78 bytes): TIFF with Make "Canon", Model "Canon EOS R6", Orientation 6. 0x0AC THMB (165 bytes): version 0, 8 x 8, L = 0x8D = 141, the stream of TV-14.
- 0x151 "trak" (231 bytes). 0x159 "mdia". 0x161 "hdlr", handler "vide". 0x182 "minf". 0x18A "stbl". 0x192 "stsd" with the CRAW entry at 0x1A2 (102 bytes, 32 x 16, Canon field 00 03 00 00, child "JPEG" at 0x1F8). 0x208 "stsz": 145. 0x220 "co64": 0x30F = 783.
- 0x238 preview holder box (199 bytes). 0x258 "PRVW" (167 bytes): version 0, 16 x 8, L = 0x8F = 143. The JPEG has the DC categories 0 and 9 and the difference -256 (gray 96).
- 0x2FF "mdat" with size field 1 and extended size 0xA1 = 161. Payload at 0x30F: P1, 145 bytes, DC categories 0 and 10, difference 576 (gray 200).

Expected result for F1: candidates P1 32 x 16, P2 16 x 8, P3 8 x 8. The decoder delivers P1, rotated by orientation 6 to 16 x 32 pixels, every pixel R = G = B = 200. Variant: unknown (no CRX track). ColorSpace reported as 1 (no CMT2).

F2 (928 bytes): F1 with the byte at 0x30F changed from FF to 00. P1 no longer starts with FF D8 and is dropped (R-CR3-36). Expected result: P2 rotated to 8 x 16 pixels, every pixel 96.

F3 (2741 bytes): F1 with these changes. CMT1 Orientation 1 (the byte at 0x090 is 01). A second "trak" follows the first inside "moov": "mdia" with "hdlr" (handler "meta"), "minf", "stbl" with "stsd" (one entry of size 16 with data format "CTMD" and 8 more bytes 00 00 00 00 00 00 00 01), "stsz" (sample size 1672, count 1) and "co64" (offset of the metadata sample). The "mdat" payload holds P1 and then the 1672-byte metadata sample. That sample is one record: 88 06 00 00 08 00 01 01 01 01 FF FF (size 1672, type 8), one block 7C 06 00 00 7C 92 00 00 (size 1660, tag 0x927C), and the TIFF block 49 49 2A 00 08 00 00 00 01 00 01 40 03 00 2D 03 00 00 1A 00 00 00 00 00 00 00 followed by 813 SHORT values (tag 0x4001, count 813, values at TIFF offset 26). The values are 0 except c[0] = 33, c[85] to c[89] = 1856, 1024, 1024, 1670, 4965, c[343] to c[346] = 512, and c[810] to c[812] = 12735, 14008, 12735. Expected result: P1 32 x 16, gray 200, not rotated. Reported white balance multipliers 1.8125, 1, 1.630859375. Black levels 512, 512, 512, 512. NormalWhiteLevel 12735, SpecularWhiteLevel 14008.

## Open questions

Each item states what is not confirmed by a permitted source or a measurement, and the behavior the decoder must take.

- OQ-1 CRX lossless coding (X1). No permitted source describes the bitstream (S12 is general). Fallback: preview path (R-CR3-1).
- OQ-2 CRX lossy coding (X2). As OQ-1. Fallback: preview path.
- OQ-3 Dual Pixel RAW data (X3, track 5). Its content is not described by any permitted source. Fallback: ignored.
- OQ-4 Files with several images (X4, raw burst). Not measured. S7 lists RawBurstInfo tags. Fallback: the first sample of each track is used (R-CR3-17), and the variant is reported as X4.
- OQ-5 HEVC previews (X5, 6 of 110 measured files: EOS R5 Mark II and EOS R8). The Canon boxes "CISZ", "GRID" and "hvcC" and the HEVC tile layout are not described here. The camera setting that produces these files is not established (S10 links HEIF output to the HDR PQ setting). Fallback: no JPEG candidate, error "no supported preview" (R-CR3-37).
- OQ-6 Orientation values other than 1 and 8. One measured file has the value 8 and stores its previews unrotated (M16). The values 6, 3 and the mirrored values 2, 4, 5, 7 were not measured. Fallback: R-CR3-38 for every value, with the exception for previews taller than wide.
- OQ-7 Meaning of NormalWhiteLevel, SpecularWhiteLevel and LinearityUpperMargin. S7 names them but does not define them, and no CRX data was decoded. Fallback: reported only (R-CR3-33).
- OQ-8 Assignment of the four PerChannelBlackLevel values to color filter positions, and the color filter pattern of the CRX images. Fallback: reported in stored order (R-CR3-33).
- OQ-9 Bit depth, color filter arrangement, the meaning of the IAD1 rectangles after the first one (they look like masked border areas) and of the other CMP1 fields. Fallback: not used.
- OQ-10 Camera color matrices. The file carries no documented matrix. Fallback: none needed for the preview path; a later raw path uses the method of arw.md.
- OQ-11 ColorSpace 0xFFFF (Uncalibrated, which S9 uses for Adobe RGB images). Not measured, so it is not known whether the previews of such files are Adobe RGB values. Fallback: the preview is delivered as sRGB-encoded values and the value is reported (R-CR3-35, R-CR3-41).
- OQ-12 ColorDataVersion values outside the table. All 110 measured files have a version in the table (M11). S7 lists 67 (EOS R8 Mark II) and 64 (EOS R1) without a measured file, and later bodies can bring new versions. Fallback: levels and white balance reported as unknown (R-CR3-31).
- OQ-13 The aspect ratio setting of the camera (1:1, 4:3, 16:9). The track-1 JPEG covers the full sensor area in the native aspect ratio in such files (M8). S7 lists AspectInfo and CropInfo maker-note tags, whose application was not measured. Fallback: the preview is delivered uncropped.
- OQ-14 CCTP and its CCDT entries, CTBO entries 4 and 5, and the fields of THMB and PRVW marked "not used". Fallback: ignored (R-CR3-21).
- OQ-15 TIFF blocks with the byte order 4D 4D. Not observed (M9). Fallback: read by R-CR3-22.
- OQ-16 The black bands of the THMB thumbnail (rows 0 to 5 and 114 to 119, M5). Fallback: the thumbnail is delivered as decoded, only when no larger candidate decodes.
- OQ-17 Files with "stco" in place of "co64", or with more than one chunk. Not observed (M3). Fallback: R-CR3-17.
- OQ-18 Cinema RAW Light movie files share the brand "crx " (S8 lists Canon movie tags). They have another file extension and were not measured. Fallback: a file that passes R-CR3-4 is handled by this document. When it has no JPEG candidate, R-CR3-37 reports an error.
