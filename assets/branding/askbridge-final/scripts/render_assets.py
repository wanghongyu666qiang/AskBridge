"""
Render AskBridge source SVGs into the full PNG / ICO / favicon / GitHub asset pipeline.

Single source of truth: assets/branding/askbridge-final/source/askbridge-mark-transparent.svg
plus three styled variants (cream, mono, reverse) and the two lockup compositions.

Run from the repo root or anywhere — paths are absolute.
"""
from __future__ import annotations

import io
import os
import resvg_py
from PIL import Image
from pathlib import Path

ROOT = Path(r"D:\askbridge\assets\branding\askbridge-final")
SRC = ROOT / "source"

CREAM_VARIANT = {
    "name": "cream",
    "svg": SRC / "askbridge-mark.svg",  # has cream background
    "transparent": False,
    "out": ROOT / "icons",
}

TRANSPARENT_VARIANT = {
    "name": "transparent",
    "svg": SRC / "askbridge-mark-transparent.svg",
    "transparent": True,
    "out": ROOT / "icons",
}

MONO_VARIANT = {
    "name": "mono",
    "svg": SRC / "askbridge-mark-mono.svg",
    "transparent": True,
    "out": ROOT / "icons",
}

REVERSE_VARIANT = {
    "name": "reverse",
    "svg": SRC / "askbridge-mark-reverse.svg",
    "transparent": False,
    "out": ROOT / "icons",
}

LOCKUP_HORIZONTAL = {
    "name": "lockup-horizontal",
    "svg": SRC / "askbridge-lockup-horizontal.svg",
    "transparent": False,
    "out": ROOT / "web",
    "w": 1440,
    "h": 480,
}

LOCKUP_STACKED = {
    "name": "lockup-stacked",
    "svg": SRC / "askbridge-lockup-stacked.svg",
    "transparent": False,
    "out": ROOT / "web",
    "w": 1024,
    "h": 1280,
}

# Square mark sizes for the PNG kit
SQUARE_SIZES = [1024, 512, 256, 192, 180, 128, 96, 64, 48, 32, 16]


def render_svg(svg_path: Path, width: int, height: int) -> bytes:
    text = svg_path.read_text(encoding="utf-8")
    return bytes(resvg_py.svg_to_bytes(svg_string=text, width=width, height=height))


def render_mark_pngs(variant: dict) -> list[Path]:
    out_dir: Path = variant["out"]
    out_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for size in SQUARE_SIZES:
        png = render_svg(variant["svg"], size, size)
        img = Image.open(io.BytesIO(png))
        target = out_dir / f"askbridge-{variant['name']}-{size}.png"
        # Preserve alpha for transparent variants
        if variant["transparent"]:
            img.save(target, format="PNG", optimize=True)
        else:
            # Convert to RGB so the file is smaller for favicon-style use
            img.convert("RGB").save(target, format="PNG", optimize=True)
        written.append(target)
    return written


def render_lockup(spec: dict) -> Path:
    out_dir: Path = spec["out"]
    out_dir.mkdir(parents=True, exist_ok=True)
    png = render_svg(spec["svg"], spec["w"], spec["h"])
    target = out_dir / f"askbridge-{spec['name']}.png"
    Image.open(io.BytesIO(png)).convert("RGB").save(target, format="PNG", optimize=True)
    return target


def main() -> None:
    print("[1/6] rendering cream + transparent + mono + reverse mark PNGs")
    for variant in (CREAM_VARIANT, TRANSPARENT_VARIANT, MONO_VARIANT, REVERSE_VARIANT):
        files = render_mark_pngs(variant)
        print(f"  {variant['name']:>12}: {len(files)} sizes -> {variant['out']}")

    print("[2/6] rendering lockups")
    for spec in (LOCKUP_HORIZONTAL, LOCKUP_STACKED):
        target = render_lockup(spec)
        print(f"  {spec['name']:>18}: {target.name} ({target.stat().st_size} bytes)")

    print("done.")


if __name__ == "__main__":
    main()
