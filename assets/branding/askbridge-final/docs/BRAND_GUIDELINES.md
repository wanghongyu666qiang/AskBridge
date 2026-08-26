# AskBridge · Brand Guidelines

> This file documents the official AskBridge mark, its colors, fonts, and usage
> rules. It is the source of truth for anyone publishing the project, building
> a website, or producing printed material. **Do not edit the SVG sources
> casually** — every variant in `source/` is rendered into the rest of the kit
> by the scripts under `scripts/`.

---

## 1. The mark

The mark is **a black four-corner screen-selection frame with a terracotta-red
bridge centered inside, on a warm cream canvas**.

| Element | Meaning |
|---|---|
| Four black corner brackets | A screenshot selection rectangle — the primary action of the tool. |
| Terracotta-red bridge | The literal *bridge* in AskBridge: the path that carries the screenshot to the AI surface. |
| Warm cream canvas | A friendly, paper-like background that signals "practical utility, not a flashy AI brand". |

The metaphor is **one symbol, one job**: the four corners say *capture*; the
bridge says *bridge*. There is no chat bubble, no sparkle, no robot, no glow,
no gradient — none of the visual vocabulary that has come to mean "AI" in
2024-2026 product design. AskBridge is the thing that *prepares* the question;
the AI surface itself is not the brand.

### Construction

The mark is built on a 1024 × 1024 grid. The artwork is original geometry; no
reference icon was copied. See `source/askbridge-mark-transparent.svg` for the
authoritative source.

| Element | Spec |
|---|---|
| Corner brackets | 4 paths, stroke 68 px, `stroke-linecap="square"`, color `#2C2C2A` |
| Bracket arms | 220 px long on each axis |
| Bridge | Single filled path, color `#D85A30` |
| Bridge outer width | 440 px |
| Bridge outer height | 500 px (y: 260 → 760) |
| Bridge inner arch radius | 148 px |
| Padding around art | 100 px on every side |

### Minimum size

| Variant | Minimum width |
|---|---|
| Mark only (transparent) | 16 px |
| Mark only (cream) | 16 px |
| Mark with wordmark (horizontal lockup) | 240 px |
| Mark with wordmark (stacked lockup) | 120 px |

Below 16 px the inner arch collapses into a single pixel. Do not ship smaller
marks — fall back to the monochrome variant or the wordmark instead.

### Clear space

Reserve a clear space of **at least 64 px** (in the 1024 grid; ≈ 6.25% of the
mark width) on every side of the mark. No other graphic, type, or background
edge may enter this zone.

---

## 2. Color

The palette is three colors plus a dark-mode swap. **Do not introduce new
colors** without updating this document and the SVG sources together.

### Primary palette

| Role | Name | HEX | RGB | CMYK | Notes |
|---|---|---|---|---|---|
| Brand | Terracotta | `#D85A30` | 216, 90, 48 | 0, 58, 78, 15 | The bridge. The only warm accent. |
| Ink | Charcoal | `#2C2C2A` | 44, 44, 42 | 0, 0, 5, 83 | Corner brackets, wordmark, body type. |
| Surface | Cream | `#FAEEDA` | 250, 238, 218 | 0, 5, 13, 2 | Default background. |

### Secondary / supporting

| Role | HEX | Use |
|---|---|---|
| Accent (text) | `#993C1D` | Subtitles, the `→ AI bridge` line. Terracotta darkened for legibility. |
| Muted text | `#5F5E5A` | Taglines, captions, metadata. |
| Hairline | `#D3D1C7` | Borders on UI mockups, dividers. |
| Soft surface | `#F5C4B3` | Reverse-mode bridge fill (coral-100 from the c-coral ramp). |

### Reverse / dark surface

On a dark background, use `askbridge-mark-reverse.svg`. Replace the cream
canvas with `#2C2C2A`, the corner brackets with `#FAEEDA`, and the bridge with
`#F5C4B3` (do not switch to pure white — it competes with the brackets).

### Accessibility

| Pair | Contrast (WCAG) | Result |
|---|---|---|
| Charcoal on cream | 12.6 : 1 | AAA for body text |
| Terracotta on cream | 3.5 : 1 | AA for large text only — never use for body copy |
| Cream on charcoal | 11.9 : 1 | AAA |
| Charcoal on terracotta | 3.6 : 1 | AA for large text only |

---

## 3. Typography

The wordmark is set in a free-licensed sans-serif with a strong humanist cut.
We recommend (in order of preference):

| Rank | Family | License | Why |
|---|---|---|---|
| 1 | **Inter** (Bold) | OFL 1.1 | Neutral, very legible at all sizes, free. |
| 2 | Manrope (Bold) | OFL 1.1 | Slightly rounder, more friendly. |
| 3 | Plus Jakarta Sans (Bold) | OFL 1.1 | Similar to Inter with a touch more personality. |
| 4 | System UI fallback | — | `-apple-system, "Segoe UI", system-ui, sans-serif` |

The reference lockups use **Inter Bold 700** at:

| Layout | "AskBridge" size | Subtitle size | Letter spacing |
|---|---|---|---|
| Horizontal lockup | 180 px | 48 px | -6 / -2 |
| Stacked lockup | 200 px | 48 px | -6 / -2 |
| GitHub social card | 140 px | 42 px + 30 px | -4 / -2 |

**Always set the wordmark bold.** A regular-weight wordmark looks like body
copy and breaks the brand impression.

### Tagline

The recommended English subtitle is **`screenshot → AI bridge`**.
A second-line longer variant is **`screenshot → AI bridge for Windows`**.
Both are set in 500 weight and use the accent color `#993C1D`.

For Chinese audiences the recommended tagline is **`截图 → AI 桥接`** or
**`把屏幕送到 AI 输入框`** — both use **思源黑体 SC Bold** (OFL 1.1).

---

## 4. Variants in this kit

| File | Use |
|---|---|
| `source/askbridge-mark.svg` | Cream background, full color. Use for marketing surfaces, README hero, social cards. |
| `source/askbridge-mark-transparent.svg` | Same mark, transparent background. Use everywhere a surface is already provided. |
| `source/askbridge-mark-mono.svg` | Single-color black. Use for stamping, etching, single-ink print, and engraving. |
| `source/askbridge-mark-reverse.svg` | Dark-mode swap: cream on charcoal. |
| `source/askbridge-lockup-horizontal.svg` | 1440 × 480 mark + wordmark + tagline, cream background. |
| `source/askbridge-lockup-stacked.svg` | 1024 × 1280 vertical layout. |
| `source/askbridge-social-card.svg` | 1280 × 640 GitHub social preview. |
| `source/askbridge-readme-header.svg` | 1600 × 400 README header banner. |
| `source/askbridge-avatar.svg` | 460 × 460 GitHub profile / org avatar. |

PNG renders of every variant live in `icons/`, `web/`, `favicon/`, and
`github/`. See `INVENTORY.md` for the full file list with byte sizes.

---

## 5. Do and do not

**Do**

- Use the supplied SVG source when you need a custom size. Render with
  `resvg`, Inkscape, or `librsvg`; never with a browser screenshot of a
  different-size PNG.
- Pair the mark with the recommended font stack. If Inter is unavailable,
  fall back to the system UI stack and verify letter spacing visually.
- Keep the terracotta-to-charcoal ratio. The bridge is always a single color.

**Do not**

- Do not rotate, skew, or outline the mark.
- Do not change the bridge to a different shape (rainbow, hexagon, …) and
  call it the AskBridge mark.
- Do not place the mark on a busy photograph without a solid plate behind it.
- Do not use the terracotta on the cream background for body text — it fails
  AA contrast.
- Do not introduce blue, cyan, or purple into the palette. The point of the
  brand is that it is **not** an AI-styled product.
- Do not stretch the lockup non-proportionally. If a layout needs a different
  aspect, ask for a new lockup instead of scaling the existing one.

---

## 6. Trademark & legal

- The artwork is original geometry built for this project. No reference icon
  (ShareX, Flameshot, Greenshot, Shottr, PowerToys, or any other) was copied
  in form or color, but the design space was studied for principles. See
  `references.md` in the design history.
- Before filing a trademark application, run a clearance search on:
  - **CNIPA** (中国国家知识产权局商标局) — https://sbj.cnipa.gov.cn
  - **USPTO TESS** — https://tmsearch.uspto.gov
  - **WIPO Global Brand Database** — https://branddb.wipo.int
  Suggested Nice classes: **9** (downloadable software), **42** (SaaS /
  hosted utilities).
- Recommended mark description for filings: *"a square icon consisting of four
  black corner brackets surrounding a centered terracotta-red bridge arch on a
  warm cream background"*.
- Keep a copy of every source SVG and the build scripts — they are the design
  provenance and the strongest defense against an opposition.

---

## 7. Build pipeline

```text
source/*.svg   ──► resvg-py   ──► icons/*.png, web/*.png, github/*.png
                                  │
                                  ├─► build_pixel_icons.py ──► icons/askbridge-cream-{16,20,24}.png
                                  ├─► build_ico.py       ──► icons/askbridge.ico
                                  ├─► build_favicon.py   ──► favicon/*
                                  └─► build_review_sheets.py ──► docs/*.png
```

To re-render the whole kit after editing an SVG:

```powershell
cd D:\askbridge\assets\branding\askbridge-final
python scripts/render_assets.py
python scripts/build_pixel_icons.py
python scripts/build_ico.py
python scripts/build_favicon.py
python scripts/build_review_sheets.py
```

All four scripts are idempotent — running them on an unchanged source produces
byte-identical output (modulo PNG encoder metadata).

---

_Last updated: 2026-08-26 — initial publication with the v3 mark ("four
corners + bridge")._
