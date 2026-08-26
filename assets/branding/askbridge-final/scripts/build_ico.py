"""
Build the Windows multi-resolution .ico used by askbridge.exe and askbridge-tray.

We pack six standard Windows icon sizes (16/24/32/48/64/128/256) into one .ico so
the OS can pick the right one for the tray, the taskbar, Alt-Tab, and the
installer's title bar without re-rasterization.

ICO container is little-endian: 6-byte header + 16-byte directory entries per
image + raw PNG payloads (ICO supports embedded PNG since Vista).
"""
from __future__ import annotations

import struct
from pathlib import Path

from PIL import Image

ROOT = Path(r"D:\askbridge\assets\branding\askbridge-final")
ICONS = ROOT / "icons"
OUT = ROOT / "icons" / "askbridge.ico"

# Standard Windows icon sizes — Vista+ supports 256 directly. We skip 24 since
# Windows picks 24 by nearest-neighbor from 32 in most cases; including 48
# covers the rare "Details" tile view.
SIZES = [16, 32, 48, 64, 128, 256]


def main() -> None:
    images: list[tuple[int, Image.Image, bytes]] = []
    from io import BytesIO
    for size in SIZES:
        src = ICONS / f"askbridge-transparent-{size}.png"
        if not src.exists():
            raise FileNotFoundError(src)
        img = Image.open(src).convert("RGBA")
        # Re-encode PNG to guarantee a tightly-packed payload for the ICO container
        buf = BytesIO()
        img.save(buf, format="PNG", optimize=True)
        images.append((size, img, buf.getvalue()))

    # 6-byte ICONDIR header: reserved=0, type=1 (icon), count
    header = struct.pack("<HHH", 0, 1, len(images))

    # 16-byte ICONDIRENTRY per image
    offset = 6 + 16 * len(images)
    entries = b""
    payloads = b""
    for size, _img, png in images:
        width = size if size < 256 else 0  # 0 means 256 in ICO
        height = size if size < 256 else 0
        entry = struct.pack(
            "<BBBBHHII",
            width,        # width
            height,       # height
            0,            # color palette count (0 for true-color)
            0,            # reserved
            1,            # color planes
            32,           # bits per pixel
            len(png),     # size of image data
            offset,       # offset from start of file
        )
        entries += entry
        payloads += png
        offset += len(png)

    OUT.write_bytes(header + entries + payloads)
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes, {len(SIZES)} sizes)")


if __name__ == "__main__":
    main()
