#!/usr/bin/env python3
"""Render spotlight-style annotated screenshots from a JSON spec.

usage: render.py SPEC.json --new-dir DIR [--old-dir DIR] --out-dir DIR [--scale 2]

SPEC: {"<image name>": [{"kind": "new|changed|removed|screen",
                         "rect": [x0, y0, x1, y1],          # capture pixels (1x)
                         "label": "Changed: profile name",  # 2-5 words
                         "pill": [x, y],                    # pill top-left, 1x
                         "inset": {"old": "<old image name>", # optional, removed items
                                   "crop": [x0, y0, x1, y1], "xy": [x, y]}}]}

Reads <new-dir>/<name>.png (and <old-dir>/<name>.png for the self-check and
insets); writes <out-dir>/<name>.png (labelled) and <out-dir>/clean/<name>.png.
Without --old-dir (NEW-only mode) the rect self-check is skipped and insets are
refused. Exit code 1 if a self-check fails (see selfcheck()); warnings do not
change it.
"""

import argparse
import json
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

COL = {"new": (0, 168, 84), "changed": (123, 63, 242), "removed": (229, 57, 53)}
PAD = 5
FONTS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans{}.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans{}.ttf",
    "/Library/Fonts/DejaVuSans{}.ttf",
]
BAD = []
WARN = []
# A rect with fewer than NOISE_PIXELS pixels differing by more than NOISE_LEVEL
# (per channel, 0-255) gets a warning: check by eye that it marks the change.
NOISE_LEVEL = 24
NOISE_PIXELS = 20


def font(size, scale, bold=True):
    for pattern in FONTS:
        path = pattern.format("-Bold" if bold else "")
        if os.path.exists(path):
            return ImageFont.truetype(path, int(size * scale))
    return ImageFont.load_default()


def rr(d, r, rad, **k):
    d.rounded_rectangle(r, radius=rad, **k)


def build(name, els, a, labels):
    new_dir, old_dir, scale = a.new_dir, a.old_dir, a.scale
    s = scale
    base = Image.open(os.path.join(new_dir, name + ".png")).convert("RGB")
    base = base.resize((base.width * s, base.height * s), Image.LANCZOS)
    if any(e["kind"] == "screen" for e in els):
        img = base.copy()  # a wholly new screen is not dimmed
    else:
        dim = base.convert("L").convert("RGB")
        dim = Image.blend(dim, Image.new("RGB", base.size, (150, 150, 150)), 0.5)
        mask = Image.new("L", base.size, 0)
        md = ImageDraw.Draw(mask)
        for e in els:
            if e["kind"] == "removed":
                continue  # nothing of it exists in the new capture
            r = e["rect"]
            rr(
                md,
                [
                    (r[0] - PAD) * s,
                    (r[1] - PAD) * s,
                    (r[2] + PAD) * s,
                    (r[3] + PAD) * s,
                ],
                10 * s,
                fill=255,
            )
        img = Image.composite(base, dim, mask.filter(ImageFilter.GaussianBlur(1)))
    d = ImageDraw.Draw(img)
    for e in els:
        k, r, lab = e["kind"], e["rect"], e["label"]
        c = COL["new" if k == "screen" else k]
        if k in ("screen", "removed"):
            box = [r[0] * s, r[1] * s, r[2] * s, r[3] * s]
        else:
            box = [
                (r[0] - PAD) * s,
                (r[1] - PAD) * s,
                (r[2] + PAD) * s,
                (r[3] + PAD) * s,
            ]
        if k == "removed":  # dashed outline
            x0, y0, x1, y1 = box
            dash = 18 * s // 2
            for x in range(int(x0), int(x1), dash * 2):
                d.line((x, y0, min(x + dash, x1), y0), fill=c, width=4 * s)
                d.line((x, y1, min(x + dash, x1), y1), fill=c, width=4 * s)
            for y in range(int(y0), int(y1), dash * 2):
                d.line((x0, y, x0, min(y + dash, y1)), fill=c, width=4 * s)
                d.line((x1, y, x1, min(y + dash, y1)), fill=c, width=4 * s)
        else:
            rr(d, box, 10 * s, outline=c, width=4 * s)
        px, py = e["pill"][0] * s, e["pill"][1] * s
        f = font(20, s)
        w, h = d.textlength(lab, font=f) + 28 * s, 36 * s
        if px < 0 or py < 0 or px + w > img.width or py + h > img.height:
            BAD.append((name, lab, "pill outside image"))
        if not labels:
            continue
        cx = min(max(px + w / 2, box[0]), box[2])
        cy = min(max(py + h / 2, box[1]), box[3])
        side = px > box[2] or px + w < box[0]
        sx = px if px > box[2] else (px + w if px + w < box[0] else px + w / 2)
        sy = py + h / 2 if side else (py if py > box[3] else py + h)
        tx = box[2] if px > box[2] else (box[0] if px + w < box[0] else cx)
        ty = cy if side else (box[3] if py > box[3] else box[1])
        if not (box[0] < px + w / 2 < box[2] and box[1] < py + h / 2 < box[3]):
            d.line((sx, sy, tx, ty), fill=c, width=3 * s)  # leader line
        rr(d, [px, py, px + w, py + h], 8 * s, fill=c)
        d.text((px + 14 * s, py + h / 2), lab, fill="white", font=f, anchor="lm")
        ins = e.get("inset")
        if ins and old_dir is None:
            BAD.append((name, lab, "inset needs --old-dir"))
        elif ins:
            o = (
                Image.open(os.path.join(old_dir, ins["old"] + ".png"))
                .convert("RGB")
                .crop(ins["crop"])
            )
            o = o.resize((o.width * s, o.height * s), Image.LANCZOS)
            ix, iy = ins["xy"][0] * s, ins["xy"][1] * s
            img.paste(o, (ix, iy))
            d.rectangle(
                (ix - 2 * s, iy - 2 * s, ix + o.width + 2 * s, iy + o.height + 2 * s),
                outline=c,
                width=3 * s,
            )
            d.text(
                (ix, iy - 6 * s),
                "Before (previous build)",
                fill=c,
                font=font(14, s),
                anchor="ld",
            )
    # corner key
    f = font(15, s)
    items = [("new", "New"), ("changed", "Changed"), ("removed", "Removed")]
    kw = int(sum(d.textlength(t, font=f) + 60 * s for _, t in items)) + 10 * s
    kh = 34 * s
    kx, ky = img.width - kw - 16 * s, img.height - kh - 12 * s
    rr(
        d,
        [kx, ky, kx + kw, ky + kh],
        8 * s,
        fill=(255, 255, 255),
        outline=(120, 120, 120),
        width=s,
    )
    x = kx + 14 * s
    for kk, t in items:
        rr(
            d,
            [x, ky + 9 * s, x + 22 * s, ky + kh - 9 * s],
            4 * s,
            outline=COL[kk],
            width=3 * s,
        )
        d.text((x + 30 * s, ky + kh / 2), t, fill=(30, 30, 40), font=f, anchor="lm")
        x += d.textlength(t, font=f) + 60 * s
    return img


def selfcheck(name, els, a):
    """Every new/changed rect must differ between the NEW and OLD capture of the
    same screen; a highlight over identical pixels means a wrong rect.

    Compared in RGB, so a change of colour alone counts. Only a rect with no
    differing pixel at all fails; a rect that differs very little is reported
    as a warning, because a subtle restyle is a legitimate highlight."""
    if a.old_dir is None:
        return  # NEW-only mode: no OLD capture to compare against
    op = os.path.join(a.old_dir, name + ".png")
    if not os.path.exists(op):
        return  # new-only screen: nothing to compare
    n = Image.open(os.path.join(a.new_dir, name + ".png")).convert("RGB")
    o = Image.open(op).convert("RGB")
    for e in els:
        if e["kind"] not in ("new", "changed"):
            continue
        ca = np.asarray(n.crop(e["rect"]), dtype=int)
        cb = np.asarray(o.crop(e["rect"]), dtype=int)
        delta = abs(ca - cb).max(axis=2)  # largest channel difference per pixel
        if not delta.any():
            BAD.append((name, e["label"], "rect identical in old and new"))
        elif int((delta > NOISE_LEVEL).sum()) < NOISE_PIXELS:
            WARN.append(
                (
                    name,
                    e["label"],
                    f"rect barely differs between old and new "
                    f"({int((delta > 0).sum())} px, largest difference {int(delta.max())} of 255)",
                )
            )


def main():
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("spec")
    p.add_argument("--new-dir", required=True)
    p.add_argument("--old-dir", help="OLD captures; omit for NEW-only mode")
    p.add_argument("--out-dir", required=True)
    p.add_argument("--scale", type=int, default=2)
    a = p.parse_args()
    spec = json.load(open(a.spec))
    os.makedirs(os.path.join(a.out_dir, "clean"), exist_ok=True)
    if a.old_dir is None:
        print("NEW-only mode: rect self-check skipped, check every rect by eye")
    for name, els in spec.items():
        selfcheck(name, els, a)
        build(name, els, a, True).save(os.path.join(a.out_dir, name + ".png"))
        build(name, els, a, False).save(os.path.join(a.out_dir, "clean", name + ".png"))
    if WARN:
        print("SELFCHECK WARNING (not a failure, check these rects by eye):")
        for w in sorted(set(WARN)):
            print("  ", *w)
    if BAD:
        print("SELFCHECK FAIL:")
        for b in sorted(set(BAD)):
            print("  ", *b)
        sys.exit(1)
    print("selfcheck ok,", len(spec), "images")


if __name__ == "__main__":
    main()
