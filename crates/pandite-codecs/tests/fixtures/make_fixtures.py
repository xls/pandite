"""Regenerate the committed binary fixtures from synthetic patterns.

Run from this directory: python -I make_fixtures.py
Needs Pillow 12 (with libwebp and libavif) and ffmpeg 8 with libjxl.
Every image is drawn here from solid color blocks; no third-party content.
"""
import os
import subprocess

from PIL import Image


def blocks(size, cells):
    """An RGBA image filled with rectangles: cells = [((x0, y0, x1, y1), rgba)]."""
    img = Image.new("RGBA", size, (0, 0, 0, 255))
    for (x0, y0, x1, y1), rgba in cells:
        for y in range(y0, y1):
            for x in range(x0, x1):
                img.putpixel((x, y), rgba)
    return img


def lossy_webp():
    img = blocks((64, 32), [((0, 0, 32, 32), (200, 40, 40, 255)), ((32, 0, 64, 32), (30, 90, 210, 255))])
    img.convert("RGB").save("lossy.webp", "WEBP", quality=90, method=6)


def lossy_alpha_webp():
    img = blocks((32, 32), [((0, 0, 32, 16), (40, 180, 60, 255)), ((0, 16, 32, 32), (40, 180, 60, 128))])
    img.save("lossy_alpha.webp", "WEBP", quality=90, alpha_quality=100, method=6)


def lossy_anim_webp():
    a = blocks((32, 32), [((0, 0, 32, 32), (220, 30, 30, 255))])
    b = blocks((32, 32), [((0, 0, 32, 32), (220, 30, 30, 255)), ((16, 16, 32, 32), (20, 40, 230, 255))])
    a.save("lossy_anim.webp", "WEBP", save_all=True, append_images=[b], duration=[80, 120],
           loop=0, quality=90, method=6, minimize_size=False, allow_mixed=False, kmin=0, kmax=0)


def avif():
    img = blocks((64, 64), [((0, 0, 32, 32), (230, 30, 30, 255)), ((32, 0, 64, 32), (30, 200, 40, 255)),
                            ((0, 32, 32, 64), (30, 40, 220, 255)), ((32, 32, 64, 64), (240, 240, 240, 255))])
    img.convert("RGB").save("blocks.avif", "AVIF", quality=90, subsampling="4:4:4")
    alpha = blocks((32, 32), [((0, 0, 32, 16), (250, 200, 20, 255)), ((0, 16, 32, 32), (250, 200, 20, 64))])
    alpha.save("alpha.avif", "AVIF", quality=90, subsampling="4:4:4")


def rotated_avif():
    """64 x 32, left red, right blue, stored with irot from EXIF orientation 6."""
    img = blocks((64, 32), [((0, 0, 32, 32), (230, 30, 30, 255)), ((32, 0, 64, 32), (30, 40, 220, 255))])
    exif = Image.Exif()
    exif[0x0112] = 6
    img.convert("RGB").save("rotated.avif", "AVIF", quality=90, subsampling="4:4:4", exif=exif.tobytes())


def deep_avif():
    """10-bit 4:2:0 through libaom."""
    img = blocks((64, 64), [((0, 0, 32, 32), (230, 30, 30, 255)), ((32, 0, 64, 32), (30, 200, 40, 255)),
                            ((0, 32, 32, 64), (30, 40, 220, 255)), ((32, 32, 64, 64), (240, 240, 240, 255))])
    img.convert("RGB").save("tmp_blocks.png")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", "tmp_blocks.png", "-c:v", "libaom-av1",
                    "-pix_fmt", "yuv420p10le", "-still-picture", "1", "-crf", "8", "-frames:v", "1",
                    "deep.avif"], check=True)


def jpeg_tiff():
    """JPEG-compressed (YCbCr) TIFF through libtiff."""
    img = blocks((64, 32), [((0, 0, 32, 32), (230, 30, 30, 255)), ((32, 0, 64, 32), (30, 40, 220, 255))])
    img.convert("RGB").save("jpeg.tif", compression="jpeg", quality=95)


def more_jxl():
    """Lossy (XYB) still and a two-frame animation through libjxl."""
    img = blocks((40, 24), [((0, 0, 20, 24), (230, 30, 30, 255)), ((20, 0, 40, 24), (30, 40, 220, 255))])
    img.convert("RGB").save("tmp_lossy.png")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", "tmp_lossy.png", "-c:v", "libjxl",
                    "-distance", "1", "lossy.jxl"], check=True)
    a = blocks((16, 16), [((0, 0, 16, 16), (220, 30, 30, 255))])
    b = blocks((16, 16), [((0, 0, 16, 16), (20, 40, 230, 255))])
    a.convert("RGB").save("tmp_f0.png")
    b.convert("RGB").save("tmp_f1.png")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", "5", "-i", "tmp_f%d.png",
                    "-c:v", "libjxl_anim", "-distance", "0", "-f", "image2pipe", "anim.jxl"], check=True)


def _boxes(data):
    """Yield (type, body) of the ISO BMFF boxes in data."""
    pos = 0
    while pos + 8 <= len(data):
        size = int.from_bytes(data[pos:pos + 4], "big")
        kind = data[pos + 4:pos + 8]
        header = 8
        if size == 1:
            size = int.from_bytes(data[pos + 8:pos + 16], "big")
            header = 16
        elif size == 0:
            size = len(data) - pos
        yield kind, data[pos + header:pos + size]
        pos += size


def _box(kind, body, full=None):
    if full is not None:
        body = full.to_bytes(4, "big") + body
    return (8 + len(body)).to_bytes(4, "big") + kind + body


def _tile(color):
    """A 32 x 32 AVIF tile: its AV1 data and its av1C property box."""
    img = Image.new("RGB", (32, 32), color)
    img.save("tmp_tile.avif", "AVIF", quality=90, subsampling="4:4:4")
    data = open("tmp_tile.avif", "rb").read()
    meta = next(b for k, b in _boxes(data) if k == b"meta")[4:]
    props = next(b for k, b in _boxes(meta) if k == b"iprp")
    ipco = next(b for k, b in _boxes(props) if k == b"ipco")
    av1c = next(_box(k, b) for k, b in _boxes(ipco) if k == b"av1C")
    iloc = next(b for k, b in _boxes(meta) if k == b"iloc")
    # iloc version 0: full box header (4), field sizes (2), item count (2),
    # item ID (2), data reference (2), extent count (2), offset (4),
    # length (4). Offsets and lengths are 4 bytes, no base offset.
    assert iloc[0] == 0 and iloc[4:6] == bytes([0x44, 0x00]), "unexpected iloc layout"
    off = int.from_bytes(iloc[14:18], "big")
    length = int.from_bytes(iloc[18:22], "big")
    return data[off:off + length], av1c


def grid_avif():
    """A 2 x 1 grid of 32 x 32 tiles (red, blue) cropped to 60 x 32."""
    red, av1c = _tile((230, 30, 30))
    blue, _ = _tile((30, 40, 220))
    grid = bytes([0, 0, 0, 1]) + (60).to_bytes(2, "big") + (32).to_bytes(2, "big")
    ftyp = _box(b"ftyp", b"avif" + bytes(4) + b"mif1avifmiaf")
    hdlr = _box(b"hdlr", bytes(4) + b"pict" + bytes(12) + bytes(1), full=0)
    pitm = _box(b"pitm", (1).to_bytes(2, "big"), full=0)
    infe = lambda i, t: _box(b"infe", i.to_bytes(2, "big") + bytes(2) + t + bytes(1), full=2 << 24)
    iinf = _box(b"iinf", (3).to_bytes(2, "big") + infe(1, b"grid") + infe(2, b"av01") + infe(3, b"av01"), full=0)
    iref = _box(b"iref", _box(b"dimg", (1).to_bytes(2, "big") + (2).to_bytes(2, "big")
                              + (2).to_bytes(2, "big") + (3).to_bytes(2, "big")), full=0)
    ispe = lambda w, h: _box(b"ispe", w.to_bytes(4, "big") + h.to_bytes(4, "big"), full=0)
    ipco = _box(b"ipco", ispe(60, 32) + ispe(32, 32) + av1c)
    ipma_body = (3).to_bytes(4, "big")
    ipma_body += (1).to_bytes(2, "big") + bytes([1, 1])
    for item in (2, 3):
        ipma_body += item.to_bytes(2, "big") + bytes([2, 2, 0x80 | 3])
    iprp = _box(b"iprp", ipco + _box(b"ipma", ipma_body, full=0))
    idat = _box(b"idat", grid)

    def iloc(mdat_start):
        body = bytes([0x44, 0x00]) + (3).to_bytes(2, "big")
        # Item 1 (grid descriptor): construction method 1 (idat), one extent.
        body += (1).to_bytes(2, "big") + (1).to_bytes(2, "big") + (0).to_bytes(2, "big")
        body += (1).to_bytes(2, "big") + (0).to_bytes(4, "big") + len(grid).to_bytes(4, "big")
        for item, data, start in ((2, red, mdat_start), (3, blue, mdat_start + len(red))):
            body += item.to_bytes(2, "big") + (0).to_bytes(2, "big") + (0).to_bytes(2, "big")
            body += (1).to_bytes(2, "big") + start.to_bytes(4, "big") + len(data).to_bytes(4, "big")
        return _box(b"iloc", body, full=1 << 24)

    def build(mdat_start):
        meta = _box(b"meta", hdlr + pitm + iloc(mdat_start) + iinf + iref + iprp + idat, full=0)
        return ftyp + meta

    head = build(0)
    head = build(len(head) + 8)
    mdat = _box(b"mdat", red + blue)
    open("grid.avif", "wb").write(head + mdat)


def jxl():
    img = blocks((40, 24), [((0, 0, 20, 24), (230, 30, 30, 255)), ((20, 0, 40, 24), (30, 40, 220, 128))])
    img.save("blocks.png")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", "blocks.png", "-c:v", "libjxl",
                    "-distance", "0", "-effort", "7", "blocks.jxl"], check=True)


if __name__ == "__main__":
    lossy_webp()
    lossy_alpha_webp()
    lossy_anim_webp()
    avif()
    rotated_avif()
    deep_avif()
    grid_avif()
    jpeg_tiff()
    jxl()
    more_jxl()
    for name in os.listdir("."):
        if name.startswith("tmp_") or name == "blocks.png":
            os.remove(name)
