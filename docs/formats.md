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

- DNG files are developed from the sensor data: black and white levels,
  as-shot white balance, the camera color matrix and the default crop.
  The result is a 16-bit image.
- Sony ARW files are developed from the sensor data. The embedded preview
  shows first, then the developed image replaces it. An ARW variant that
  pandite cannot read shows its embedded preview.
- CR2, CR3, NEF, RAF, ORF, RW2, PEF and SRW files show the JPEG preview
  that the camera embeds in the file. pandite uses the largest preview.
  Most cameras store it at full resolution. pandite does not develop the
  sensor data of these formats.
- A DNG file with sensor data in a compression that pandite cannot read
  also shows its embedded preview.

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
