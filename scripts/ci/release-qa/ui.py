#!/usr/bin/python3
"""Narrow GUI command surface; no shell, network, wallet or arbitrary Python execution."""

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from logic import CAPTURE_NAME, has_secret

ROOT = Path(os.environ["QA_ROOT"])
OUT = ROOT / "report"


def run(*args):
    subprocess.run(args, check=True, timeout=60)


def name(value):
    if not re.fullmatch(CAPTURE_NAME, value):
        raise ValueError("Expected a capture name, without an extension")
    return value


def tree():
    import pyatspi

    lines = []

    def walk(node, depth=0):
        if depth > 30:
            return
        rect = ""
        try:
            rect = str(node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS))
        except NotImplementedError:
            pass
        lines.append("  " * depth + f"{node.getRoleName()}: {node.name} {rect}")
        for child in node:
            if child is not None:
                walk(child, depth + 1)

    for app in pyatspi.Registry.getDesktop(0):
        if app.name == "dash-evo-tool":
            walk(app)
    text = "\n".join(lines)
    if has_secret(text):
        raise ValueError("Accessibility output withheld: suspected credential text")
    print(text)


def main():
    command, *args = sys.argv[1:]
    wid = (ROOT / "window").read_text().strip()
    if command == "click" and len(args) == 2:
        x, y = map(int, args)
        if not (0 <= x < 1280 and 0 <= y < 800):
            raise ValueError("Coordinates outside the QA display")
        run(
            "xdotool",
            "windowactivate",
            "--sync",
            wid,
            "mousemove",
            str(x),
            str(y),
            "click",
            "1",
        )
    elif (
        command == "key"
        and len(args) == 1
        and args[0]
        in (
            "Tab",
            "shift+Tab",
            "Return",
            "Escape",
            "space",
            "ctrl+a",
            "BackSpace",
            "Delete",
            "Home",
            "End",
            "Left",
            "Right",
            "Up",
            "Down",
            "Page_Up",
            "Page_Down",
        )
    ):
        run(
            "xdotool",
            "windowactivate",
            "--sync",
            wid,
            "key",
            "--clearmodifiers",
            args[0],
        )
    elif command == "type" and len(args) == 1 and len(args[0]) <= 256:
        run(
            "xdotool",
            "windowactivate",
            "--sync",
            wid,
            "type",
            "--clearmodifiers",
            "--",
            args[0],
        )
    elif command == "scroll" and args in (["up"], ["down"]):
        run("xdotool", "click", "4" if args[0] == "up" else "5")
    elif command == "wait" and len(args) == 1 and 0 <= int(args[0]) <= 30:
        time.sleep(int(args[0]))
    elif command == "capture" and len(args) == 1:
        if args[0] == "00-smoke":
            raise ValueError("The smoke screenshot is reserved")
        path = OUT / "new" / (name(args[0]) + ".png")
        run("scrot", "-o", str(path))
        print(path)
    elif command == "tree" and not args:
        tree()
    elif command == "gallery" and not args:
        spec = json.loads((OUT / "spec.json").read_text())
        for item in spec:
            name(item)
        meta = json.loads((OUT / "meta.json").read_text())
        for item in meta["images"]:
            name(item["name"])
        scripts = Path("/opt/release-qa/visual")
        run(
            "/usr/bin/python3",
            str(scripts / "render.py"),
            str(OUT / "spec.json"),
            "--new-dir",
            str(OUT / "new"),
            "--out-dir",
            str(OUT / "annotated"),
        )
        run(
            "/usr/bin/python3",
            str(scripts / "gallery.py"),
            "--root",
            str(OUT),
            "--meta",
            str(OUT / "meta.json"),
            "--title",
            "Release GUI review (NEW only)",
        )
    else:
        raise ValueError("Unsupported GUI command")


if __name__ == "__main__":
    main()
