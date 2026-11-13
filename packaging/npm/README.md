# Pandite for npx

Run [Pandite](https://github.com/xls/pandite), a fast offline image viewer,
with one command:

```sh
npx pandite photo.jpg
```

This package holds a small launcher, not the viewer. On the first run, the
launcher downloads the Pandite release file for your system from the GitHub
release of the same version. It checks the SHA-256 of the file against the
checksums in this package, unpacks it into a cache folder and starts it.
Later runs start Pandite from the cache. The package has no dependencies and
no install script.

| System | Release file |
|---|---|
| Windows x64, ARM64 | `pandite-<version>-<target>.zip` |
| macOS 11 or later | `pandite-<version>-universal-apple-darwin.zip` |
| Linux x64, glibc 2.35 or later | `pandite-<version>-x86_64-unknown-linux-gnu.tar.gz` |

The launcher passes every argument to Pandite and returns its exit code. On
Windows, Pandite stays in the background after you close its window. For
this reason, a run that only opens images returns at once. A run with an
option, such as `--version`, waits for the exit code.

Cache folder:

- Windows: `%LOCALAPPDATA%\pandite\npm`
- macOS: `~/Library/Caches/pandite/npm`
- Linux: `$XDG_CACHE_HOME/pandite/npm` or `~/.cache/pandite/npm`

To use a different folder, set `PANDITE_NPM_CACHE`. To remove Pandite,
delete the cache folder.

The launcher downloads over HTTPS only and refuses a redirect to any other
scheme. It refuses a file whose SHA-256 differs from the checksum in this
package.

License: MIT.
