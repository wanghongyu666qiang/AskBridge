#!/usr/bin/env python
"""Generate INVENTORY.md (asset list with sizes)."""
from pathlib import Path

ROOT = Path(r"D:\askbridge\assets\branding\askbridge-final")
rows = []
for p in sorted(ROOT.rglob("*")):
    if p.is_file() and not p.name.startswith("_"):
        size = p.stat().st_size
        rel = p.relative_to(ROOT).as_posix()
        if size >= 1024:
            sz = f"{size/1024:.1f} KB"
        else:
            sz = f"{size} B"
        rows.append((rel, sz))

lines = ["# Asset inventory", "", f"Files in this kit: **{len(rows)}**", "",
          "| File | Size |", "|---|---:|"]
for rel, sz in rows:
    lines.append(f"| `{rel}` | {sz} |")
(ROOT / "INVENTORY.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"wrote INVENTORY.md with {len(rows)} rows")
