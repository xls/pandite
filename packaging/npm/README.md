# Pandite installer

Installs [Pandite](https://github.com/xls/pandite), a fast offline image
viewer, from its GitHub releases. Needs Node.js 18 or later. No
dependencies, no install scripts.

```sh
npx pandite
```

## Commands

Install the latest release of Pandite. The installer reads the releases on
GitHub each time, so this command always gets the newest one, and it
updates an older Pandite:

```sh
npx pandite
```

Install one named release. `0.2.0` and `v0.2.0` are equal:

```sh
npx pandite install -v 0.2.0
```

Install the latest release only when it is newer than the installed one.
When Pandite is not installed, this installs it:

```sh
npx pandite update
```

Remove Pandite. `--uninstall` is the same:

```sh
npx pandite uninstall
```

Print the release versions, newest first:

```sh
npx pandite --list
```

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
| macOS 11 or later | `pandite-<version>-universal-macos.dmg` (the `.zip` in releases up to 0.2.2) | Copies `Pandite.app` to `~/Applications`, or to `/Applications` with `--system`. Replaces an older copy. |
| Linux x64, glibc 2.35 or later | `pandite-<version>-x86_64-linux.tar.gz` | Runs the `install.sh` of the tarball with `PREFIX=~/.local`: `~/.local/bin/pandite`, desktop entry, icons. |

Other systems get a "not supported" message.

## Linux: default viewer and Hyprland

On a first install in a terminal, the installer asks two questions. The
answer `y` or `yes` does the step. Any other answer changes nothing.

| Question | Asked when | Step |
|---|---|---|
| Make Pandite the default viewer? | `xdg-mime` is in `PATH` | Runs `xdg-mime default pandite.desktop` with every MIME type of the desktop entry. |
| Open Pandite as a floating window in Hyprland? | `~/.config/hypr/hyprland.lua` exists | Adds the line `o.window("pandite", { tag = "+floating-window" })` at the end of that file. The line is added one time only. |

The installer asks nothing on an update and nothing without a terminal.

| Option | Effect |
|---|---|
| `--default` | Does the first step without a question. |
| `--no-default` | Does not ask the first question. |
| `--float` | Does the second step without a question. |
| `--no-float` | Does not ask the second question. |

To do the steps for a Pandite that is installed, run:

```sh
npx pandite --default --float
```

The installer keeps the earlier default program of each format.
`npx pandite uninstall` gives each format that still opens with Pandite
back to that program, and removes the line that it added to the Hyprland
file. A format that you gave to another program stays with that program.

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

- npx keeps a copy of the installer. The kept copy still installs the
  latest Pandite. To use the newest installer also, run:

  ```sh
  npx pandite@latest
  ```
- `npm install -g pandite` adds a `pandite` command that is this installer,
  not the image viewer. Use npx.

License: MIT.
