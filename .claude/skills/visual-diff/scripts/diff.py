#!/usr/bin/env python3
"""Pixel-diff old/new captures of the same names to find unmarked changes.

usage: diff.py --new-dir DIR --old-dir DIR
Prints per capture name one of:
  NAME NEW ONLY | OLD ONLY          the screen exists in one build only
  NAME SIZE DIFFERS (w, h) (w, h)
  NAME identical
  NAME changed=N above_noise=M bbox=(x0, y0, x1, y1)
changed counts every pixel that differs at all; above_noise counts those whose
summed RGB difference exceeds NOISE. The box encloses the above-noise pixels,
or every differing pixel when none is above the noise level.
Look at the results side by side; clocks, sync progress, banners, scroll
offsets and title-bar versions are noise, not changes.
"""

import argparse
import os

import numpy as np
from PIL import Image, ImageChops

# A summed RGB difference at or below this is usually rendering noise.
NOISE = 40


def captures(directory):
    return {f for f in os.listdir(directory) if f.endswith(".png")}


def bbox(mask):
    ys, xs = np.where(mask)
    return (int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max()))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--new-dir", required=True)
    p.add_argument("--old-dir", required=True)
    a = p.parse_args()
    new, old = captures(a.new_dir), captures(a.old_dir)
    for f in sorted(new | old):
        if f not in old:
            print(f, "NEW ONLY")
            continue
        if f not in new:
            print(f, "OLD ONLY")
            continue
        x = Image.open(os.path.join(a.old_dir, f)).convert("RGB")
        y = Image.open(os.path.join(a.new_dir, f)).convert("RGB")
        if x.size != y.size:
            print(f, "SIZE DIFFERS", x.size, y.size)
            continue
        delta = np.asarray(ImageChops.difference(x, y), dtype=int).sum(2)
        differs = delta > 0
        if not differs.any():
            print(f, "identical")
            continue
        strong = delta > NOISE
        print(
            f,
            f"changed={int(differs.sum())}",
            f"above_noise={int(strong.sum())}",
            f"bbox={bbox(strong if strong.any() else differs)}",
        )


if __name__ == "__main__":
    main()
