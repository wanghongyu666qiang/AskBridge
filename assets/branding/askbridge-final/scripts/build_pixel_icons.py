"""
Hand-tuned pixel-snapped renders of the AskBridge mark for the smallest icon sizes.

The vector source renders poorly below ~32 px because the bracket stroke and the
bridge walls land on fractional pixels after scaling. This script rebuilds the
same geometry on exact pixel grids (16 / 20 / 24 px) with integer coordinates:

  - bracket stroke: 1 px (16/20) or 2 px (24)
  - bridge walls:   2 px everywhere
  - straight edges land on the pixel grid; only the arch curves keep mild AA

Run after render_assets.py whenever the source SVG changes; the outputs overwrite
icons/askbridge-cream-{16,20,24}.png. build_ico.py packs them into the .ico.
"""
from __future__ import annotations

import io
import resvg_py
from PIL import Image
from pathlib import Path

ROOT = Path(r"D:\askbridge\assets\branding\askbridge-final")
OUT = ROOT / "icons"

CREAM = "#FAEEDA"
CHARCOAL = "#2C2C2A"
TERRACOTTA = "#D85A30"


def bracket_rects(arm: int, thickness: int, edge: int, canvas: int) -> str:
    """Four corner L-brackets. `arm` = length incl. corner, `edge` = inset."""
    inner = edge + thickness
    far = canvas - edge
    far_arm = far - arm
    far_inner = far - thickness
    return "\n".join(
        (
            # top-left
            f'  <rect x="{edge}" y="{edge}" width="{arm}" height="{thickness}"/>',
            f'  <rect x="{edge}" y="{edge}" width="{thickness}" height="{arm}"/>',
            # top-right
            f'  <rect x="{far_arm}" y="{edge}" width="{arm}" height="{thickness}"/>',
            f'  <rect x="{far_inner}" y="{edge}" width="{thickness}" height="{arm}"/>',
            # bottom-left
            f'  <rect x="{edge}" y="{far_inner}" width="{arm}" height="{thickness}"/>',
            f'  <rect x="{edge}" y="{far_arm}" width="{thickness}" height="{arm}"/>',
            # bottom-right
            f'  <rect x="{far_arm}" y="{far_inner}" width="{arm}" height="{thickness}"/>',
            f'  <rect x="{far_inner}" y="{far_arm}" width="{thickness}" height="{arm}"/>',
        )
    )


def arch_path(left: int, right: int, top: int, bottom: int, wall: int) -> str:
    """Horseshoe arch: outer semicircle + straight legs, hollowed by `wall`."""
    mid = (left + right) // 2
    outer_radius = (right - left) // 2
    springline = top + outer_radius
    inner_left = left + wall
    inner_right = right - wall
    inner_radius = (inner_right - inner_left) // 2
    return (
        f"M {left} {bottom} L {left} {springline} "
        f"A {outer_radius} {outer_radius} 0 0 1 {right} {springline} "
        f"L {right} {bottom} L {inner_right} {bottom} L {inner_right} {springline} "
        f"A {inner_radius} {inner_radius} 0 0 0 {inner_left} {springline} "
        f"L {inner_left} {bottom} Z"
    )


# size -> (bracket arm, bracket thickness, bracket inset, arch left/right/top/bottom, wall)
LAYOUTS: dict[int, tuple[int, int, int, tuple[int, int, int, int], int]] = {
    16: (4, 1, 1, (4, 12, 4, 12), 2),
    20: (5, 1, 1, (6, 14, 5, 15), 2),
    24: (6, 2, 1, (7, 17, 6, 18), 2),
}


def build_svg(size: int) -> str:
    arm, thickness, inset, (left, right, top, bottom), wall = LAYOUTS[size]
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {size} {size}">\n'
        f'  <rect width="{size}" height="{size}" fill="{CREAM}"/>\n'
        f'  <g fill="{CHARCOAL}">\n'
        f"{bracket_rects(arm, thickness, inset, size)}\n"
        f"  </g>\n"
        f'  <path d="{arch_path(left, right, top, bottom, wall)}" fill="{TERRACOTTA}"/>\n'
        f"</svg>\n"
    )


def main() -> None:
    for size in sorted(LAYOUTS):
        svg = build_svg(size)
        png = bytes(resvg_py.svg_to_bytes(svg_string=svg, width=size, height=size))
        target = OUT / f"askbridge-cream-{size}.png"
        Image.open(io.BytesIO(png)).convert("RGB").save(target, format="PNG", optimize=True)
        print(f"wrote {target.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
