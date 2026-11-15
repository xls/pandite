# Artwork

| File | Use |
|---|---|
| `../../crates/pandite-app/assets/icon-1024.png` | The icon master: the tile on a transparent background. Every icon file comes from it. |
| `pandite-borderless.png` | The mark on a full square background, without the tile. |
| `github-social-preview.png` | The image for GitHub, Settings, Social preview (1280 x 640). |

To make every icon file again after a change of the master, run:

```sh
cargo run -p pandite-app --example make_icon -- <dir> 256
```

Copy `pandite.ico`, `icon-256.png`, `icon-32.rgba` and `icon-64.rgba` from
`<dir>` to `crates/pandite-app/assets/`. Then run:

```sh
node packaging/make-assets.mjs
```
