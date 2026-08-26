"""
Build two review sheets so the user can audit the kit before publication.

  * contact-sheet.png            — one tile per variant at the largest size,
                                   plus the two lockups and a dark-background
                                   sanity check for the reverse mark.
  * favicon-readability-sheet.png — the four variants × {64, 32, 16, 16 on
                                   dark} to verify 16 px legibility.
"""
from __future__ import annotations

from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(r"D:\askbridge\assets\branding\askbridge-final")
ICONS = ROOT / "icons"
WEB = ROOT / "web"
DOCS = ROOT / "docs"

# 14px sans fallback chain — Pillow uses system fonts; we try a few common names.
FONT_CANDIDATES = [
    "C:/Windows/Fonts/segoeuib.ttf",   # Segoe UI Bold
    "C:/Windows/Fonts/segoeui.ttf",    # Segoe UI
    "C:/Windows/Fonts/arialbd.ttf",    # Arial Bold
    "C:/Windows/Fonts/arial.ttf",      # Arial
]


def load_font(size: int, bold: bool = False) -> ImageFont.FreeTypeFont:
    paths = FONT_CANDIDATES if not bold else [FONT_CANDIDATES[0], FONT_CANDIDATES[2], *FONT_CANDIDATES]
    for p in paths:
        try:
            return ImageFont.truetype(p, size)
        except OSError:
            continue
    return ImageFont.load_default()


CREAM = (250, 238, 218)
INK = (44, 44, 42)
ACCENT = (216, 90, 48)
MUTED = (95, 94, 90)
LINE = (211, 209, 199)
DARK = (44, 44, 42)


def tile(img: Image.Image, size: int) -> Image.Image:
    img = img.convert("RGBA")
    img.thumbnail((size, size), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), CREAM + (255,))
    canvas.paste(img, ((size - img.width) // 2, (size - img.height) // 2), img)
    return canvas


def text_label(draw: ImageDraw.ImageDraw, xy, text, fill=INK, size=18, bold=False):
    font = load_font(size, bold=bold)
    draw.text(xy, text, font=font, fill=fill)


def build_contact_sheet() -> Path:
    cols, rows = 4, 3
    tile_size = 240
    padding = 32
    label_h = 36
    title_h = 60
    width = cols * tile_size + (cols + 1) * padding
    height = title_h + rows * (tile_size + label_h + padding) + padding
    canvas = Image.new("RGB", (width, height), CREAM)
    draw = ImageDraw.Draw(canvas)

    text_label(draw, (padding, 16), "AskBridge — brand kit overview", INK, 24, bold=True)
    text_label(draw, (padding, 44), "Each tile: 240 px render · all artwork is original geometry", MUTED, 14)

    variants = [
        ("mark · cream",         "icons/askbridge-cream-256.png"),
        ("mark · transparent",   "icons/askbridge-transparent-256.png"),
        ("mark · monochrome",    "icons/askbridge-mono-256.png"),
        ("mark · reverse (dark)","icons/askbridge-reverse-256.png"),
    ]
    extras = [
        ("horizontal lockup",    "web/askbridge-lockup-horizontal.png",  None, None),
        ("stacked lockup",       "web/askbridge-lockup-stacked.png",    None, None),
        ("GitHub social card",   "github/askbridge-social-card.png",    None, None),
        ("README header",        "github/askbridge-readme-header.png",  None, None),
    ]

    for i, (name, path) in enumerate(variants):
        x = padding + (i % cols) * (tile_size + padding)
        y = title_h + (i // cols) * (tile_size + label_h + padding)
        img = Image.open(ROOT / path)
        tile_img = tile(img, tile_size)
        canvas.paste(tile_img, (x, y), tile_img)
        text_label(draw, (x, y + tile_size + 6), name, INK, 14, bold=True)

    inner_w = tile_size - 24
    inner_h = tile_size - 24
    for j, (name, path, _w, _h) in enumerate(extras):
        idx = 4 + j
        x = padding + (idx % cols) * (tile_size + padding)
        y = title_h + (idx // cols) * (tile_size + label_h + padding)
        img = Image.open(ROOT / path).convert("RGBA")
        img.thumbnail((inner_w, inner_h), Image.LANCZOS)
        box = Image.new("RGBA", (inner_w, inner_h), CREAM + (255,))
        box.paste(img, ((inner_w - img.width) // 2, (inner_h - img.height) // 2), img)
        canvas.paste(box, (x + 12, y + 12), box)
        text_label(draw, (x, y + tile_size + 6), name, INK, 14, bold=True)

    out = DOCS / "contact-sheet.png"
    DOCS.mkdir(parents=True, exist_ok=True)
    canvas.save(out, format="PNG", optimize=True)
    return out


def build_favicon_readability() -> Path:
    variants = [
        ("cream",       "icons/askbridge-cream-{}.png",       CREAM, INK),
        ("transparent", "icons/askbridge-transparent-{}.png", (255, 255, 255), INK),
        ("mono",        "icons/askbridge-mono-{}.png",        (255, 255, 255), INK),
        ("reverse",     "icons/askbridge-reverse-{}.png",     DARK, CREAM),
    ]
    sizes = [64, 32, 16, 16]
    size_labels = ["64 px", "32 px", "16 px", "16 px on dark"]
    rows = len(variants) + 1
    cols = len(sizes) + 1
    cell_w, cell_h = 140, 100
    pad = 24
    header_h = 80
    width = cell_w * cols + pad * (cols + 1)
    height = header_h + rows * cell_h + pad * (rows + 1)
    canvas = Image.new("RGB", (width, height), CREAM)
    draw = ImageDraw.Draw(canvas)

    text_label(draw, (pad, 16), "AskBridge — small-size readability", INK, 22, bold=True)
    text_label(draw, (pad, 46), "Read from the 16 px columns. Anything illegible here is unusable as a favicon.", MUTED, 13)

    # Column headers
    for c, label in enumerate(["variant"] + size_labels):
        x = pad + c * (cell_w + pad)
        y = header_h
        text_label(draw, (x + 8, y + 8), label, INK, 14, bold=True)

    for r, (name, fmt, bg, fg) in enumerate(variants):
        y = header_h + pad + (r + 1) * (cell_h + pad) - pad
        # Row label
        text_label(draw, (pad + 8, y + cell_h // 2 - 8), name, INK, 14, bold=True)
        for c, (size, label) in enumerate(zip(sizes, size_labels)):
            x = pad + (c + 1) * (cell_w + pad)
            cell_box = Image.new("RGB", (cell_w, cell_h), bg)
            cell_draw = ImageDraw.Draw(cell_box)
            cell_draw.rectangle([(0, 0), (cell_w - 1, cell_h - 1)], outline=LINE, width=1)
            png = Image.open(ROOT / fmt.format(size))
            tile_img = tile(png, cell_h - 24)
            cell_box.paste(tile_img, ((cell_w - tile_img.width) // 2, (cell_h - tile_img.height) // 2), tile_img)
            canvas.paste(cell_box, (x, y))
            text_label(cell_draw, (6, 4), label, MUTED, 11)

    out = DOCS / "favicon-readability-sheet.png"
    canvas.save(out, format="PNG", optimize=True)
    return out


def main() -> None:
    DOCS.mkdir(parents=True, exist_ok=True)
    a = build_contact_sheet()
    b = build_favicon_readability()
    print(f"contact-sheet:        {a} ({a.stat().st_size} bytes)")
    print(f"readability-sheet:    {b} ({b.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
