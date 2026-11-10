#!/usr/bin/env python3
"""Regression tests for render.py, diff.py and gallery.py on synthetic images,
and for env-value.awk on throwaway env files.

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
AMBER = (245, 124, 0)
VIOLET = (123, 63, 242)
INK = (30, 30, 40)
WHITE = (255, 255, 255)


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


class RenderDefect(Case):
    """A defect is outlined in amber, its pill carries a marker and a dark
    label, and render.py lists it in annotated/defects.json for gallery.py."""

    def render(self, *els, old=True, new_patch=()):
        return self.render_spec({"s": list(els)}, old=old, new_patch=new_patch)

    def render_spec(self, spec, old=True, new_patch=()):
        for name in spec:
            image(self.path("old", name + ".png"))
            image(self.path("new", name + ".png"), *new_patch)
        with open(self.path("spec.json"), "w") as f:
            json.dump(spec, f)
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

    def defect(self, **extra):
        return dict(
            {"kind": "defect", "rect": RECT, "label": "Defect: x", "pill": PILL},
            **extra,
        )

    def amber_pixels(self, *parts):
        px = Image.open(self.path("out", *parts)).convert("RGB").getdata()
        return sum(1 for p in px if p == AMBER)

    def pill(self, fill):
        """The labelled pill at PILL (2x): its pixels, found by its fill."""
        img = Image.open(self.path("out", "s.png")).convert("RGB")
        x0, y0 = PILL[0] * 2, PILL[1] * 2
        row = y0 + 18  # below the rounded corners, above the text
        x1 = x0
        while x1 < img.width and img.getpixel((x1 + 20, row)) == fill:
            x1 += 1
        y1 = y0
        while y1 < img.height and img.getpixel((x0 + 20, y1)) == fill:
            y1 += 1
        return x1 + 20 - x0, list(img.crop((x0, y0, x1 + 20, y1)).getdata())

    def defects_json(self):
        with open(self.path("out", "defects.json")) as f:
            return json.load(f)

    def test_a_defect_needs_no_change_between_builds(self):
        code, out = self.render(self.defect())
        self.assertEqual(code, 0, out)
        self.assertNotIn("FAIL", out)

    def test_the_outline_is_amber_in_both_variants(self):
        code, out = self.render(self.defect())
        self.assertEqual(code, 0, out)
        # Middle of the outline's left edge: (RECT x0 - PAD) * scale + 2.
        for parts in (("s.png",), ("clean", "s.png")):
            img = Image.open(self.path("out", *parts)).convert("RGB")
            self.assertEqual(img.getpixel((72, 140)), AMBER, parts)

    def test_the_pill_adds_amber_only_to_the_labelled_variant(self):
        code, out = self.render(self.defect())
        self.assertEqual(code, 0, out)
        self.assertGreater(
            self.amber_pixels("s.png"), self.amber_pixels("clean", "s.png")
        )

    def test_the_pill_is_read_without_colour(self):
        sys.dont_write_bytecode = True  # no __pycache__ in the repository
        import render

        self.assertEqual(render.pill_text(self.defect()), ("\u26a0 Defect: x", None))
        self.assertEqual(
            render.pill_text(self.defect(pre_existing=True)),
            ("\u26a0 Defect: x", "Existed before this release"),
        )
        changed = {"kind": "changed", "label": "Changed: x"}
        self.assertEqual(render.pill_text(changed), ("Changed: x", None))

    def test_the_marker_widens_the_pill(self):
        label = {"rect": RECT, "label": "Defect: x", "pill": PILL}
        self.assertEqual(self.render(dict(label, kind="changed"), old=False)[0], 0)
        changed_width, _ = self.pill(VIOLET)
        self.assertEqual(self.render(dict(label, kind="defect"), old=False)[0], 0)
        defect_width, _ = self.pill(AMBER)
        self.assertGreater(defect_width, changed_width + 20)

    def test_the_pill_text_is_dark_not_white(self):
        for el in (self.defect(), self.defect(pre_existing=True)):
            code, out = self.render(el, old=False)
            self.assertEqual(code, 0, out)
            _, pixels = self.pill(AMBER)
            self.assertGreater(pixels.count(INK), 50, el)
            self.assertEqual(pixels.count(WHITE), 0, el)

    def test_pre_existing_must_be_a_boolean(self):
        code, out = self.render(self.defect(pre_existing="false"), old=False)
        self.assertEqual(code, 1, out)
        self.assertIn("pre_existing must be true or false", out)

    def test_an_unchanged_defect_not_marked_pre_existing_warns(self):
        code, out = self.render(self.defect())
        self.assertEqual(code, 0, out)
        self.assertIn("defect identical in old and new", out)
        code, out = self.render(self.defect(pre_existing=True))
        self.assertEqual(code, 0, out)
        self.assertNotIn("WARNING", out)

    def test_a_changed_defect_marked_pre_existing_warns(self):
        new = [(RECT, (255, 255, 255))]
        code, out = self.render(self.defect(pre_existing=True), new_patch=new)
        self.assertEqual(code, 0, out)
        self.assertIn("pre-existing defect differs between old and new", out)
        code, out = self.render(self.defect(), new_patch=new)
        self.assertEqual(code, 0, out)
        self.assertNotIn("WARNING", out)

    def test_a_partial_render_keeps_the_other_images_defects(self):
        entry = {"label": "Defect: x", "pre_existing": False}
        self.assertEqual(self.render_spec({"a": [self.defect()]})[0], 0)
        self.assertEqual(self.render_spec({"b": [self.defect()]})[0], 0)
        self.assertEqual(self.defects_json(), {"a": [entry], "b": [entry]})
        changed = {"kind": "changed", "rect": RECT, "label": "Changed: x", "pill": PILL}
        self.assertEqual(self.render_spec({"a": [changed]}, old=False)[0], 0)
        self.assertEqual(self.defects_json(), {"b": [entry]})

    def test_an_image_without_a_defect_shows_no_amber(self):
        changed = {"kind": "changed", "rect": RECT, "label": "Changed: x", "pill": PILL}
        code, out = self.render(changed, old=False)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.amber_pixels("s.png"), 0)

    def test_a_defect_label_must_say_defect(self):
        code, out = self.render(self.defect(label="Broken x"))
        self.assertEqual(code, 1, out)
        self.assertIn('defect label must start with "Defect: "', out)

    def test_only_a_defect_can_be_pre_existing(self):
        changed = {
            "kind": "changed",
            "rect": RECT,
            "label": "Changed: x",
            "pill": PILL,
            "pre_existing": True,
        }
        code, out = self.render(changed, old=False)
        self.assertEqual(code, 1, out)
        self.assertIn("pre_existing applies only to a defect", out)

    def test_a_pre_existing_defect_has_a_taller_pill(self):
        # Fits as one line (36 px at 1x) but not with the second line.
        low = [PILL[0], SIZE[1] - 40]
        code, out = self.render(self.defect(pill=low))
        self.assertEqual(code, 0, out)
        code, out = self.render(self.defect(pill=low, pre_existing=True))
        self.assertEqual(code, 1, out)
        self.assertIn("pill outside image", out)

    def test_defects_are_listed_for_the_gallery(self):
        code, out = self.render(
            self.defect(),
            self.defect(label="Defect: y", pre_existing=True, pill=[150, 140]),
        )
        self.assertEqual(code, 0, out)
        with open(self.path("out", "defects.json")) as f:
            self.assertEqual(
                json.load(f),
                {
                    "s": [
                        {"label": "Defect: x", "pre_existing": False},
                        {"label": "Defect: y", "pre_existing": True},
                    ]
                },
            )


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
    def gallery(self, meta, files, defects=None):
        for f in files:
            image(self.path(*f))
        if defects is not None:
            with open(self.path("annotated", "defects.json"), "w") as f:
                json.dump(defects, f)
        with open(self.path("meta.json"), "w") as f:
            json.dump(meta, f)
        return run("gallery.py", "--root", self.root, "--meta", self.path("meta.json"))

    META = {"images": [{"name": "01-a", "title": "A"}], "coverage": []}
    BOTH = [("annotated", "01-a.png"), ("annotated", "clean", "01-a.png")]

    def index(self):
        with open(self.path("index.html")) as f:
            return f.read()

    def test_complete_gallery_is_written(self):
        code, out = self.gallery(self.META, self.BOTH, defects={})
        self.assertEqual(code, 0, out)
        self.assertNotIn("Known issues", self.index())

    def test_missing_defect_list_stops_the_gallery(self):
        code, out = self.gallery(self.META, self.BOTH)
        self.assertNotEqual(code, 0, out)
        self.assertIn("defects.json is missing", out)
        self.assertFalse(os.path.exists(self.path("index.html")))

    def test_images_with_defects_are_marked_and_listed(self):
        defects = {
            "01-a": [
                {"label": "Defect: <late> balance", "pre_existing": False},
                {"label": "Defect: clipped title", "pre_existing": True},
            ]
        }
        code, out = self.gallery(self.META, self.BOTH, defects)
        self.assertEqual(code, 0, out)
        page = self.index()
        self.assertIn('class="badge"', page)
        self.assertIn("<h2>Known issues</h2>", page)
        self.assertIn("Defect: &lt;late&gt; balance", page)
        self.assertIn("Defect: clipped title (existed before this release)", page)

    def test_a_defect_on_an_unlisted_image_stops_the_gallery(self):
        defects = {"02-b": [{"label": "Defect: x", "pre_existing": False}]}
        code, out = self.gallery(self.META, self.BOTH, defects)
        self.assertNotEqual(code, 0, out)
        self.assertIn("defects.json lists an image not listed: 02-b", out)

    def test_missing_labelled_image_stops_the_gallery(self):
        code, out = self.gallery(self.META, self.BOTH[1:], defects={})
        self.assertNotEqual(code, 0, out)
        self.assertIn("image file is missing", out)
        self.assertFalse(os.path.exists(self.path("index.html")))

    def test_missing_clean_image_stops_the_gallery(self):
        code, out = self.gallery(self.META, self.BOTH[:1], defects={})
        self.assertNotEqual(code, 0, out)
        self.assertIn("image file is missing", out)
        self.assertFalse(os.path.exists(self.path("index.html")))

    def test_coverage_of_an_unlisted_image_stops_the_gallery(self):
        meta = dict(
            self.META, coverage=[{"commit": "abc", "subject": "s", "images": ["02-b"]}]
        )
        code, out = self.gallery(meta, self.BOTH, defects={})
        self.assertNotEqual(code, 0, out)
        self.assertIn("not listed", out)


class EnvValue(unittest.TestCase):
    """env-value.awk reads one variable from an env file the way the secret
    helpers need it, and prints nothing for anything it cannot read safely.
    The values here are fake; no real credential is ever used."""

    NAME = "E2E_WALLET_MNEMONIC"

    def value(self, text):
        with tempfile.NamedTemporaryFile(
            "w", suffix=".env", newline="", delete=False
        ) as f:
            f.write(text)
        self.addCleanup(os.unlink, f.name)
        r = subprocess.run(
            [
                "awk",
                "-v",
                f"name={self.NAME}",
                "-f",
                os.path.join(HERE, "env-value.awk"),
                f.name,
            ],
            capture_output=True,
            text=True,
        )
        return r.returncode, r.stdout

    def test_accepted_forms_yield_the_bare_value(self):
        for text in [
            f"{self.NAME}=alpha bravo\n",
            f"{self.NAME}=alpha bravo   \n",
            f"{self.NAME}=alpha bravo # note\n",
            f'{self.NAME}="alpha bravo"\n',
            f'{self.NAME}="alpha bravo"\r\n',
            f"{self.NAME}='alpha bravo'\r\n",
            f'export {self.NAME}="alpha bravo"\n',
            f'{self.NAME}="alpha bravo"   # note\n',
            f"OTHER=x\n{self.NAME}_NOT=y\n{self.NAME}=alpha bravo",
        ]:
            self.assertEqual(self.value(text), (0, "alpha bravo"), repr(text))

    def test_anything_else_fails_and_prints_nothing(self):
        for text in [
            "OTHER=x\n",
            f"{self.NAME}=\n",
            f'{self.NAME}=""\n',
            f'{self.NAME}="alpha bravo\n',
            f'{self.NAME}="alpha bravo" trailing\n',
            f"{self.NAME} = alpha bravo\n",
            f"{self.NAME}= alpha bravo\n",
            f'{self.NAME}= "alpha bravo"\n',
            f"{self.NAME}=\talpha bravo\n",
        ]:
            code, out = self.value(text)
            self.assertNotEqual(code, 0, repr(text))
            self.assertEqual(out, "", repr(text))


if __name__ == "__main__":
    unittest.main()
