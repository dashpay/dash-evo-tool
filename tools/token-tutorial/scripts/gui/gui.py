#!/usr/bin/python3
"""Semantic GUI inspection/actions and opt-in state-driven scene recording."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
SESSION = ROOT / "local/evidence/session.json"
REQUIRED_SESSION = (
    "display",
    "dbus",
    "xauthority",
    "app_pid",
    "window_id",
    "atspi_bus_name",
    "atspi_host_pid",
)
SENSITIVE_WORDS = (
    "recovery phrase",
    "seed phrase",
    "mnemonic",
    "private key",
    "password",
    "passphrase",
)
# Clicks that open credential prompts must happen while recording is stopped.
CREDENTIAL_TRIGGERS = SENSITIVE_WORDS + ("unlock",)
# Lets a dialog opened by the last action reach the accessibility tree.
STOP_SETTLE_SECONDS = 0.5


class SensitiveScreen(RuntimeError):
    """A credential prompt may be visible; nothing may be captured."""


def command(*args):
    return subprocess.check_output([str(a) for a in args], text=True).strip()


class Gui:
    recording = False

    def __init__(self, evidence, session_path=SESSION):
        session = json.loads(Path(session_path).read_text())
        if any(not session.get(key) for key in REQUIRED_SESSION):
            raise RuntimeError(
                "Session manifest is incomplete; wait until the launcher reports the app ready"
            )
        try:
            os.kill(session["app_pid"], 0)
        except OSError:
            raise RuntimeError("The recorded app process is not running") from None
        os.environ.update(
            DISPLAY=session["display"],
            DBUS_SESSION_BUS_ADDRESS=session["dbus"],
            XAUTHORITY=session["xauthority"],
        )
        for name in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET"):
            os.environ.pop(name, None)
        import pyatspi

        self.spi = pyatspi
        self.app_bus_name = session["atspi_bus_name"]
        self.evidence = Path(evidence)
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.window = session["window_id"]
        self.display = session["display"]
        if command("xdotool", "getwindowpid", self.window) != str(session["app_pid"]):
            raise RuntimeError(
                "The recorded window does not belong to the recorded app"
            )
        apps = [
            app
            for app in self.spi.Registry.getDesktop(0)
            if app.name == "dash-evo-tool" and app.app.bus_name == self.app_bus_name
        ]
        if len(apps) != 1 or apps[0].get_process_id() != session["atspi_host_pid"]:
            raise RuntimeError(
                "The recorded accessibility registration is unavailable or reused"
            )
        command("xdotool", "windowactivate", "--sync", self.window)

    def scan(self):
        """Return visible non-entry nodes and whether a password field may be visible.

        A node whose role or name cannot be read, or a password field whose extents
        cannot be read, counts as a visible password field.
        """

        def walk(node):
            yield node
            for child in node:
                if child is not None:
                    yield from walk(child)

        apps = [
            app
            for app in self.spi.Registry.getDesktop(0)
            if app.name == "dash-evo-tool" and app.app.bus_name == self.app_bus_name
        ]
        if len(apps) != 1:
            raise RuntimeError(
                "The recorded app accessibility registration is unavailable"
            )
        nodes = []
        has_frame = secret_field = False
        for node in walk(apps[0]):
            try:
                role, name = node.getRoleName(), node.name
            except (NotImplementedError, RuntimeError):
                secret_field = True
                continue
            try:
                bounds = tuple(
                    node.queryComponent().getExtents(self.spi.DESKTOP_COORDS)
                )
            except (NotImplementedError, RuntimeError):
                secret_field = secret_field or role == "password text"
                continue
            visible = bounds[2] > 0 and bounds[3] > 0
            if role in ("text", "password text", "entry"):
                secret_field = secret_field or (visible and role == "password text")
                continue
            if visible:
                has_frame = has_frame or role == "frame"
                nodes.append((node, {"name": name, "role": role, "bounds": bounds}))
        if not has_frame:
            raise RuntimeError(
                "The app accessibility tree has no readable window frame"
            )
        return nodes, secret_field

    def nodes(self):
        yield from self.scan()[0]

    def guard(self):
        """Refuse a screen that may show a credential prompt; field values are never read."""
        nodes, secret_field = self.scan()
        if secret_field or any(
            word in (data["name"] or "").lower()
            for _, data in nodes
            for word in SENSITIVE_WORDS
        ):
            raise SensitiveScreen(
                "Sensitive form detected; screenshots and recorded scenes must stay paused"
            )

    def match(self, target):
        return [
            (node, data)
            for node, data in self.nodes()
            if data["name"] == target["name"]
            and (not target.get("role") or data["role"] == target["role"])
        ]

    def wait(self, target, timeout=30, absent=False):
        deadline = time.monotonic() + timeout
        previous = None
        while time.monotonic() < deadline:
            if self.recording:
                self.guard()
            found = self.match(target)
            if absent and not found:
                if previous == "absent":
                    return None
                previous = "absent"
            if not absent and len(found) == 1:
                node, data = found[0]
                if previous == data:
                    return node, data
                previous = data
            elif len(found) > 1:
                raise RuntimeError("Ambiguous target; specify an exact role")
            elif found or not absent:
                previous = None
            time.sleep(0.15)
        raise TimeoutError(f"Target condition not met: {target}")

    def screenshot(self, name):
        if Path(name).name != name:
            raise ValueError("Screenshot name must be a filename without directories")
        self.guard()
        path = self.evidence / f"{name}.png"
        command("scrot", str(path))
        return path

    def click(self, target, timeout=30):
        node, data = self.wait(target, timeout)
        if not node.getState().contains(self.spi.STATE_ENABLED):
            raise RuntimeError("Target is disabled")
        x, y, width, height = data["bounds"]
        if x < 0 or y < 0 or x + width > 1920 or y + height > 1080:
            raise RuntimeError(
                "Target lies outside the Full HD capture; scroll or resize first"
            )
        stamp = f"action-{time.time_ns()}"
        self.screenshot(stamp + "-before")
        # Re-resolve after the screenshot; transitions invalidate old coordinates.
        _, fresh = self.wait(target, timeout)
        if fresh != data:
            raise RuntimeError(
                "Target moved during screenshot validation; inspect and retry"
            )
        (self.evidence / f"{stamp}.json").write_text(json.dumps(data, indent=2) + "\n")
        command("xdotool", "mousemove", x + width // 2, y + height // 2)
        command("xdotool", "mousedown", "1")
        time.sleep(0.12)
        command("xdotool", "mouseup", "1")

    def scene(self, spec, output=None):
        recorder = partial = None
        if output:
            output = Path(output)
            if any(
                step["action"] == "click"
                and any(
                    word in step["target"]["name"].lower()
                    for word in CREDENTIAL_TRIGGERS
                )
                for step in spec["steps"]
            ):
                raise ValueError(
                    "A recorded scene cannot open a credential prompt; stop recording, "
                    "unlock or import, then record the next scene"
                )
            self.screenshot(f"recording-preflight-{time.time_ns()}")
            # Only a verified, completed scene is published at the requested path.
            partial = output.with_name(f"{output.stem}.incomplete{output.suffix}")
            if output.exists() or partial.exists():
                raise FileExistsError(output)
            self.recording = True
            recorder = subprocess.Popen(
                [
                    "ffmpeg",
                    "-nostdin",
                    "-n",
                    "-loglevel",
                    "warning",
                    "-f",
                    "x11grab",
                    "-draw_mouse",
                    "1",
                    "-framerate",
                    "30",
                    "-video_size",
                    "1920x1080",
                    "-i",
                    f"{self.display}.0+0,0",
                    "-c:v",
                    "libx264",
                    "-threads",
                    "2",
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "18",
                    "-pix_fmt",
                    "yuv420p",
                    str(partial),
                ]
            )
        sensitive = False
        try:
            for step in spec["steps"]:
                if recorder and recorder.poll() is not None:
                    raise RuntimeError("Recorder stopped before the scene completed")
                action = step["action"]
                if action == "click":
                    self.click(step["target"], step.get("timeout", 30))
                elif action in ("wait", "wait_absent"):
                    self.wait(
                        step["target"], step.get("timeout", 30), action == "wait_absent"
                    )
                elif action == "screenshot":
                    self.screenshot(step["name"])
                elif action == "hold":
                    end = time.monotonic() + min(max(float(step["seconds"]), 0), 60)
                    while time.monotonic() < end:
                        if recorder:
                            self.guard()
                        time.sleep(min(0.15, max(0.0, end - time.monotonic())))
                else:
                    raise ValueError(f"Unknown action: {action}")
                if recorder:
                    self.guard()
        except SensitiveScreen:
            sensitive = True
            raise
        finally:
            self.recording = False
            if recorder and recorder.poll() is None:
                recorder.send_signal(signal.SIGINT)
                recorder.wait(timeout=20)
            if sensitive and partial:
                # The clip may show a credential prompt; it is never kept.
                partial.unlink(missing_ok=True)
        if recorder and recorder.returncode not in (0, 255):
            raise RuntimeError("Recording failed")
        if partial:
            # Prompts persist: re-check after capture stops, before publishing.
            time.sleep(STOP_SETTLE_SECONDS)
            try:
                self.guard()
            except SensitiveScreen:
                partial.unlink(missing_ok=True)
                raise
            os.link(partial, output)
            partial.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", default=str(ROOT / "local/evidence"))
    parser.add_argument("--session", type=Path, default=SESSION)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser(
        "list", help="List semantic labels/roles/bounds; input values are omitted"
    )
    for name in ("wait", "click"):
        cmd = sub.add_parser(name)
        cmd.add_argument("name")
        cmd.add_argument("--role")
        cmd.add_argument("--timeout", type=float, default=30)
        if name == "click":
            cmd.add_argument(
                "--execute", action="store_true", help="Required to send input"
            )
    screen = sub.add_parser("screenshot")
    screen.add_argument("name")
    scene = sub.add_parser(
        "scene", help="Without --execute, only validate and print the scene"
    )
    scene.add_argument("file", type=Path)
    scene.add_argument("--execute", action="store_true")
    scene.add_argument(
        "--record", type=Path, help="Opt-in recording output; never overwritten"
    )
    args = parser.parse_args()
    if args.command == "scene":
        spec = json.loads(args.file.read_text())
        for step in spec["steps"]:
            if step["action"] not in (
                "click",
                "wait",
                "wait_absent",
                "screenshot",
                "hold",
            ):
                parser.error("Scene contains an unsupported action")
            if step["action"] in ("click", "wait", "wait_absent"):
                if not isinstance(step.get("target", {}).get("name"), str):
                    parser.error("Semantic targets require an exact name")
        if not args.execute:
            print(json.dumps(spec, indent=2))
            return
    gui = Gui(args.evidence, args.session)
    if args.command == "list":
        print(json.dumps([data for _, data in gui.nodes()], indent=2))
    elif args.command in ("wait", "click"):
        target = {"name": args.name, "role": args.role}
        if args.command == "click" and args.execute:
            gui.click(target, args.timeout)
        else:
            print(json.dumps(gui.wait(target, args.timeout)[1]))
    elif args.command == "screenshot":
        print(gui.screenshot(args.name))
    elif args.command == "scene":
        gui.scene(spec, args.record)


if __name__ == "__main__":
    main()
