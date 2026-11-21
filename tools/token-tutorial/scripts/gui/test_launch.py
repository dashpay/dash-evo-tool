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
