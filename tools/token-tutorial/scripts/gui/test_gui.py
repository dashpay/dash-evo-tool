"""Offline lost-session checks; no live app input or screenshots."""

import importlib.util
from pathlib import Path
from types import SimpleNamespace

import pytest


@pytest.fixture
def module():
    spec = importlib.util.spec_from_file_location(
        "tutorial_gui_test", Path(__file__).with_name("gui.py")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Node:
    name = ""

    def __init__(self, *, bus=":1.target", children=(), frame=False):
        self.name = "" if frame else "dash-evo-tool"
        self.app = SimpleNamespace(bus_name=bus)
        self.children = children
        self.frame = frame

    def __iter__(self):
        return iter(self.children)

    def queryComponent(self):
        if not self.frame:
            raise NotImplementedError()
        return SimpleNamespace(getExtents=lambda _: (0, 0, 1920, 1080))

    def getRoleName(self):
        return "frame"


def fake_gui(module, tmp_path, apps):
    gui = module.Gui.__new__(module.Gui)
    gui.spi = SimpleNamespace(
        Registry=SimpleNamespace(getDesktop=lambda _: apps), DESKTOP_COORDS=0
    )
    gui.app_bus_name = ":1.target"
    gui.evidence = tmp_path
    return gui


@pytest.mark.parametrize("apps", [[], [Node(bus=":1.other")], [Node()]])
def test_lost_or_unreadable_app_does_not_satisfy_absence(module, tmp_path, apps):
    gui = fake_gui(module, tmp_path, apps)
    with pytest.raises(RuntimeError):
        gui.wait({"name": "Success"}, timeout=1, absent=True)


@pytest.mark.parametrize("apps", [[], [Node(bus=":1.other")], [Node()]])
def test_lost_or_unreadable_app_cannot_be_screenshotted(
    module, tmp_path, monkeypatch, apps
):
    gui = fake_gui(module, tmp_path, apps)
    calls = []
    monkeypatch.setattr(module, "command", lambda *args: calls.append(args))
    with pytest.raises(RuntimeError):
        gui.screenshot("missing-app")
    assert calls == []


def test_real_absence_with_readable_app_is_allowed(module, tmp_path):
    gui = fake_gui(module, tmp_path, [Node(children=[Node(frame=True)])])
    assert gui.wait({"name": "Gone"}, timeout=1, absent=True) is None


def test_readable_app_allows_screenshot(module, tmp_path, monkeypatch):
    gui = fake_gui(module, tmp_path, [Node(children=[Node(frame=True)])])
    calls = []
    monkeypatch.setattr(module, "command", lambda *args: calls.append(args))
    assert gui.screenshot("present-app") == tmp_path / "present-app.png"
    assert len(calls) == 1
