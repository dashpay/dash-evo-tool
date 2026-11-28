"""Launching a tutorial must never reuse an unrelated application profile."""

import importlib.util
from pathlib import Path

import pytest

spec = importlib.util.spec_from_file_location(
    "tutorial_launch", Path(__file__).with_name("launch.py")
)
launch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launch)


def test_unmarked_existing_profile_is_refused(tmp_path):
    profile = tmp_path / "profile"
    profile.mkdir()
    (profile / "data.db").touch()
    with pytest.raises(ValueError):
        launch.validate_profile(profile)


def test_only_owned_isolated_profile_can_resume(tmp_path):
    profile = tmp_path / "profile"
    profile.mkdir()
    (profile / ".tutorial-profile").touch()
    launch.validate_profile(profile)
    link = tmp_path / "link"
    link.symlink_to(profile)
    with pytest.raises(ValueError):
        launch.validate_profile(link)


def test_launched_app_environment_is_bound_to_the_x11_session(tmp_path):
    environ = {
        "WAYLAND_DISPLAY": "wayland-0",
        "WAYLAND_SOCKET": "3",
        "MCP_API_KEY": "synthetic",
        "DISPLAY": ":0",
    }
    launch.prepare_environment(
        environ, ":100", tmp_path / "profile", tmp_path / "xauth"
    )
    assert environ["DISPLAY"] == ":100"
    assert environ["XAUTHORITY"] == str(tmp_path / "xauth")
    assert environ["DASH_EVO_DATA_DIR"] == str(tmp_path / "profile")
    assert not {"WAYLAND_DISPLAY", "WAYLAND_SOCKET", "MCP_API_KEY"} & environ.keys()


def test_xvfb_requires_the_session_cookie_and_has_no_tcp(tmp_path):
    command = launch.xvfb_command(":100", tmp_path / "xauth")
    assert command[command.index("-auth") + 1] == str(tmp_path / "xauth")
    assert command[command.index("-nolisten") + 1] == "tcp"


def test_session_cookie_is_private_fresh_and_never_overwritten(tmp_path):
    import shutil
    import stat
    import subprocess

    first, second = tmp_path / "first", tmp_path / "second"
    launch.write_xauthority(first, ":100")
    launch.write_xauthority(second, ":100")
    assert stat.S_IMODE(first.stat().st_mode) == 0o600
    assert first.read_bytes() != second.read_bytes()
    with pytest.raises(FileExistsError):
        launch.write_xauthority(first, ":100")
    if shutil.which("xauth"):
        listing = subprocess.run(
            ["xauth", "-f", str(first), "list"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
        assert listing[0].endswith(":100") and listing[1] == "MIT-MAGIC-COOKIE-1"
        assert len(listing[2]) == 32


def test_inherited_private_bus_marker_cannot_bypass_session(tmp_path, monkeypatch):
    binary = tmp_path / "binary"
    binary.write_bytes(b"reviewed")
    config = tmp_path / "example"
    config.touch()
    monkeypatch.setenv("TUTORIAL_PRIVATE_BUS", "1")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/shared")
    monkeypatch.setattr(
        "sys.argv",
        [
            "launch.py",
            "--binary",
            str(binary),
            "--binary-sha256",
            launch.hashlib.sha256(b"reviewed").hexdigest(),
            "--profile",
            str(tmp_path / "profile"),
            "--config",
            str(config),
            "--evidence",
            str(tmp_path / "evidence"),
            "--display",
            ":98765",
        ],
    )
    calls = []
    monkeypatch.setattr(
        launch, "run_private_session", lambda args: calls.append(args) or 0
    )
    monkeypatch.setattr(
        launch,
        "write_xauthority",
        lambda *_: pytest.fail("Shared bus reached desktop setup"),
    )
    with pytest.raises(SystemExit) as result:
        launch.main()
    assert result.value.code == 0
    assert len(calls) == 1


def test_private_session_handoff_uses_fresh_bus(tmp_path, monkeypatch):
    import json

    child = tmp_path / "child.py"
    result = tmp_path / "result.json"
    child.write_text(
        "import os, sys, json\n"
        "def main(*, private_session=False):\n"
        "    with open(sys.argv[1], 'w') as out:\n"
        "        json.dump([private_session, os.environ.get('DBUS_SESSION_BUS_ADDRESS')], out)\n"
    )
    monkeypatch.setattr(launch, "__file__", str(child))
    monkeypatch.setenv("TUTORIAL_PRIVATE_BUS", "1")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/shared")
    assert launch.run_private_session([str(result)]) == 0
    private, address = json.loads(result.read_text())
    assert private
    assert address and address != "unix:path=/shared"
