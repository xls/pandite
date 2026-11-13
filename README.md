# pandite

pandite is an image viewer for Windows, macOS and Linux. Open a picture and
it is on screen. Press the arrow key and the next one is already there. It
works offline, and a file that is stuck on a cloud drive cannot freeze it.

## Why pandite

Most viewers stop and wait when a file is slow to arrive. pandite does
not. Every file read and every decode runs on a worker thread, so the
window keeps drawing and Esc always answers. When a file on a cloud drive
or a network share takes its time, pandite shows a loading screen that you
can leave at once. On Windows, pandite also knows which OneDrive files are
only placeholders. It reads ahead only the files that are already on your
disk, so browsing a folder does not start downloads behind your back.

pandite reads ahead. After it shows an image, it decodes the next and the
previous images and the thumbnails for the gallery strip, inside a fixed
memory budget. When you press the arrow key, the work is usually done
already.

pandite draws with the GPU (Direct3D 12, Metal, Vulkan or OpenGL ES).
Zoomed out, images are filtered through a full mipmap chain, so fine detail
stays clean instead of shimmering. Zoomed in past 3x, every image pixel is
a crisp square.

pandite is one executable. It has no account, no telemetry and no network
code. On every push, the CI checks that no networking library is linked
into the program on any platform.

## Features

- Opens 37 file extensions: JPEG, PNG, APNG, GIF, WebP, BMP, TIFF, Targa,
  ICO, QOI, AVIF, HEIC, JPEG XL, DDS, KTX2, OpenEXR, Radiance HDR, SVG,
  Photoshop and ten camera RAW formats. See [docs/formats.md](docs/formats.md).
- Plays animated GIF, APNG, WebP and JPEG XL.
- Shows HDR images (OpenEXR, Radiance HDR, HDR AVIF) with tone mapping.
- Converts embedded color profiles to sRGB and turns photos upright from
  their EXIF orientation.
- Fit to window, 1:1, and zoom at the mouse pointer. F11 for full screen.
- A gallery strip of thumbnails along the bottom. Scroll it with the wheel.
- Rotate, flip, crop and resize. Edits change only the view until you
  choose Save as, and pandite asks before it writes over the original.
- Save as PNG, JPEG, WebP (lossless) or BMP.
- Bulk convert and resize a set of files or a whole folder.
- Select and copy the text in an image, on Windows. See
  [Text in images](#text-in-images).
- A button that copies the path of the current file.
- Remembers the window size and position. On first open, the window takes
  a share of the screen and the image fits inside it.
- A minimal dark interface that steps aside for the image and follows the
  light or dark theme of the system.

## Text in images

On Windows, pandite reads the text in the image on screen. Point at a word
and the pointer becomes a text cursor. Drag across words to select them,
double-click a word or triple-click a line, or press Ctrl+A for all the
text. Ctrl+C then copies the text, with one line for each line in the
image.

The reading happens on your computer, with the text recognizer that comes
with Windows, so it works offline. It starts in the background once the
image is on screen, so the image always comes first. It needs an OCR
language, and Windows installs one with most display languages. Without
one, pandite shows no text features. macOS and Linux have no text
features yet. To turn the feature off, set `ocr = off` in the settings
file. [docs/text-recognition.md](docs/text-recognition.md) has the details.

## Install

### Windows

1. Download `pandite-<version>-x86_64-pc-windows-msvc.zip` from the
   [releases page](https://github.com/xls/pandite/releases). On an ARM64
   computer, download the `aarch64-pc-windows-msvc` zip.
2. Extract the zip to a folder.
3. Start `pandite.exe`.
4. To open images with pandite from File Explorer, run this command once
   in the folder of the program:

   ```text
   pandite.exe --register
   ```

   The command adds pandite to "Open with" and to Default apps for each
   supported file type, for your user account only. It needs no
   administrator rights. pandite then opens each file type that you did
   not give to a different program in Settings. Windows keeps the choices
   you made in Settings. The command opens the Default apps page, where
   you can change them.
5. If you move the program to a different folder, run the command again.
6. To remove the entries, run `pandite.exe --unregister`.

A Microsoft Store version is in preparation. It registers the file types
itself and adds automatic updates.

### macOS

pandite needs macOS 11 or later and runs natively on Apple Silicon and Intel.

1. Download `pandite-<version>-universal-apple-darwin.dmg` from the
   [releases page](https://github.com/xls/pandite/releases).
2. Open the disk image and drag pandite to Applications.
3. The app is not notarized. When macOS blocks the first start, open
   System Settings, Privacy and Security, and select Open Anyway. If the
   button does not show, run this command in Terminal:

   ```sh
   xattr -dr com.apple.quarantine /Applications/pandite.app
   ```

### Linux

pandite needs an x86_64 system with glibc 2.35 or later, and a Vulkan or
OpenGL ES driver (Mesa supplies both).

1. Download `pandite-<version>-x86_64-unknown-linux-gnu.tar.gz` from the
   [releases page](https://github.com/xls/pandite/releases).
2. Extract it:

   ```sh
   tar -xzf pandite-<version>-x86_64-unknown-linux-gnu.tar.gz
   cd pandite-<version>-x86_64-unknown-linux-gnu
   ```

3. Start pandite from this folder with `./pandite`, or install it for your
   user (`~/.local`) with `./install.sh`. To remove it, run
   `./install.sh --uninstall`.

HEIC and HEIF files do not open on Linux. On Windows and macOS, pandite
reads them through the image codec of the system. On Windows, that codec
comes from "HEIF Image Extensions" and "HEVC Video Extensions" in the
Microsoft Store.

To check a download, compare its SHA-256 hash with `SHA256SUMS.txt` on the
releases page.

## Quick start

1. Open an image. Use one of these methods:
   - Start pandite with the file: `pandite photo.jpg`.
   - Start pandite and press Ctrl+O.
   - On Windows and Linux, open the image with pandite from the file
     manager (Open with).
2. Press Right or Space for the next image. Press Left or Backspace for the
   previous image.
3. Turn the mouse wheel to zoom at the pointer. Drag the image to pan.
4. Press F to fit the image to the window. Press 1 for 1:1.
5. Press F1 to see every keyboard shortcut.

## Key shortcuts

On macOS, use Command for Ctrl and Option for Alt.

| Keys | Command |
|---|---|
| Right, Space / Left, Backspace | Next image / previous image |
| Home / End | First image / last image |
| F or 0 / 1 | Fit to window / actual size (1:1) |
| + / - | Zoom in / zoom out |
| F11 | Full screen |
| G | Show or hide the gallery strip |
| R / Shift+R | Rotate right / rotate left |
| H / V | Flip horizontal / flip vertical |
| C | Crop |
| Ctrl+R | Resize |
| Ctrl+B | Bulk convert and resize |
| Ctrl+Shift+S | Save as |
| Ctrl+C | Copy the selected text, or the file path when no text is selected |
| Ctrl+A | Select all the text in the image (Windows) |
| Ctrl+Z | Undo |
| Esc | Clear a text selection, cancel loading, close a dialog, leave full screen |
| F1 | Show all shortcuts |

The full list, with the mouse actions, is in
[docs/shortcuts.md](docs/shortcuts.md).

## Command line

```text
pandite [options] [--] [<image>]

  --config-dir <dir>  keep settings in this folder
  --register          Windows: add pandite to Open with and Default apps
                      for every supported file type (this user only)
  --unregister        Windows: remove what --register added
  -h, --help          show this help
  -V, --version       show the version
```

pandite keeps its settings in `%APPDATA%\pandite\settings.ini` on Windows,
`~/Library/Application Support/pandite/settings.ini` on macOS and
`~/.config/pandite/settings.ini` (or `$XDG_CONFIG_HOME/pandite`) on Linux.
In the Microsoft Store version, Windows keeps the file in the private
folder of the app. The file explains each setting. For example,
`ocr = off` turns off the reading of text in images.

## Build from source

1. Install Rust 1.87 or later from https://rustup.rs.
2. On Linux, install the libraries in [docs/building.md](docs/building.md).
3. Build and start pandite:

   ```sh
   cargo build --release -p pandite-app
   ./target/release/pandite
   ```

[docs/building.md](docs/building.md) gives the steps for each system and for
the release packages. [docs/store-submission.md](docs/store-submission.md)
gives the steps for the Microsoft Store.

## Status

This release is for 2D images. Support for 3D formats (STL, OBJ, FBX and
glTF) is planned for a later release. This release does not open them.

## License

pandite is released under the [MIT License](LICENSE). The licenses of the
libraries in pandite, the IJG attribution and the SIL Open Font License of
the embedded Fira Sans font are in
[docs/THIRD-PARTY-NOTICES.md](docs/THIRD-PARTY-NOTICES.md).
