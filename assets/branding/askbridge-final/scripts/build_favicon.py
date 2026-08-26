"""
Produce the favicon kit that a docs site (or a future website) can drop into
`/` and `/apple-touch-icon` paths without further work.

Layout follows the modern web favicon recipe:
  - favicon.ico  (16 + 32 + 48, classic)
  - favicon-16x16.png / favicon-32x32.png  (modern browsers + bookmark bar)
  - apple-touch-icon.png  (180x180, iOS home screen)
  - android-chrome-192x192.png / android-chrome-512x512.png  (PWA / Android)
  - site.webmanifest  (PWA manifest referencing the above)
  - browserconfig.xml  (legacy IE/Edge tile color)
  - svg (optional modern reference)
"""
from __future__ import annotations

import json
import struct
from io import BytesIO
from pathlib import Path

from PIL import Image

ROOT = Path(r"D:\askbridge\assets\branding\askbridge-final")
ICONS = ROOT / "icons"
FAV = ROOT / "favicon"

# A neutral theme color so PWA installs blend with the warm cream background.
THEME = "#FAEEDA"
BG = "#FAEEDA"


def copy_png(name: str, size: int) -> Path:
    src = ICONS / f"askbridge-cream-{size}.png"
    dst = FAV / name
    Image.open(src).save(dst, format="PNG", optimize=True)
    return dst


def build_favicon_ico() -> Path:
    """Three-size ICO (16/32/48) for legacy browsers."""
    sizes = [16, 32, 48]
    images: list[tuple[int, bytes]] = []
    for s in sizes:
        src = ICONS / f"askbridge-cream-{s}.png"
        img = Image.open(src).convert("RGBA")
        buf = BytesIO()
        img.save(buf, format="PNG", optimize=True)
        images.append((s, buf.getvalue()))

    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    payloads = b""
    for s, png in images:
        w = s if s < 256 else 0
        h = s if s < 256 else 0
        entries += struct.pack(
            "<BBBBHHII", w, h, 0, 0, 1, 32, len(png), offset
        )
        payloads += png
        offset += len(png)

    out = FAV / "favicon.ico"
    out.write_bytes(header + entries + payloads)
    return out


def build_webmanifest() -> Path:
    manifest = {
        "name": "AskBridge",
        "short_name": "AskBridge",
        "description": "Screenshot → AI bridge for Windows",
        "icons": [
            {
                "src": "/android-chrome-192x192.png",
                "sizes": "192x192",
                "type": "image/png",
            },
            {
                "src": "/android-chrome-512x512.png",
                "sizes": "512x512",
                "type": "image/png",
            },
        ],
        "theme_color": THEME,
        "background_color": BG,
        "display": "standalone",
    }
    out = FAV / "site.webmanifest"
    out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return out


def build_browserconfig() -> Path:
    xml = (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<browserconfig>\n'
        '  <msapplication>\n'
        '    <tile>\n'
        f'      <tileColor>{THEME}</tileColor>\n'
        '    </tile>\n'
        '  </msapplication>\n'
        '</browserconfig>\n'
    )
    out = FAV / "browserconfig.xml"
    out.write_text(xml, encoding="utf-8")
    return out


def main() -> None:
    FAV.mkdir(parents=True, exist_ok=True)

    targets = [
        copy_png("favicon-16x16.png", 16),
        copy_png("favicon-32x32.png", 32),
        copy_png("apple-touch-icon.png", 180),
        copy_png("android-chrome-192x192.png", 192),
        copy_png("android-chrome-512x512.png", 512),
    ]
    ico = build_favicon_ico()
    manifest = build_webmanifest()
    bc = build_browserconfig()

    print("favicon kit:")
    for t in [*targets, ico, manifest, bc]:
        print(f"  {t.relative_to(ROOT)} ({t.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
