#!/usr/bin/env python3
"""Regression tests for render.py, diff.py and gallery.py on synthetic images.

usage: selftest.py
Needs no display, no application build and no captured data. Run it after
changing any script in this directory.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
SIZE = (480, 240)
GREY = (128, 128, 128)
RECT = [40, 40, 120, 100]
PILL = [150, 40]


def image(path, *patches):
    """Grey image with (box, colour) patches painted on it."""
    img = Image.new("RGB", SIZE, GREY)
    d = ImageDraw.Draw(img)
    for box, colour in patches:
        d.rectangle(box, fill=colour)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.save(path)


def run(script, *args):
    r = subprocess.run(
        [sys.executable, os.path.join(HERE, script), *args],
        capture_output=True,
        text=True,
    )
    return r.returncode, r.stdout + r.stderr


class Case(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        self.addCleanup(self.tmp.cleanup)

    def path(self, *parts):
        return os.path.join(self.root, *parts)


class RenderSelfCheck(Case):
    def render(self, old_patch, new_patch, pill=PILL, old=True, inset=None):
        image(self.path("old", "s.png"), *old_patch)
        image(self.path("new", "s.png"), *new_patch)
        el = {"kind": "changed", "rect": RECT, "label": "Changed: x", "pill": pill}
        if inset:
            el["inset"] = inset
        with open(self.path("spec.json"), "w") as f:
            json.dump({"s": [el]}, f)
        old_args = ["--old-dir", self.path("old")] if old else []
        return run(
            "render.py",
            self.path("spec.json"),
            "--new-dir",
            self.path("new"),
            *old_args,
            "--out-dir",
            self.path("out"),
        )

    def test_identical_rect_fails(self):
        code, out = self.render([], [])
        self.assertEqual(code, 1, out)
        self.assertIn("rect identical in old and new", out)

    def test_colour_change_of_equal_brightness_passes_silently(self):
        # (255, 0, 0) and (0, 130, 0) have the same grey level.
        code, out = self.render([(RECT, (255, 0, 0))], [(RECT, (0, 130, 0))])
        self.assertEqual(code, 0, out)
        self.assertNotIn("WARNING", out)
        self.assertNotIn("FAIL", out)

    def test_low_contrast_change_warns_but_passes(self):
        code, out = self.render([], [(RECT, (140, 140, 140))])
        self.assertEqual(code, 0, out)
        self.assertIn("SELFCHECK WARNING", out)
        self.assertIn("rect barely differs", out)

    def test_small_changed_region_warns_but_passes(self):
        code, out = self.render([], [([50, 50, 52, 52], (255, 255, 255))])
        self.assertEqual(code, 0, out)
        self.assertIn("rect barely differs", out)

    def test_pill_outside_image_fails(self):
        code, out = self.render([], [(RECT, (255, 255, 255))], pill=[470, 40])
        self.assertEqual(code, 1, out)
        self.assertIn("pill outside image", out)

    def test_passing_run_writes_both_variants(self):
        code, out = self.render([], [(RECT, (255, 255, 255))])
        self.assertEqual(code, 0, out)
        self.assertTrue(os.path.exists(self.path("out", "s.png")))
        self.assertTrue(os.path.exists(self.path("out", "clean", "s.png")))

    def test_new_only_mode_skips_the_selfcheck(self):
        code, out = self.render([], [], old=False)
        self.assertEqual(code, 0, out)
        self.assertIn("NEW-only mode", out)
        self.assertTrue(os.path.exists(self.path("out", "s.png")))
        self.assertTrue(os.path.exists(self.path("out", "clean", "s.png")))

    def test_new_only_mode_refuses_an_inset(self):
        inset = {"old": "s", "crop": [0, 0, 20, 20], "xy": [300, 150]}
        code, out = self.render([], [], old=False, inset=inset)
        self.assertEqual(code, 1, out)
        self.assertIn("inset needs --old-dir", out)


class Diff(Case):
    def diff(self):
        code, out = run(
            "diff.py", "--new-dir", self.path("new"), "--old-dir", self.path("old")
        )
        self.assertEqual(code, 0, out)
        return out.splitlines()

    def test_reports_screens_of_one_build_only(self):
        image(self.path("old", "gone.png"))
        image(self.path("new", "added.png"))
        self.assertEqual(self.diff(), ["added.png NEW ONLY", "gone.png OLD ONLY"])

    def test_subtle_change_is_not_reported_as_identical(self):
        image(self.path("old", "s.png"), (RECT, (100, 100, 100)))
        image(self.path("new", "s.png"), (RECT, (112, 112, 112)))
        pixels = (RECT[2] - RECT[0] + 1) * (RECT[3] - RECT[1] + 1)
        self.assertEqual(
            self.diff(),
            [f"s.png changed={pixels} above_noise=0 bbox=(40, 40, 120, 100)"],
        )

    def test_box_follows_the_pixels_above_noise(self):
        image(self.path("old", "s.png"))
        image(
            self.path("new", "s.png"),
            (RECT, (130, 130, 130)),
            ([300, 150, 309, 159], (255, 255, 255)),
        )
        (line,) = self.diff()
        self.assertIn("above_noise=100 bbox=(300, 150, 309, 159)", line)

    def test_identical_captures(self):
        image(self.path("old", "s.png"))
        image(self.path("new", "s.png"))
        self.assertEqual(self.diff(), ["s.png identical"])


class Gallery(Case):
    def gallery(self, meta, files):
        for f in files:
            image(self.path(*f))
        with open(self.path("meta.json"), "w") as f:
            json.dump(meta, f)
        return run("gallery.py", "--root", self.root, "--meta", self.path("meta.json"))

    META = {"images": [{"name": "01-a", "title": "A"}], "coverage": []}
    BOTH = [("annotated", "01-a.png"), ("annotated", "clean", "01-a.png")]

    def test_complete_gallery_is_written(self):
        code, out = self.gallery(self.META, self.BOTH)
        self.assertEqual(code, 0, out)
        self.assertTrue(os.path.exists(self.path("index.html")))

    def test_missing_labelled_image_stops_the_gallery(self):
        code, out = self.gallery(self.META, self.BOTH[1:])
        self.assertNotEqual(code, 0, out)
        self.assertIn("image file is missing", out)
        self.assertFalse(os.path.exists(self.path("index.html")))

    def test_missing_clean_image_stops_the_gallery(self):
        code, out = self.gallery(self.META, self.BOTH[:1])
        self.assertNotEqual(code, 0, out)
        self.assertIn("image file is missing", out)
        self.assertFalse(os.path.exists(self.path("index.html")))

    def test_coverage_of_an_unlisted_image_stops_the_gallery(self):
        meta = dict(
            self.META, coverage=[{"commit": "abc", "subject": "s", "images": ["02-b"]}]
        )
        code, out = self.gallery(meta, self.BOTH)
        self.assertNotEqual(code, 0, out)
        self.assertIn("not listed", out)


if __name__ == "__main__":
    unittest.main()
