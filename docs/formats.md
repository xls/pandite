# Supported formats

pandite opens the formats below on Windows, macOS and Linux, with one
exception: HEIC and HEIF need a codec of the operating system. Every
decoder applies the EXIF orientation, so photos come out upright. Embedded
ICC color profiles (wide-gamut RGB, CMYK) are converted to sRGB.

`packaging/extensions.txt` is the authoritative extension list. The codecs
crate prints it with `cargo run -p pandite-codecs --example extensions`.

## Reading

| Format | Extensions | Notes |
|---|---|---|
| JPEG | .jpg .jpeg .jpe .jfif | Baseline, progressive and lossless. Grayscale, YCbCr, RGB, CMYK and YCCK. |
| PNG and APNG | .png .apng | Every color type and bit depth. APNG animations play. |
| GIF | .gif | Animations play. |
| WebP | .webp | Lossy, lossless, alpha. Animations play. |
| BMP | .bmp .dib | 1 to 32 bits per pixel, RLE4 and RLE8, bit fields, embedded JPEG and PNG. |
| TIFF | .tif .tiff | Gray, RGB, RGBA, CMYK, YCbCr and palette images. Integer samples up to 32 bits, floating-point samples up to 64 bits. LZW, Deflate, PackBits, JPEG and CCITT compression. |
| Targa | .tga | Color-mapped, true-color and grayscale, uncompressed and RLE. |
| Icon and cursor | .ico .cur | Shows the largest image in the file. |
| QOI | .qoi | |
| AVIF | .avif | 8, 10 and 12-bit, alpha, HDR (PQ and HLG). An animated AVIF shows its primary still image when the file has one. |
| HEIC and HEIF | .heic .heif .hif | Operating system codec. See "HEIC and HEIF" below. |
| JPEG XL | .jxl | Stills and animations. |
| DDS | .dds | Shows the top mip level of the first surface. Legacy and DX10 headers, BC compression. |
| KTX2 | .ktx2 | Shows the top mip level. Uncompressed, BC, ETC, EAC and ASTC data. Basis Universal data is not supported. |
| OpenEXR | .exr | HDR. Shows the largest resolution level of the first layer. |
| Radiance HDR | .hdr | HDR. |
| SVG | .svg | Text uses the system fonts. Embedded images and linked images with relative paths are drawn. |
| Photoshop | .psd | Shows the merged image that Photoshop stores in the file. |
| Camera RAW | .dng .cr2 .cr3 .nef .arw .raf .orf .rw2 .pef .srw | See "Camera RAW" below. |

### Camera RAW

pandite develops the sensor data of the formats in this table. The embedded
JPEG preview of the camera shows first, then the developed image replaces it.
A file in a storage variant that the table lists under "Preview only" shows
its embedded preview, the largest one in the file.

| Format | Developed from sensor data | Preview only |
|---|---|---|
| DNG | Uncompressed and lossless JPEG data, CFA or linear RGB | Other compressions |
| Sony ARW | Uncompressed, compressed and lossless compressed files | Variants that the specification leaves out |
| Canon CR2 | RAW files (lossless JPEG, 12 and 14 bits) | sRAW and mRAW |
| Canon CR3 | None | All files. The file must contain a JPEG preview |
| Nikon NEF | Uncompressed, lossless compressed and lossy compressed files, 12 and 14 bits | High Efficiency files (Z 8, Z 9, Z f), small RAW, D1 family, Coolpix NEF |
| Fujifilm RAF | Uncompressed files, X-Trans and Bayer sensors | Compressed files, SuperCCD and EXR sensors |
| Olympus ORF | All measured variants: uncompressed and lossless compressed, 12 and 14 bits | Files of an unknown layout |
| Panasonic RW2 | RawFormat 4 to 7 (LX3 to GH5, GH5S, G9, GH5M2, S1, S5) and older .RAW layouts | RawFormat 8 (GH6, GH7, G9M2, S5M2 and later) |
| Pentax PEF | Uncompressed, packed and Huffman compressed files, also monochrome | Other compressions. Pixel Shift files develop their first frame |
| Samsung SRW | Uncompressed files (NX10 to NX1100, EX1, WB2000) | Compressed files (NX300 and later) |

- Each decoder is written from a specification in `specs/raw/`. The
  specifications use public documents and measurements of public sample
  files only. See `specs/raw/PROVENANCE.md`.
- Colors are closest to the camera JPEG for Sony, Olympus, Canon EOS and
  Nikon files. Fujifilm files and some Panasonic, Pentax and Canon bodies
  have no color matrix for their model and use a generic matrix. Their
  colors can differ from the camera JPEG.
- Samsung and Canon PowerShot files can show brighter than the camera JPEG.
- pandite applies no lens correction. A compact camera corrects distortion
  in its JPEG, so the developed image can show more distortion.
- A Canon CR3 file that contains only an HEVC preview (some EOS R5 Mark II
  and EOS R8 files) does not open.

### HEIC and HEIF

pandite reads HEIC and HEIF through the image codec of the operating system.

- Windows: install "HEIF Image Extensions" and "HEVC Video Extensions" from
  the Microsoft Store. Without them, HEIC files do not open.
- macOS: the codec is part of the system.
- Linux: HEIC and HEIF are not supported.

### HDR images

OpenEXR, Radiance HDR, HDR AVIF, HDR JPEG XL and floating-point TIFF files
decode to linear floating-point color. pandite tone maps them for the
screen.

### Photoshop

pandite shows the composite image that Photoshop stores with the file. It
does not combine layers. If a file was saved without "Maximize
Compatibility", the stored composite can be missing or out of date.

## Writing

Save as and bulk convert write these formats:

| Format | Extensions | Notes |
|---|---|---|
| PNG | .png | Lossless. 8-bit images stay 8-bit. 16-bit and floating-point images are written as 16-bit. |
| JPEG | .jpg .jpeg .jpe .jfif | Quality 1 to 100. Transparent areas become white. Each side is limited to 65535 pixels. |
| WebP | .webp | Lossless only. pandite does not write lossy WebP. Each side is limited to 16383 pixels. |
| BMP | .bmp .dib | 24-bit, or 32-bit when the image has transparency. |

For Save as, the format follows the extension of the file name that you
choose. For bulk convert, you choose one format in the dialog, or you keep
the format of each file.

## Planned

3D formats (STL, OBJ, FBX, glTF) are planned for a later release. This
release does not open them.
