"""Slide module import must not parse caller arguments, write or launch a browser."""

import argparse
import importlib.util
from pathlib import Path
import subprocess
import sys

import pytest


def test_slide_generator_import_has_no_side_effects(monkeypatch):
    monkeypatch.setattr(sys, "argv", ["unrelated-host-program", "--unknown-argument"])
    monkeypatch.setattr(
        argparse.ArgumentParser,
        "parse_args",
        lambda *_: pytest.fail("Parsed arguments on import"),
    )
    monkeypatch.setattr(
        Path, "mkdir", lambda *_, **__: pytest.fail("Created a directory on import")
    )
    monkeypatch.setattr(
        subprocess, "run", lambda *_, **__: pytest.fail("Launched browser on import")
    )
    path = Path(__file__).resolve().parents[1] / "assets/slides/render-slides.py"
    spec = importlib.util.spec_from_file_location("tutorial_slides", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert callable(module.main)
