#!/usr/bin/env python3
"""Pixel-diff old/new captures of the same names to find unmarked changes.

usage: diff.py --new-dir DIR --old-dir DIR
Prints per image: changed-pixel count and bounding box (x0,y0,x1,y1).
Look at the results side by side; clocks, sync progress, banners, scroll
offsets and title-bar versions are noise, not changes.
"""

import argparse
import os

import numpy as np
from PIL import Image, ImageChops

p = argparse.ArgumentParser()
p.add_argument("--new-dir", required=True)
p.add_argument("--old-dir", required=True)
a = p.parse_args()
for f in sorted(os.listdir(a.new_dir)):
    if not f.endswith(".png"):
        continue
    o = os.path.join(a.old_dir, f)
    if not os.path.exists(o):
        print(f, "NEW ONLY")
        continue
    x = Image.open(o).convert("RGB")
    y = Image.open(os.path.join(a.new_dir, f)).convert("RGB")
    if x.size != y.size:
        print(f, "SIZE DIFFERS", x.size, y.size)
        continue
    m = np.array(ImageChops.difference(x, y)).sum(2) > 40
    ys, xs = np.where(m)
    print(f, int(m.sum()), (xs.min(), ys.min(), xs.max(), ys.max()) if m.any() else "")
