#!/usr/bin/env python3
"""Build a single browsable index.html out of the folders release-check.sh
just filled with screenshots and manifest.json files.

    ci/certify/release-check-report.py <out-dir> <variant-name> [<variant-name> ...]

No dependencies beyond the standard library, on purpose — this only ever
needs to run right after release-check.sh, on whichever of the three
platforms it just ran on, and none of them are guaranteed to have anything
installed beyond what ci/check/checks.sh already needs.
"""
from __future__ import annotations

import html
import json
import sys
from pathlib import Path


def load_manifest(variant_dir: Path) -> dict:
    for manifest_path in sorted(variant_dir.glob("*manifest.json")):
        try:
            return json.loads(manifest_path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            return {"steps": [], "warnings": [f"could not read {manifest_path.name}: {exc}"], "frame_errors": 0}
    # `tour` and `walkthrough` write plain PNGs with no manifest — build one
    # from whatever is on disk so their sections still render in the gallery.
    steps = [
        {"section": "", "name": png.stem, "file": png.name, "shapes": 1, "frame_errors": 0}
        for png in sorted(variant_dir.glob("*.png"))
    ]
    return {"steps": steps, "warnings": [], "frame_errors": 0}


def section_html(variant: str, out_dir: Path) -> str:
    variant_dir = out_dir / variant
    manifest = load_manifest(variant_dir)
    status = (variant_dir / "status.txt").read_text().strip() if (variant_dir / "status.txt").exists() else "?"
    steps = manifest.get("steps", [])
    warnings = manifest.get("warnings", [])
    frame_errors = manifest.get("frame_errors", 0)

    badge = "pass" if status == "OK" and not frame_errors else "fail"
    parts = [
        f'<section class="variant" id="{html.escape(variant)}">',
        f'<h2><span class="badge {badge}">{html.escape(status)}</span> {html.escape(variant)}'
        f' <span class="count">{len(steps)} shots · {len(warnings)} warnings · {frame_errors} frame errors</span></h2>',
    ]
    if warnings:
        parts.append('<details class="warnings" open><summary>Warnings</summary><ul>')
        for w in warnings:
            parts.append(f"<li>{html.escape(str(w))}</li>")
        parts.append("</ul></details>")

    parts.append('<div class="grid">')
    current_group = None
    for step in steps:
        group = step.get("section", "")
        if group != current_group:
            parts.append(f'<h3 class="group">{html.escape(group)}</h3>')
            current_group = group
        rel = f"{variant}/{step.get('file', '')}"
        name = step.get("name", "")
        shapes = step.get("shapes", 0)
        errs = step.get("frame_errors", 0)
        warn_class = " suspect" if shapes == 0 or errs else ""
        parts.append(
            f'<figure class="shot{warn_class}">'
            f'<a href="{html.escape(rel)}" target="_blank">'
            f'<img loading="lazy" src="{html.escape(rel)}" alt="{html.escape(name)}"></a>'
            f'<figcaption>{html.escape(name)}</figcaption>'
            f"</figure>"
        )
    parts.append("</div></section>")
    return "\n".join(parts)


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    out_dir = Path(argv[0])
    variants = argv[1:]

    env_text = ""
    env_path = out_dir / "environment.txt"
    if env_path.exists():
        env_text = env_path.read_text()

    nav = "".join(f'<a href="#{html.escape(v)}">{html.escape(v)}</a>' for v in variants)
    sections = "\n".join(section_html(v, out_dir) for v in variants)

    page = f"""<!doctype html>
<html><head><meta charset="utf-8">
<title>vieww Studio — release check</title>
<style>
:root {{ color-scheme: light dark; }}
body {{ font-family: -apple-system, Segoe UI, sans-serif; margin: 0; background: #111318; color: #e8e8ec; }}
header {{ position: sticky; top: 0; background: #181b22; padding: 12px 20px; border-bottom: 1px solid #2a2e38; z-index: 2; }}
header h1 {{ font-size: 15px; margin: 0 0 6px; font-weight: 600; }}
header nav a {{ color: #8ab4ff; margin-right: 14px; font-size: 12.5px; text-decoration: none; }}
header nav a:hover {{ text-decoration: underline; }}
pre.env {{ font-size: 11px; color: #9aa0ac; margin: 6px 0 0; white-space: pre-wrap; }}
section.variant {{ padding: 18px 20px; border-bottom: 1px solid #23262e; }}
h2 {{ font-size: 14px; margin: 0 0 10px; }}
h3.group {{ font-size: 12px; color: #9aa0ac; text-transform: uppercase; letter-spacing: .04em;
  margin: 18px 0 6px; grid-column: 1 / -1; }}
.badge {{ display: inline-block; font-size: 10.5px; font-weight: 700; padding: 2px 7px; border-radius: 4px; margin-right: 8px; }}
.badge.pass {{ background: #1e3a2a; color: #7ee0a0; }}
.badge.fail {{ background: #402024; color: #ff9aa0; }}
.count {{ color: #9aa0ac; font-weight: 400; font-size: 12px; }}
details.warnings {{ background: #241f14; border: 1px solid #4a3a1a; border-radius: 6px; padding: 8px 12px; margin-bottom: 12px; }}
details.warnings summary {{ cursor: pointer; color: #e8c37a; font-size: 12.5px; }}
details.warnings li {{ font-size: 12px; color: #e0c98f; margin: 3px 0; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 10px; }}
figure.shot {{ margin: 0; background: #1a1d24; border: 1px solid #262a33; border-radius: 6px; overflow: hidden; }}
figure.shot.suspect {{ border-color: #6a3030; }}
figure.shot img {{ display: block; width: 100%; height: 130px; object-fit: cover; object-position: top; background: #0c0d10; }}
figure.shot figcaption {{ font-size: 10.5px; color: #b7bcc6; padding: 5px 7px; white-space: nowrap;
  overflow: hidden; text-overflow: ellipsis; }}
</style></head>
<body>
<header>
  <h1>vieww Studio — release check</h1>
  <nav>{nav}</nav>
  <pre class="env">{html.escape(env_text)}</pre>
</header>
{sections}
</body></html>
"""
    (out_dir / "index.html").write_text(page)
    print(f"release-check-report: wrote {out_dir / 'index.html'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
