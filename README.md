<p align="center">
  <img src="packaging/linux/icons/hicolor/128x128/apps/pandite.png" width="112" height="112" alt="Pandite logo">
</p>

<h1 align="center">Pandite</h1>

<p align="center"><b>Sick and tired of slow or unresponsive image viewers? Pandite is for you.</b></p>

<p align="center">
  <a href="https://github.com/xls/pandite/actions/workflows/ci.yml"><img src="https://github.com/xls/pandite/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/xls/pandite/releases"><img src="https://img.shields.io/github/v/release/xls/pandite?include_prereleases&label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-informational" alt="Windows, macOS, Linux">
  <img src="https://img.shields.io/badge/arch-x64%20%7C%20ARM64-informational" alt="x64, ARM64">
  <img src="https://img.shields.io/badge/rust-1.87%2B-orange" alt="Rust 1.87 or later">
  <img src="https://img.shields.io/badge/GPU-Direct3D%2012%20%7C%20Metal%20%7C%20Vulkan-success" alt="Direct3D 12, Metal, Vulkan">
  <img src="https://img.shields.io/badge/network-none-success" alt="No network code">
</p>

<p align="center">
  <a href="#performance">Performance</a> |
  <a href="#formats">Formats</a> |
  <a href="#features">Features</a> |
  <a href="#install">Install</a> |
  <a href="#build">Build</a>
</p>

Pandite is a GPU image viewer for Windows, macOS and Linux, written in
Rust with wgpu. It is one executable of 12 MiB. It has no account, no
telemetry and no network code.

**Made for** people who are sick of image viewers that lock up and take
their time to show a picture.

## Performance

Time from program start to the picture on screen. Machine: AMD Ryzen 9
9950X3D, Windows 11, Direct3D 12.

| File | GPU | First picture | Full resolution |
|---|---|---|---|
| Sony ARW, 9504 x 6336 (60 MP) | NVIDIA RTX 5090 | 47 to 60 ms | 47 to 60 ms |
| Sony ARW, 9504 x 6336 (60 MP) | AMD Radeon integrated | 356 to 389 ms | 356 to 389 ms |
| Sony JPEG, camera original | AMD Radeon integrated | 181 ms | 571 ms |
| JPEG, 1280 x 720 | NVIDIA RTX 5090 | 56 ms | 56 ms |

The test suite fails when a first picture takes longer than 200 ms for
ARW or 300 ms for JPEG on the discrete GPU.

How Pandite gets these times:

- Every file read and decode runs on a worker thread. The window never
  waits for a file, also not for a stalled cloud or network file.
- The next and previous images and the thumbnails decode ahead, inside a
  fixed memory budget. One thread stays free for the image you select.
- Camera files show their embedded preview first, then the full image.
- Decoders use SIMD. Textures get a full mipmap chain on the GPU.
- On Windows, Pandite stays in the background after you close it and keeps
  the GPU device and the thumbnails of the last folder. The next image
  opens without a new program start.

## Formats

37 file extensions. Details and limits: [docs/formats.md](docs/formats.md).

| Format | Extensions | Notes |
|---|---|---|
| JPEG | .jpg .jpeg .jpe .jfif | Baseline, progressive, lossless. Gray, YCbCr, RGB, CMYK, YCCK |
| PNG, APNG | .png .apng | Every color type and bit depth. Animation |
| GIF | .gif | Animation |
| WebP | .webp | Lossy, lossless, alpha. Animation |
| AVIF | .avif | 8, 10 and 12-bit, alpha, HDR (PQ, HLG) |
| HEIC, HEIF | .heic .heif .hif | Codec of the operating system. Not on Linux |
| JPEG XL | .jxl | Stills and animation, HDR |
| TIFF | .tif .tiff | Integer to 32-bit, float to 64-bit. LZW, Deflate, PackBits, JPEG, CCITT |
| BMP | .bmp .dib | 1 to 32 bits per pixel, RLE, bit fields |
| Targa | .tga | Color-mapped, true-color, gray, RLE |
| Icon, cursor | .ico .cur | Largest image in the file |
| QOI | .qoi | |
| DDS | .dds | Legacy and DX10 headers, BC compression |
| KTX2 | .ktx2 | Uncompressed, BC, ETC, EAC, ASTC |
| OpenEXR | .exr | HDR, tone mapped |
| Radiance HDR | .hdr | HDR, tone mapped |
| SVG | .svg | System fonts, embedded and linked images |
| Photoshop | .psd | Stored composite image |
| Camera RAW | .dng .cr2 .cr3 .nef .arw .raf .orf .rw2 .pef .srw | See docs/formats.md |

Writes PNG, JPEG, lossless WebP and BMP.

## Features

- **Gallery strip.** Thumbnails of the folder along the bottom. G shows
  or hides it.
- **Zoom.** Fit, 1:1 and zoom at the pointer. Past 3x, each image pixel
  is a crisp square.
- **Edit.** Rotate, flip, crop and resize with undo. Save as PNG, JPEG,
  WebP or BMP.
- **Bulk convert.** Convert and resize a set of files or a whole folder.
- **Text in images (Windows).** Select and copy the text in a picture
  with the mouse. The Windows recognizer reads it offline.
- **Image information.** I shows file size, pixel size, codec, channels,
  bit depth, compression ratio, color profile and camera settings.
- **Copy.** Ctrl+C copies the image, Ctrl+Shift+C copies the file path.
- **Delete.** Del sends the file to the Recycle Bin or the Trash and
  shows the next image. Shift+Del asks first, then deletes the file for good.
- **Select.** Space or the check box on a thumbnail selects images. Del
  and bulk convert then act on the selected images.
- **Color.** Embedded ICC profiles convert to sRGB. HDR images are tone
  mapped. Photos turn upright from their EXIF orientation.
- **Stall safe.** A file stuck on a cloud drive cannot freeze the window.
  Esc always quits at once.
- **Offline.** No account, no telemetry, no network code. CI checks that
  no networking library is linked.

Keys: [docs/shortcuts.md](docs/shortcuts.md), or F1 in the program.
Text recognition: [docs/text-recognition.md](docs/text-recognition.md).

## Install

Release packages come from the
[releases page](https://github.com/xls/pandite/releases).

| System | Package | Steps |
|---|---|---|
| Windows x64, ARM64 | `pandite-<version>-<target>-setup.exe` | Run it. It installs for your user, without administrator rights, and associates image files. It is not code-signed: at the SmartScreen warning, select More info, then Run anyway. |
| Windows x64, ARM64, portable | `pandite-<version>-<target>.zip` | Extract. Run `pandite.exe --register` once to add Pandite to Open with and Default apps. |
| macOS 11+, universal | `pandite-<version>-universal-apple-darwin.dmg` | Drag to Applications. The app is not notarized: allow it in Privacy and Security. |
| Debian, Ubuntu x86_64, glibc 2.35+ | `pandite_<version>_amd64.deb` | `sudo apt install ./pandite_<version>_amd64.deb` |
| Fedora, openSUSE x86_64, glibc 2.35+ | `pandite-<version>-1.x86_64.rpm` | `sudo dnf install ./pandite-<version>-1.x86_64.rpm` |
| Linux x86_64, glibc 2.35+ | `pandite-<version>-x86_64-unknown-linux-gnu.tar.gz` | Extract. Run `./install.sh` to install for your user. |
| Windows, macOS, Linux x86_64 with Node.js 18+ | npm package `pandite` (installer) | `npx pandite` installs the latest release for your user. It checks the download against `SHA256SUMS.txt`. `npx pandite update` installs a newer release, `npx pandite -v <version>` a named one, `npx pandite uninstall` removes Pandite. |

On Windows, HEIC needs "HEIF Image Extensions" and "HEVC Video
Extensions" from the Microsoft Store.

## Command line

```text
pandite [options] [--] [<image>]

  --config-dir <dir>  keep settings in this folder
  --background        Windows: start hidden and wait for the first image
  --quit              Windows: end the pandite that waits in the background
  --register          Windows: add pandite to Open with and Default apps
                      for every supported file type (this user only)
  --unregister        Windows: remove what --register added
  -h, --help          show this help
  -V, --version       show the version
```

Settings file: `%APPDATA%\pandite\settings.ini` on Windows,
`~/Library/Application Support/pandite/settings.ini` on macOS,
`~/.config/pandite/settings.ini` on Linux. The file explains each setting.

## Build

Rust 1.87 or later. Linux needs the libraries in
[docs/building.md](docs/building.md).

```sh
cargo build --release -p pandite-app
./target/release/pandite
```

Store packaging: [docs/store-submission.md](docs/store-submission.md).

## License

[MIT](LICENSE). Third-party licenses:
[docs/THIRD-PARTY-NOTICES.md](docs/THIRD-PARTY-NOTICES.md).
