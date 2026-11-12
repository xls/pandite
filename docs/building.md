# Building pandite

This guide gives the steps to build pandite from source and to make the
release packages on your computer. The GitHub workflows in
`.github/workflows/` use the same commands and scripts.

## Prerequisites on all systems

1. Install Rust with rustup from https://rustup.rs. pandite needs Rust 1.87
   or later.
2. Install Git.
3. If you make packages, install Node.js 18 or later. The packaging scripts
   use only the Node.js standard library.
4. Get the source:

   ```sh
   git clone https://github.com/xls/pandite.git
   cd pandite
   ```

## Windows

### Tools

1. Install Visual Studio 2022 or later, or the Build Tools for Visual Studio.
2. Select the workload "Desktop development with C++". This workload
   includes the MSVC linker and the Windows SDK.
3. If you build for ARM64, also select the component "MSVC C++ ARM64 build
   tools". Then add the Rust target:

   ```sh
   rustup target add aarch64-pc-windows-msvc
   ```

### Build

1. If you make a package for other computers, link the C runtime
   statically. Then the program does not need the Visual C++
   Redistributable. In PowerShell:

   ```powershell
   $env:RUSTFLAGS = "-C target-feature=+crt-static"
   ```

2. Build the release binary:

   ```sh
   cargo build --release -p pandite-app
   ```

3. Find the program at `target\release\pandite.exe`.
4. If you build for ARM64, add `--target aarch64-pc-windows-msvc`. The
   program is then at `target\aarch64-pc-windows-msvc\release\pandite.exe`.

The release workflow always sets this `RUSTFLAGS` value for Windows.
When you change `RUSTFLAGS`, cargo builds everything again.

### Portable zip

1. Make the zip with PowerShell 7:

   ```powershell
   pwsh packaging/windows/build-zip.ps1 -Exe target/release/pandite.exe -Version 0.1.0 -Target x86_64-pc-windows-msvc -OutDir dist
   ```

2. Find the zip at `dist\pandite-0.1.0-x86_64-pc-windows-msvc.zip`.

### MSIX package

The script needs `makeappx.exe` and `makepri.exe` from the Windows SDK. It
finds them in `C:\Program Files (x86)\Windows Kits\10\bin\<version>\x64\`.
If the SDK is in a different folder, set `PANDITE_MAKEAPPX` and
`PANDITE_MAKEPRI` to the full paths of the two tools.

1. Build the binary and the package in one step:

   ```sh
   node packaging/windows/build-msix.mjs --build
   ```

2. Find the package at `dist\pandite-<version>-x86_64-pc-windows-msvc.msix`.
3. For ARM64, add `--target aarch64-pc-windows-msvc`.
4. To package a binary that you built before, use `--exe <path>` instead of
   `--build`.
5. If the binary needs the Visual C++ Redistributable, the script prints a
   note. With `--require-static-crt`, the script stops instead.
6. To bundle the x64 and ARM64 packages into one file, run:

   ```sh
   node packaging/windows/build-msix.mjs --bundle dist/pandite-0.1.0-x86_64-pc-windows-msvc.msix dist/pandite-0.1.0-aarch64-pc-windows-msvc.msix
   ```

The package is unsigned. The Microsoft Store signs the packages that it
accepts. For the Store identity values and for signing, see
[store-submission.md](store-submission.md).

## macOS

### Tools

1. Install the Xcode command line tools:

   ```sh
   xcode-select --install
   ```

2. Add both Apple targets to Rust:

   ```sh
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   ```

### Build

1. Build both architectures. The deployment target matches
   `LSMinimumSystemVersion` in `packaging/macos/Info.plist`.

   ```sh
   MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release -p pandite-app --target aarch64-apple-darwin --target x86_64-apple-darwin
   ```

2. Make the universal app, the disk image and the zip:

   ```sh
   bash packaging/macos/build-app.sh 0.1.0 dist target
   ```

3. Find `pandite-0.1.0-universal-apple-darwin.dmg` and
   `pandite-0.1.0-universal-apple-darwin.zip` in `dist`.

The script signs the app ad hoc. It does not notarize the app. To change the
placeholder bundle identifier, set `PANDITE_BUNDLE_ID` before you run the
script.

## Linux

### Tools

1. Install a C toolchain and pkg-config. On Debian and Ubuntu:

   ```sh
   sudo apt install build-essential pkg-config
   ```

2. Install the window system and graphics libraries. On Debian and Ubuntu:

   ```sh
   sudo apt install libx11-dev libxkbcommon-dev libwayland-dev libvulkan-dev mesa-vulkan-drivers
   ```

pandite draws with Vulkan or with OpenGL ES. The Mesa drivers supply both.

### Build

1. Build the release binary:

   ```sh
   cargo build --release -p pandite-app
   ```

2. Find the program at `target/release/pandite`.
3. Make the tarball:

   ```sh
   bash packaging/linux/build-tarball.sh 0.1.0 dist target/release/pandite
   ```

4. Find `dist/pandite-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.

The release workflow builds the Linux binary on Ubuntu 22.04. That binary
needs glibc 2.35 or later.

## Tests and checks

1. Run the tests:

   ```sh
   cargo test --workspace
   ```

2. Run the linter:

   ```sh
   cargo clippy --workspace --all-targets -- -D warnings
   ```

3. Check the formatting:

   ```sh
   cargo fmt --all -- --check
   ```

4. Check the packaging files:

   ```sh
   node packaging/check.mjs
   ```

The GPU tests need a GPU adapter. When the computer has no adapter, these
tests print a notice and pass. To make a missing adapter fail the tests,
set `PANDITE_REQUIRE_GPU=1`.

The GitHub runners have no GPU, only software rasterizers. On Linux, the
CI runs the renderer tests on the Mesa software rasterizer with
`PANDITE_REQUIRE_GPU=1` and `PANDITE_FORCE_FALLBACK_ADAPTER=1`. In the
workspace test step, the CI sets `WGPU_BACKEND` to a backend that the
build does not contain, so the GPU tests skip there. Run the full GPU tests
on a computer with a real GPU.

## Packaging files

| File | Purpose |
|---|---|
| `packaging/extensions.txt` | The file extensions pandite opens. It must equal the output of `cargo run -p pandite-codecs --example extensions`. |
| `packaging/windows/AppxManifest.xml` | MSIX manifest with the file associations. |
| `packaging/windows/assets/` | MSIX logos at every scale. |
| `packaging/macos/Info.plist` | App bundle keys and document types. |
| `packaging/macos/pandite.icns` | macOS icon. |
| `packaging/linux/pandite.desktop` | Desktop entry with the MIME types. |
| `packaging/linux/icons/` | Linux icons. |
| `docs/THIRD-PARTY-NOTICES.md` | Licenses of the dependencies. |

After you change a file in this table, do the related step:

1. If the codecs crate gets a new format, update `packaging/extensions.txt`,
   the manifest, `Info.plist` and `pandite.desktop`. Then run
   `node packaging/check.mjs`.
2. If the application icon changes, run `node packaging/make-assets.mjs`.
3. If the dependencies change, run `node packaging/third-party-notices.mjs`.

## Release

1. Set the new version in `[workspace.package]` in `Cargo.toml`.
2. Commit the change and push it to `main`.
3. Make a tag that is `v` plus the version, and push it:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. Wait for the Release workflow. It builds every package, writes
   `SHA256SUMS.txt` and publishes the GitHub release.

If the tag and the `Cargo.toml` version are different, the workflow stops
before it builds. To test the packaging without a release, start the
Release workflow manually on a branch (Actions, Release, Run workflow). The
packages are then workflow artifacts.

## Continuous integration cost

When the repository is private, GitHub counts the Actions minutes against
the quota of the account. Windows minutes cost more than Linux minutes, and
macOS minutes cost the most. Each CI run uses two Windows jobs and two
macOS jobs. Public repositories do not pay for minutes on these standard
runners.
