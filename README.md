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
<div align="center">
  
[![BuyMeACoffee](https://raw.githubusercontent.com/pachadotdev/buymeacoffee-badges/main/bmc-donate-yellow.svg)](https://buymeacoffee.com/pacha)

</div>



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
  or hides it. It follows the folder: images that you add, remove or
  rename show up or leave within seconds, and the image on screen stays.
  A thin track under the thumbnails shows where you are in the folder.
- **Zoom.** Fit, 1:1 and zoom at the pointer. Past 3x, each image pixel
  is a crisp square.
- **Edit.** Rotate, flip, crop and resize with undo. Save as PNG, JPEG,
  WebP or BMP.
- **Bulk convert.** Convert and resize a set of files or a whole folder.
- **Text in images.** Select and copy the text in a picture with the
  mouse. Windows reads it with its own recognizer. On macOS and Linux, a
  recognizer built into Pandite reads Latin scripts and Chinese. Both work
  offline.
- **Image information.** I shows file size, pixel size, codec, channels,
  bit depth, compression ratio, color profile and camera settings.
- **Copy.** Ctrl+C copies the image, Ctrl+Shift+C copies the file path.
- **Delete.** Del sends the file to the Recycle Bin or the Trash and
  shows the next image. Shift+Del asks first, then deletes the file for good.
- **Select.** Space or Shift+arrows select images: a badge marks them in
  the gallery strip, and the bottom right corner shows "Marked" and the count. Del,
  M (move to a folder), Shift+M (copy to a folder) and bulk convert then
  act on the selected images.
- **Compare.** D sets one selected image against the current image with a
  slider, or shows two to eight selected images in tiles. In tiles, the
  arrow keys and Space add and remove images.
- **Pixel peek.** P shows a magnifier at the pointer, also on every image
  of a comparison.
- **Find a command.** F1 lists every command with its key. Type to search
  the list, and Enter does the command.
- **Color.** Embedded ICC profiles convert to sRGB. HDR images are tone
  mapped. Photos turn upright from their EXIF orientation.
- **Stall safe.** A file stuck on a cloud drive cannot freeze the window.
  Esc always quits at once.
- **Offline.** No account, no telemetry, no network code. CI checks that
  no networking library is linked.

Keys: [docs/shortcuts.md](docs/shortcuts.md), or F1 in the program.
Text recognition: [docs/text-recognition.md](docs/text-recognition.md).

## Install

### With npx (Windows, macOS, Linux)

This needs Node.js 18 or later.

```sh
npx pandite
```

The command always installs the latest release of Pandite for your user. It
reads the releases on GitHub each time it runs, and it checks the download
against `SHA256SUMS.txt`. When an older Pandite is installed, the same
command updates it.

To install a newer release only when one exists:

```sh
npx pandite update
```

To install one named release:

```sh
npx pandite -v 0.2.5
```

To remove Pandite:

```sh
npx pandite uninstall
```

`npx` keeps the installer that it downloaded before. To get the newest
installer also, add `@latest`:

```sh
npx pandite@latest
```

### From the release files

The files are on the
[releases page](https://github.com/xls/pandite/releases). In the commands,
replace `<version>` with the version of the file.

**Windows x64 and ARM64, setup program.** Run
`pandite-<version>-<arch>-windows-setup.exe`. It installs for your user,
without administrator rights, and associates image files. It is not
code-signed: at the SmartScreen warning, select More info, then Run anyway.

**Windows x64 and ARM64, portable.** Extract
`pandite-<version>-<arch>-windows-portable.zip`. To add Pandite to Open
with and Default apps, run this command once in the extracted folder:

```powershell
.\pandite.exe --register
```

**macOS 11 or later.** Open `pandite-<version>-universal-macos.dmg` and
drag Pandite to Applications. The app is signed and notarized by Apple.

**Debian and Ubuntu x86_64, glibc 2.35 or later.**

```sh
sudo apt install ./pandite_<version>_amd64.deb
```

**Fedora and openSUSE x86_64, glibc 2.35 or later.**

```sh
sudo dnf install ./pandite-<version>-1.x86_64.rpm
```

**Other Linux x86_64, glibc 2.35 or later.** This installs for your user:

```sh
tar -xzf pandite-<version>-x86_64-linux.tar.gz
cd pandite-<version>-x86_64-linux
./install.sh
```

On Windows, HEIC needs "HEIF Image Extensions" and "HEVC Video
Extensions" from the Microsoft Store.

### Omarchy

`npx pandite` asks about both steps on a first install and does them for
you. To do both steps for a Pandite that is installed, run:

```sh
npx pandite --default --float
```

To do the steps by hand, use these commands. The desktop entry of Pandite
is `pandite.desktop`.

1. Make Pandite the default viewer for every format that it opens:

   ```sh
   desktop=$(ls ~/.local/share/applications/pandite.desktop /usr/share/applications/pandite.desktop 2>/dev/null | head -n 1)
   xdg-mime default pandite.desktop $(grep '^MimeType=' "$desktop" | cut -d= -f2 | tr ';' ' ')
   ```

2. Make sure that the default is set. This command must print
   `pandite.desktop`:

   ```sh
   xdg-mime query default image/jpeg
   ```

3. To open Pandite as a floating window, add this line at the bottom of
   `~/.config/hypr/hyprland.lua`:

   ```lua
   o.window("pandite", { tag = "+floating-window" })
   ```

The system can report a Sony ARW file as `image/tiff` and not as
`image/x-sony-arw`. Step 1 sets Pandite for both types, so ARW files open
in Pandite. To see the type of a file, run:

```sh
xdg-mime query filetype <file>
```

## Command line

```text
pandite [options] [--] [<image>]

  --config-dir <dir>  keep settings in this folder
  --background        Windows: start hidden and wait for the first image
  --quit              Windows: end the pandite that waits in the background
  --new-instance      Windows: open a second window, not the one that is open
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

Rust 1.87 or later on Windows, 1.91 or later on macOS and Linux. Linux
needs the libraries in [docs/building.md](docs/building.md).

```sh
cargo build --release -p pandite-app
./target/release/pandite
```

Store packaging: [docs/store-submission.md](docs/store-submission.md) (Microsoft Store), [docs/mac-app-store.md](docs/mac-app-store.md) (Mac App Store and App Sandbox test build).

## License

[MIT](LICENSE). Third-party licenses:
[docs/THIRD-PARTY-NOTICES.md](docs/THIRD-PARTY-NOTICES.md).
