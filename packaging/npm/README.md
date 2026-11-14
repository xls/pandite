# Pandite installer

Installs [Pandite](https://github.com/xls/pandite), a fast offline image
viewer, from its GitHub releases. Needs Node.js 18 or later. No
dependencies, no install scripts.

```sh
npx pandite
```

## Commands

| Command | Action |
|---|---|
| `npx pandite` | Install the latest release. |
| `npx pandite install -v 0.2.0` | Install that release. `0.2.0` and `v0.2.0` are equal. |
| `npx pandite update` | Install the latest release when it is newer than the installed one. Install it when Pandite is not installed. |
| `npx pandite uninstall` | Remove Pandite. `--uninstall` is the same. |
| `npx pandite --list` | Print the release versions, newest first. |

| Option | Action |
|---|---|
| `-v`, `--version <version>` | The Pandite version to install. |
| `--dry-run` | Print the downloads and the steps. Download and change nothing. |
| `--force` | Install again when the same version is installed. |
| `--system` | macOS: install in `/Applications`, not `~/Applications`. |
| `--installer-version` | Print the version of this installer. |
| `-h`, `--help` | Print the help. |

`pandite-installer` is the same command. Exit codes: 0 done, 1 error,
2 wrong command line.

## Downloads and checks

1. The installer reads the release from the GitHub API:
   `https://api.github.com/repos/xls/pandite/releases`.
2. It downloads one release file for your system and `SHA256SUMS.txt` of
   the same release. It uses HTTPS only and follows redirects to HTTPS
   addresses only.
3. It compares the SHA-256 of the file with `SHA256SUMS.txt`. When the
   SHA-256 differs, or `SHA256SUMS.txt` does not list the file, it deletes
   the file and installs nothing.

The requests carry the User-Agent `pandite-installer/<version>` and no
token. The installer sends no other data.

## Systems and install locations

| System | Release file | Install |
|---|---|---|
| Windows x64, ARM64 | `pandite-<version>-<arch>-windows-setup.exe` | Runs the setup program silently for the current user, without administrator rights: `%LOCALAPPDATA%\Programs\Pandite`, Start menu, image file associations. |
| macOS 11 or later | `pandite-<version>-universal-apple-darwin.dmg` (the `.zip` in releases up to 0.2.2) | Copies `Pandite.app` to `~/Applications`, or to `/Applications` with `--system`. Replaces an older copy. |
| Linux x64, glibc 2.35 or later | `pandite-<version>-x86_64-unknown-linux-gnu.tar.gz` | Runs the `install.sh` of the tarball with `PREFIX=~/.local`: `~/.local/bin/pandite`, desktop entry, icons. |

Other systems get a "not supported" message.

The installer keeps the installed version in `installed.json` in its
per-user folder:

- Windows: `%LOCALAPPDATA%\pandite-installer`
- macOS: `~/Library/Application Support/pandite-installer`
- Linux: `$XDG_DATA_HOME/pandite-installer` or
  `~/.local/share/pandite-installer`. The folder also keeps `install.sh`
  for the uninstall.

On Windows, the uninstall runs the uninstaller that the setup program
registered. On Linux, it runs the kept `install.sh --uninstall`. On macOS,
it deletes the app.

## Notes

- npx keeps a copy of the installer. To use the newest installer, run
  `npx pandite@latest`.
- `npm install -g pandite` adds a `pandite` command that is this installer,
  not the image viewer. Use npx.

License: MIT.
