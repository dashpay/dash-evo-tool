#!/usr/bin/env python3
"""Regression tests for render.py, diff.py and gallery.py on synthetic images,
and for env-value.awk on throwaway env files.

usage: selftest.py
Needs no display, no application build and no captured data. Run it after
changing any script in this directory.
"""

import json
import base64
import re
from pathlib import Path
import os
import shutil
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
        self.assertTrue(os.path.exists(self.path("index.html")))
        text = Path(self.path("index.html")).read_text()
        self.assertIn(
            "<details><summary>Version without labels</summary>"
            '<img src="annotated/clean/01-a.png" alt="A"></details>',
            text,
        )
        self.assertNotIn("Known issues", self.index())

    def test_embedded_gallery_keeps_style_charset_and_both_images(self):
        name = "01-a&b"
        files = [("annotated", name + ".png"), ("annotated", "clean", name + ".png")]
        code, out = self.gallery(
            {"images": [{"name": name, "title": "A"}], "coverage": []},
            files,
            defects={},
        )
        self.assertEqual(code, 0, out)
        image(self.path(*files[1]), (RECT, (255, 0, 0)))
        code, out = run(
            "gallery.py",
            "--root",
            self.root,
            "--meta",
            self.path("meta.json"),
            "--embed",
        )
        self.assertEqual(code, 0, out)
        with open(self.path("index.html")) as page:
            text = page.read()
        self.assertIn("<meta charset=utf-8>", text)
        self.assertIn("img{max-width:100%", text)
        images = re.findall(r'<img src="data:image/png;base64,([^"]+)"', text)
        self.assertEqual(len(images), 2)
        for encoded, file in zip(images, files):
            self.assertEqual(
                base64.b64decode(encoded), Path(self.path(*file)).read_bytes()
            )
        self.assertRegex(
            text,
            r"<details><summary>Version without labels</summary>"
            r'<img src="data:image/png;base64,[^"]+" alt="A"></details>',
        )
        self.assertNotIn('href="data:', text)
        self.assertNotIn('src="annotated/', text)

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
        with open(f.name) as stdin:
            r = subprocess.run(
                [
                    "awk",
                    "-v",
                    f"name={self.NAME}",
                    "-f",
                    os.path.join(HERE, "env-value.awk"),
                ],
                stdin=stdin,
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
            f"{self.NAME}=alpha\\ bravo\n",
            f"{self.NAME}=alpha bravo\\\n",
            f'{self.NAME}=alpha "bravo\n',
            f"{self.NAME}=alpha bravo'\n",
        ]:
            code, out = self.value(text)
            self.assertNotEqual(code, 0, repr(text))
            self.assertEqual(out, "", repr(text))


class ProvisionWallet(unittest.TestCase):
    """provision-wallet.sh pipes the phrase to the det-cli it checked, read
    from the env file it was given. A fake det-cli records what it receives."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir)
        self.received = os.path.join(self.dir, "received")
        fake = os.path.join(self.dir, "det-cli")
        with open(fake, "w") as f:
            f.write(
                '#!/bin/sh\ncase "$*" in *core-wallet-import*) cat >"$RECEIVED" ;; esac\n'
            )
        os.chmod(fake, 0o700)
        os.mkdir(os.path.join(self.dir, "data"))

    def provision(self, det_cli, env_file):
        with open(os.path.join(self.dir, env_file), "w") as f:
            f.write("E2E_WALLET_MNEMONIC=alpha bravo\n")
        r = subprocess.run(
            [
                os.path.join(HERE, "provision-wallet.sh"),
                det_cli,
                "data",
                "--env-file",
                env_file,
            ],
            cwd=self.dir,
            env={"PATH": "/usr/bin:/bin", "RECEIVED": self.received},
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        with open(self.received) as f:
            return f.read()

    def test_a_bare_det_cli_name_runs_the_checked_file(self):
        self.assertEqual(self.provision("det-cli", "test.env"), "alpha bravo")

    def test_an_option_shaped_env_file_is_read_as_a_file(self):
        self.assertEqual(self.provision("./det-cli", "--version"), "alpha bravo")


# Window 42 has the focus until the SWITCH_AT-th getactivewindow call, then
# window 99, which belongs to no process. `type` appends its input to typed.
FAKE_XDOTOOL = """#!/bin/sh
n=$(cat "$STATE/calls" 2>/dev/null || echo 0)
case "$1" in
  getactivewindow)
    n=$((n + 1)); echo "$n" >"$STATE/calls"
    wid=42; [ -n "$SWITCH_AT" ] && [ "$n" -ge "$SWITCH_AT" ] && wid=99
    if [ "$2" = getwindowpid ]; then [ "$wid" = 42 ] && echo "$FOCUS_PID"; else echo "$wid"; fi ;;
  getwindowpid) [ "$2" = 42 ] && echo "$FOCUS_PID" ;;
  type) cat >>"$STATE/typed" ;;
esac
"""


class TypeSecret(unittest.TestCase):
    """type-secret.sh types only into a window of a dash-evo-tool process, and
    stops when the focus leaves it. A fake xdotool stands in for the display."""

    VALUE = "alpha bravo"

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir)
        bin_dir = os.path.join(self.dir, "bin")
        os.mkdir(bin_dir)
        self.write(os.path.join(bin_dir, "xdotool"), FAKE_XDOTOOL)
        # Its process name (comm) is the script's file name: dash-evo-tool.
        gui = os.path.join(self.dir, "dash-evo-tool")
        self.write(gui, "#!/bin/sh\nread -r _\n")
        self.gui = subprocess.Popen([gui], stdin=subprocess.PIPE)
        self.addCleanup(self.gui.wait)
        self.addCleanup(self.gui.stdin.close)
        self.path = f"{bin_dir}:/usr/bin:/bin"

    def write(self, path, text):
        with open(path, "w") as f:
            f.write(text)
        os.chmod(path, 0o700)

    def type_secret(
        self, focus_pid, *args, env_file="test.env", switch_at="", extra_env=None
    ):
        with open(os.path.join(self.dir, env_file), "w") as f:
            f.write(f"E2E_WALLET_MNEMONIC={self.VALUE}\n")
        r = subprocess.run(
            [
                os.path.join(HERE, "type-secret.sh"),
                "--env-file",
                env_file,
                "E2E_WALLET_MNEMONIC",
                *args,
            ],
            cwd=self.dir,
            env={
                "PATH": self.path,
                "DISPLAY": ":fake",
                "STATE": self.dir,
                "FOCUS_PID": focus_pid,
                "SWITCH_AT": str(switch_at),
                **(extra_env or {}),
            },
            capture_output=True,
            text=True,
        )
        self.output = r.stdout + r.stderr
        typed = os.path.join(self.dir, "typed")
        if not os.path.exists(typed):
            return r.returncode, ""
        with open(typed) as f:
            return r.returncode, f.read()

    def test_the_value_is_typed_into_the_gui_under_test(self):
        self.assertEqual(self.type_secret(str(self.gui.pid)), (0, self.VALUE))
        os.unlink(os.path.join(self.dir, "typed"))
        self.assertEqual(self.type_secret(str(self.gui.pid), "2"), (0, "bravo"))

    def test_a_window_of_another_program_gets_nothing(self):
        self.assertEqual(self.type_secret(str(os.getpid())), (1, ""))

    def test_a_window_without_a_process_gets_nothing(self):
        self.assertEqual(self.type_secret(""), (1, ""))

    def test_typing_stops_when_the_focus_moves(self):
        code, typed = self.type_secret(str(self.gui.pid), switch_at=4)
        self.assertEqual(code, 1)
        self.assertTrue(
            self.VALUE.startswith(typed) and len(typed) < len(self.VALUE), repr(typed)
        )

    def test_an_option_shaped_env_file_is_read_as_a_file(self):
        self.assertEqual(
            self.type_secret(str(self.gui.pid), env_file="--version"), (0, self.VALUE)
        )

    def test_an_inherited_trace_never_prints_the_value(self):
        bash_env = os.path.join(self.dir, "trace.sh")
        self.write(bash_env, "set -x\n")
        for extra_env in [{"SHELLOPTS": "xtrace"}, {"BASH_ENV": bash_env}]:
            code, typed = self.type_secret(str(self.gui.pid), extra_env=extra_env)
            os.unlink(os.path.join(self.dir, "typed"))
            self.assertEqual((code, typed), (0, self.VALUE), extra_env)
            self.assertNotIn("bravo", self.output, extra_env)
            traced = [line for line in self.output.splitlines() if line.startswith("+")]
            self.assertEqual(traced, [], extra_env)


if __name__ == "__main__":
    unittest.main()
