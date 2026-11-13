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
