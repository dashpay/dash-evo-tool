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

    def get_process_id(self):
        return 4242


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


def session(tmp_path, **changes):
    import json
    import os

    data = {
        "display": ":100",
        "dbus": "unix:path=/synthetic/bus",
        "xauthority": "/synthetic/xauthority",
        "app_pid": os.getpid(),
        "window_id": "12345",
        "atspi_bus_name": ":1.target",
        "atspi_host_pid": 4242,
    }
    data.update(changes)
    path = tmp_path / "session.json"
    path.write_text(json.dumps({k: v for k, v in data.items() if v is not None}))
    return path


@pytest.fixture
def desktop(module, monkeypatch):
    import os
    import sys

    calls, apps = [], [Node(children=[Node(frame=True)])]
    monkeypatch.setitem(
        sys.modules,
        "pyatspi",
        SimpleNamespace(Registry=SimpleNamespace(getDesktop=lambda _: apps)),
    )
    for name in ("DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "XAUTHORITY"):
        monkeypatch.setenv(name, os.environ.get(name, ""))
    for name in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET"):
        monkeypatch.delenv(name, raising=False)

    def command(*args):
        calls.append(args)
        return str(os.getpid()) if args[1] == "getwindowpid" else ""

    monkeypatch.setattr(module, "command", command)
    return calls, apps


@pytest.mark.parametrize(
    "missing", ["window_id", "atspi_bus_name", "app_pid", "xauthority"]
)
def test_incomplete_session_manifest_is_refused_before_input(
    module, tmp_path, desktop, missing
):
    with pytest.raises(RuntimeError, match="incomplete"):
        module.Gui(tmp_path, session(tmp_path, **{missing: None}))
    assert desktop[0] == []


def test_window_of_another_process_is_never_activated(
    module, tmp_path, desktop, monkeypatch
):
    calls, _ = desktop
    monkeypatch.setattr(module, "command", lambda *args: calls.append(args) or "1")
    with pytest.raises(RuntimeError, match="window"):
        module.Gui(tmp_path, session(tmp_path))
    assert not any("windowactivate" in call for call in calls)


def test_registration_of_another_process_is_never_used(module, tmp_path, desktop):
    calls, _ = desktop
    with pytest.raises(RuntimeError, match="accessibility"):
        module.Gui(tmp_path, session(tmp_path, atspi_host_pid=1))
    assert not any("windowactivate" in call for call in calls)


def test_verified_session_activates_only_the_recorded_window(module, tmp_path, desktop):
    calls, _ = desktop
    module.Gui(tmp_path, session(tmp_path))
    assert ("xdotool", "windowactivate", "--sync", "12345") in calls
    assert module.os.environ["XAUTHORITY"] == "/synthetic/xauthority"
    assert not any("search" in call for call in calls)


class Element:
    def __init__(self, name, role, children=()):
        self.name, self.role, self.children = name, role, list(children)

    def __iter__(self):
        return iter(self.children)

    def queryComponent(self):
        return SimpleNamespace(getExtents=lambda _: (10, 10, 200, 40))

    def getRoleName(self):
        return self.role


def screen(*elements):
    return Node(children=[Element("", "frame", elements)])


@pytest.mark.parametrize(
    "element",
    [Element("", "password text"), Element("Enter the wallet password", "label")],
)
def test_credential_prompt_blocks_screenshots(module, tmp_path, monkeypatch, element):
    gui = fake_gui(module, tmp_path, [screen(element)])
    calls = []
    monkeypatch.setattr(module, "command", lambda *args: calls.append(args))
    with pytest.raises(RuntimeError, match="Sensitive"):
        gui.screenshot("prompt")
    assert calls == []


class Recorder:
    started = []

    def __init__(self, args, **_):
        self.path = Path(args[-1])
        self.path.write_bytes(b"synthetic frames")
        self.returncode = None
        Recorder.started.append(self.path)

    def poll(self):
        return self.returncode

    def send_signal(self, _):
        self.returncode = 255

    def wait(self, timeout=None):
        return self.returncode


@pytest.fixture
def recording(module, tmp_path, monkeypatch):
    Recorder.started = []
    tree = screen(Element("Ready", "label"))
    gui = fake_gui(module, tmp_path, [tree])
    gui.display = ":100"
    monkeypatch.setattr(module, "command", lambda *args: None)
    monkeypatch.setattr(module.subprocess, "Popen", Recorder)
    return gui, tree, tmp_path / "captures" / "03.mkv"


def test_recorded_scene_cannot_click_into_a_credential_prompt(recording):
    gui, _, output = recording
    output.parent.mkdir()
    spec = {"steps": [{"action": "click", "target": {"name": "Unlock Wallet"}}]}
    with pytest.raises(ValueError, match="credential"):
        gui.scene(spec, output)
    assert Recorder.started == []


def test_prompt_during_hold_stops_and_discards_the_clip(module, recording, monkeypatch):
    gui, tree, output = recording
    output.parent.mkdir()
    monkeypatch.setattr(
        module.time,
        "sleep",
        lambda _: tree.children[0].children.append(Element("", "password text")),
    )
    with pytest.raises(RuntimeError, match="Sensitive"):
        gui.scene({"steps": [{"action": "hold", "seconds": 2}]}, output)
    assert not output.exists()
    assert not any(path.exists() for path in Recorder.started)


def test_clip_appears_at_its_path_only_after_a_verified_scene(recording):
    gui, _, output = recording
    output.parent.mkdir()
    gui.scene({"steps": [{"action": "wait", "target": {"name": "Ready"}}]}, output)
    assert Recorder.started and Recorder.started[0] != output
    assert output.read_bytes() == b"synthetic frames"
    assert not Recorder.started[0].exists()


def test_interrupted_clip_stays_out_of_the_media_inputs(recording):
    gui, _, output = recording
    output.parent.mkdir()
    with pytest.raises(ValueError):
        gui.scene({"steps": [{"action": "unknown"}]}, output)
    assert not output.exists()
    assert Recorder.started[0].exists()
    assert Recorder.started[0].name == "03.incomplete.mkv"
