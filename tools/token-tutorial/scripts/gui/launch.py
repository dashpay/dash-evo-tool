#!/usr/bin/python3
"""Keep an isolated Full HD GUI alive in this foreground process."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time
import xml.etree.ElementTree as ET


def validate_profile(profile):
    if profile.is_symlink():
        raise ValueError("Profile symlinks are refused")
    if profile.exists():
        if profile.stat().st_uid != os.getuid() or not profile.is_dir():
            raise ValueError("Profile must be an owned directory")
        if any(profile.iterdir()) and not (profile / ".tutorial-profile").is_file():
            raise ValueError(
                "Existing non-tutorial profile is refused; choose a fresh private path"
            )


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--display", default=":100")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument(
        "--config",
        type=Path,
        required=True,
        help="Reviewed .env.example, never a live profile",
    )
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Validate inputs without launching or changing files",
    )
    args = parser.parse_args()
    binary = args.binary.resolve()
    profile = args.profile.absolute()
    validate_profile(profile)
    with binary.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != args.binary_sha256:
            raise ValueError("Binary hash differs from the reviewed build")
    if not args.config.is_file():
        raise ValueError("Supply an existing reviewed environment example")
    if args.check:
        print("Launch preflight passed; no GUI or profile was opened")
        return
    if not args.display.startswith(":") or not args.display[1:].isdigit():
        parser.error("Use a local display such as :100")
    if Path(f"/tmp/.X11-unix/X{args.display[1:]}").exists():
        parser.error("Display is occupied; existing displays are never modified")
    os.umask(0o077)
    profile.mkdir(parents=True, mode=0o700, exist_ok=True)
    profile.chmod(0o700)
    (profile / ".tutorial-profile").touch(mode=0o600)
    config = profile / ".env"
    if not config.exists():
        shutil.copyfile(args.config, config)
        config.chmod(0o600)
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    os.environ.update(
        DISPLAY=args.display,
        DBUS_SESSION_BUS_ADDRESS=os.environ.get(
            "DBUS_SESSION_BUS_ADDRESS", f"unix:path=/run/user/{os.getuid()}/bus"
        ),
        DASH_EVO_DATA_DIR=str(profile),
        DASH_EVO_TOOL_ACCESSIBILITY="1",
    )
    os.environ.pop("MCP_API_KEY", None)
    processes = []
    try:
        log = (evidence / "desktop-launch.log").open("a")
        processes.append(
            subprocess.Popen(
                [
                    "Xvfb",
                    args.display,
                    "-screen",
                    "0",
                    "1920x1080x24",
                    "-nolisten",
                    "tcp",
                ],
                stdout=log,
                stderr=log,
            )
        )
        deadline = time.monotonic() + 15
        while subprocess.run(
            ["xdpyinfo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        ).returncode:
            if time.monotonic() > deadline or processes[0].poll() is not None:
                raise RuntimeError("Xvfb did not start")
            time.sleep(0.1)
        wm_config = profile / "openbox.xml"
        tree = ET.parse("/etc/xdg/openbox/rc.xml")
        ns = "{http://openbox.org/3.4/rc}"
        apps = tree.getroot().find(f"{ns}applications")
        app_rule = ET.SubElement(
            apps, f"{ns}application", {"class": "org.dash.DashEvoTool"}
        )
        # TODO: Manually verify fullscreen on each target desktop before recording.
        ET.SubElement(app_rule, f"{ns}fullscreen").text = "yes"
        tree.write(wm_config)
        processes.append(
            subprocess.Popen(
                ["openbox", "--sm-disable", "--config-file", str(wm_config)],
                stdout=log,
                stderr=log,
            )
        )
        for prop in ("ScreenReaderEnabled", "IsEnabled"):
            run(
                "gdbus",
                "call",
                "--session",
                "--dest",
                "org.a11y.Bus",
                "--object-path",
                "/org/a11y/bus",
                "--method",
                "org.freedesktop.DBus.Properties.Set",
                "org.a11y.Status",
                prop,
                "<true>",
            )
        import pyatspi

        existing = {app.app.bus_name for app in pyatspi.Registry.getDesktop(0)}
        app = subprocess.Popen([str(binary)], stdout=log, stderr=log)
        processes.append(app)
        info = {
            "display": args.display,
            "dbus": os.environ["DBUS_SESSION_BUS_ADDRESS"],
            "app_pid": app.pid,
            "launcher_pid": os.getpid(),
            "profile": str(profile),
            "binary": str(binary),
            "sha256": args.binary_sha256,
            "capture": [0, 0, 1920, 1080],
        }
        (evidence / "session.json").write_text(json.dumps(info, indent=2) + "\n")
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline and app.poll() is None:
            windows = subprocess.run(
                ["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)],
                capture_output=True,
                text=True,
            )
            if windows.returncode == 0:
                window = windows.stdout.splitlines()[0]
                run("xdotool", "windowactivate", "--sync", window)
                new = [
                    a
                    for a in pyatspi.Registry.getDesktop(0)
                    if a.name == "dash-evo-tool" and a.app.bus_name not in existing
                ]
                if len(new) == 1:
                    info.update(
                        window_id=window,
                        atspi_bus_name=new[0].app.bus_name,
                        atspi_host_pid=new[0].get_process_id(),
                    )
                    break
            time.sleep(0.15)
        if "window_id" not in info:
            raise RuntimeError(
                "App window or accessibility registration did not become ready"
            )
        (evidence / "session.json").write_text(json.dumps(info, indent=2) + "\n")
        print(json.dumps(info), flush=True)
        app.wait()
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: exit(0))
    main()
