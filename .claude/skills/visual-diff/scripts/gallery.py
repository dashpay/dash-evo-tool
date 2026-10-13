#!/usr/bin/env python3
"""Build index.html (gallery + commit-coverage table) from a meta JSON.

usage: gallery.py --root DIR --meta meta.json [--title TITLE]

DIR holds annotated/<name>.png and annotated/clean/<name>.png (render.py output).
meta.json:
  {"intro": "text",
   "images": [{"name": "08-masternodes-nodes", "title": "Masternodes: Nodes"}],
   "coverage": [{"commit": "abc1234", "subject": "feat: ...", "screen": "Identities",
                 "images": ["04-identities-empty"], "note": "optional"}],
   "not_reached": ["what was not covered and why"],
   "presenter_notes": ["faces in profile pictures", "banners", "data differing between shots"]}
A coverage row with no images is rendered as "not visible" and should carry a
note saying why. Text is HTML-escaped.
"""

import argparse
import html
import json
import os

e = html.escape
CSS = """:root{--bg:#fff;--fg:#1c1e21;--mut:#666;--line:#d0d4da}
@media(prefers-color-scheme:dark){:root{--bg:#16181c;--fg:#e6e8eb;--mut:#9aa0a6;--line:#3a3f46}}
body{background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif;max-width:1100px;margin:0 auto;padding:16px}
img{max-width:100%;height:auto;border:1px solid var(--line)}
table{border-collapse:collapse;width:100%}td,th{border:1px solid var(--line);padding:4px 8px;text-align:left;vertical-align:top}
.mut{color:var(--mut)}"""


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--root", required=True)
    p.add_argument("--meta", required=True)
    p.add_argument("--title", default="Visual diff")
    a = p.parse_args()
    m = json.load(open(a.meta))
    known = {i["name"] for i in m.get("images", [])}
    for r in m.get("coverage", []):
        for n in r.get("images", []):
            if n not in known or not os.path.exists(
                os.path.join(a.root, "annotated", n + ".png")
            ):
                raise SystemExit(f"coverage refers to a missing image: {n}")
    o = [
        f"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>{e(a.title)}</title><style>{CSS}</style></head><body>"
    ]
    o.append(f"<h1>{e(a.title)}</h1><p>{e(m.get('intro', ''))}</p>")
    for i in m.get("images", []):
        n = i["name"]
        o.append(
            f'<h2 id="{e(n)}">{e(n[:3].rstrip("-"))} &middot; {e(i["title"])}</h2>'
        )
        o.append(f'<img src="annotated/{e(n)}.png" alt="{e(i["title"])}">')
        o.append(
            f'<p><a href="annotated/clean/{e(n)}.png">Version without labels</a></p>'
        )
    o.append(
        "<h2>Commit coverage</h2><table><tr><th>Commit<th>Subject<th>Screen<th>Image<th>Note</tr>"
    )
    for r in m.get("coverage", []):
        links = ", ".join(
            f'<a href="#{e(n)}">{e(n[:3].rstrip("-"))}</a>' for n in r.get("images", [])
        )
        o.append(
            f"<tr><td><code>{e(r['commit'])}</code><td>{e(r['subject'])}<td>{e(r.get('screen', ''))}<td>{links or '<b>not visible</b>'}<td>{e(r.get('note', ''))}</tr>"
        )
    o.append("</table>")
    for key, head in (
        ("not_reached", "Not reached or not verified"),
        ("presenter_notes", "Presenter notes"),
    ):
        if m.get(key):
            o.append(
                f"<h2>{head}</h2><ul>"
                + "".join(f"<li>{e(x)}</li>" for x in m[key])
                + "</ul>"
            )
    o.append("</body></html>")
    open(os.path.join(a.root, "index.html"), "w").write("\n".join(o))
    print("wrote", os.path.join(a.root, "index.html"))


if __name__ == "__main__":
    main()
