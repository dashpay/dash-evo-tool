"""Trusted stage controller. Evidence is separate from Claude-writable reports."""

import hashlib
import shutil
import json
import os
import re
import signal
import subprocess
import sys
import time
import zipfile
from pathlib import Path

from PIL import Image, ImageStat
from logic import CAPTURE_NAME, isolated_env, verdict, has_secret
from passive_html import sanitize_html

VISUAL = Path(__file__).resolve().parent / "visual"
if not VISUAL.exists():
    VISUAL = Path(__file__).resolve().parents[3] / ".claude/skills/visual-diff/scripts"
sys.path.insert(0, str(VISUAL))
from gallery import CSS, embed_images  # noqa: E402 - installed and checkout paths differ

ROOT = Path(
    sys.argv[2]
    if len(sys.argv) > 2 and sys.argv[1] == "monitor"
    else os.environ["QA_ROOT"]
)
OUT = ROOT / "report"


def read(path, default=None):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return default


def receipt(**values):
    state = read(ROOT / "receipt.json", {})
    state.update(values)
    (ROOT / "receipt.json").write_text(json.dumps(state))


def env():
    result = isolated_env(ROOT, display=True)
    result.update(read(ROOT / "display.json", {}))
    return result


def launch(args, log):
    with (ROOT / log).open("wb") as output:
        process = subprocess.Popen(
            args,
            env=env(),
            cwd=ROOT,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    (ROOT / (log + ".pid")).write_text(str(process.pid))
    return process


def init():
    for directory in (ROOT, OUT, OUT / "new", ROOT / "data", ROOT / "home"):
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        directory.chmod(0o700)
    receipt(smoke=None, agent_ok=False)
    with zipfile.ZipFile(ROOT / "dash-evo-tool-linux-x86_64.zip") as archive:
        for member in archive.infolist():
            path = Path(member.filename)
            if (
                path.is_absolute()
                or ".." in path.parts
                or (member.external_attr >> 16) & 0o170000 == 0o120000
            ):
                raise ValueError("Unsafe binary archive")
        archive.extractall(ROOT / "app")
    for binary in ("dash-evo-tool", "det-cli"):
        (ROOT / "app/dash-evo-tool" / binary).chmod(0o700)
    actual = subprocess.check_output(
        [str(ROOT / "app/dash-evo-tool/det-cli"), "--version"],
        env=env(),
        cwd=ROOT,
        text=True,
        timeout=30,
    ).strip()
    inputs = read(ROOT / "context/inputs.json")
    if (
        actual.split("+", 1)[0]
        != "det-cli " + inputs["new_tag"].removeprefix("v").split("+", 1)[0]
    ):
        raise ValueError("Release binary version mismatch")


def smoke():
    from wallet import det

    receipt(smoke=None)
    det("network-switch", "network=testnet")
    if det("network-info").get("active") != "testnet":
        raise RuntimeError("Refusing to launch a non-testnet profile")
    bus = subprocess.check_output(
        ["dbus-daemon", "--session", "--fork", "--print-address=1", "--print-pid=1"],
        env=env(),
        text=True,
    ).splitlines()
    (ROOT / "display.json").write_text(json.dumps({"DBUS_SESSION_BUS_ADDRESS": bus[0]}))
    (ROOT / "dbus.log.pid").write_text(bus[1])
    launch(
        ["Xvfb", ":99", "-screen", "0", "1280x800x24", "-nolisten", "tcp"], "xvfb.log"
    )
    for _ in range(50):
        if subprocess.run(["xdpyinfo"], env=env(), capture_output=True).returncode == 0:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError("Xvfb did not become ready")
    launch(["openbox"], "openbox.log")
    for flag in ("ScreenReaderEnabled", "IsEnabled"):
        subprocess.run(
            [
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
                flag,
                "<true>",
            ],
            env=env(),
            check=True,
            timeout=30,
        )
    resume()


def monitor():
    app = subprocess.Popen(
        [str(ROOT / "app/dash-evo-tool/dash-evo-tool")], env=env(), cwd=ROOT
    )
    (ROOT / "gui.pid").write_text(str(app.pid))
    code = app.wait()
    (ROOT / "gui-exit.json").write_text(
        json.dumps({"code": code, "controlled": (ROOT / "gui-stopping").exists()})
    )


def resume():
    from wallet import det

    if read(ROOT / "receipt.json", {}).get("smoke") is False:
        raise RuntimeError("An earlier GUI crash invalidated this run")
    if det("network-info").get("active") != "testnet":
        raise RuntimeError("Refusing to launch a non-testnet profile")
    for name in ("gui.pid", "gui-exit.json", "gui-stopping"):
        (ROOT / name).unlink(missing_ok=True)
    receipt(smoke=None, gui_stopped=False)
    app = launch(
        [sys.executable, str(Path(__file__).resolve()), "monitor", str(ROOT)],
        "launch.log",
    )
    for _ in range(100):
        if (ROOT / "gui.pid").exists():
            break
        if app.poll() is not None:
            raise RuntimeError("GUI monitor did not start")
        time.sleep(0.1)
    pid = int((ROOT / "gui.pid").read_text())
    window = ""
    for _ in range(120):
        found = subprocess.run(
            ["xdotool", "search", "--onlyvisible", "--pid", str(pid)],
            env=env(),
            capture_output=True,
            text=True,
        )
        if found.stdout.strip():
            window = found.stdout.splitlines()[0]
            break
        if app.poll() is not None:
            app_failure("GUI exited before mapping its window")
        time.sleep(1)
    if not window:
        app_failure("No GUI window within 120 seconds")
    (ROOT / "window").write_text(window)
    subprocess.run(
        [
            "xdotool",
            "windowsize",
            window,
            "1260",
            "780",
            "windowmove",
            window,
            "10",
            "0",
            "windowactivate",
            "--sync",
            window,
        ],
        env=env(),
        check=True,
    )
    for _ in range(60):
        shot = OUT / "new/00-smoke.png"
        subprocess.run(["scrot", "-o", str(shot)], env=env(), check=True)
        # Exclude title bar and desktop; a decorated blank window must not pass.
        with Image.open(shot) as image:
            spread = ImageStat.Stat(
                image.crop((30, 80, 1240, 740)).convert("L")
            ).stddev[0]
        if spread > 5:
            break
        time.sleep(1)
    else:
        app_failure("Blank client area")
    time.sleep(5)
    if app.poll() is not None or panicked():
        app_failure("GUI crashed during smoke")
    receipt(smoke=True)


def app_failure(message):
    receipt(smoke=False)
    raise RuntimeError(message)


def panicked():
    paths = list((ROOT / "data").glob("*.log")) + [ROOT / "launch.log"]
    return any(
        re.search(
            r"panicked at|location=.*\.rs:[0-9]+|FATAL:\s*SIG[A-Z]+",
            p.read_text(errors="replace"),
        )
        for p in paths
        if p.exists()
    )


def agent():
    # No plaintext phrase is ever written; remove generated app config too.
    for path in (ROOT / "data").rglob(".env*"):
        path.unlink()
    public_wallet = read(ROOT / "wallet.json", {"funding": "not_reached"})
    (OUT / "wallet-status.json").write_text(json.dumps(public_wallet))
    prompt = (
        Path(os.environ["QA_SOURCE"]) / ".claude/skills/release-review/ci-prompt.md"
    ).read_text()
    prompt += f"\nPublic context: {ROOT}/context. Output: {OUT}. Skills: {os.environ['QA_SOURCE']}/.claude/skills."
    settings = {
        "disableAllHooks": False,
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": ".*",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "/usr/bin/python3 /opt/release-qa/guard.py",
                            "onFailure": "block",
                            "timeout": 10,
                        }
                    ],
                }
            ]
        },
    }
    (ROOT / "settings.json").write_text(json.dumps(settings))
    agent_env = env() | {
        "PATH": os.environ["PATH"],
        "QA_ROOT": str(ROOT),
        "QA_SOURCE": os.environ["QA_SOURCE"],
        "CLAUDE_CODE_OAUTH_TOKEN": os.environ["CLAUDE_CODE_OAUTH_TOKEN"],
        "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
        "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS": "1",
    }
    work = ROOT / "agent-work"
    work.mkdir(exist_ok=True, mode=0o700)
    args = [
        "claude",
        "--restricted",
        "--disable-slash-commands",
        "--add-dir",
        str(ROOT / "context"),
        str(OUT),
        os.environ["QA_SOURCE"],
        "-p",
        "--permission-mode",
        "default",
        "--settings",
        str(ROOT / "settings.json"),
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        '{"mcpServers":{}}',
        "--tools",
        "Read,Write,Edit,Bash",
        "--allowedTools",
        "Read",
        "Bash(/usr/local/bin/qa-ui *)",
        "--max-turns",
        "600",
        "--no-session-persistence",
        "--output-format",
        "json",
        prompt,
    ]
    with (ROOT / "agent.log").open("w") as log:
        result = subprocess.run(
            ["timeout", "--signal=TERM", "--kill-after=30s", "130m", *args],
            env=agent_env,
            cwd=work,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=7840,
        )
    response = read(ROOT / "agent.log", {})
    receipt(
        agent_ok=result.returncode == 0
        and isinstance(response, dict)
        and response.get("subtype") == "success"
        and response.get("is_error") is False,
        agent_exit=result.returncode,
    )


def stop():
    path = ROOT / "gui.pid"
    if not path.exists():
        if read(ROOT / "receipt.json", {}).get("smoke") is True:
            receipt(smoke=False)
        return False
    pid = int(path.read_text())
    ended = read(ROOT / "gui-exit.json")
    if ended is None and not Path(f"/proc/{pid}").exists():
        receipt(smoke=False, gui_stopped=False)
        return False
    if not (ROOT / "gui-stopping").exists():
        if ended is not None:
            receipt(smoke=False)
        (ROOT / "gui-stopping").touch()
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            receipt(smoke=False)
    for _ in range(300):
        ended = read(ROOT / "gui-exit.json")
        if ended is not None:
            break
        time.sleep(0.1)
    if ended is None:
        receipt(gui_stopped=False)
        if Path(f"/proc/{pid}").exists():
            os.kill(pid, signal.SIGKILL)
        return False
    if (
        not ended.get("controlled")
        or ended.get("code") not in (0, -signal.SIGTERM)
        or panicked()
    ):
        receipt(smoke=False)
    receipt(gui_stopped=True)


def evidence_complete():
    try:
        required = (
            "report.md",
            "verified-changelog.md",
            "scenarios.md",
            "story-checklist.md",
            "index.html",
        )
        if not all((OUT / name).read_text().strip() for name in required):
            return False
        meta, spec = read(OUT / "meta.json"), read(OUT / "spec.json")
        if (
            not isinstance(meta, dict)
            or not isinstance(spec, dict)
            or not meta.get("images")
        ):
            return False
        names = {item["name"] for item in meta["images"]}
        if not names.issubset(spec) or any(
            not re.fullmatch(CAPTURE_NAME, name) for name in names
        ):
            return False
        for name in names:
            if not isinstance(spec[name], list):
                return False
            for directory in ("new", "annotated", "annotated/clean"):
                with Image.open(OUT / directory / (name + ".png")) as picture:
                    picture.verify()
        commits = [
            line.split()[0]
            for line in (ROOT / "context/commits.txt").read_text().splitlines()
        ]
        coverage = meta["coverage"]
        if not isinstance(coverage, list):
            return False
        for row in coverage:
            if (
                not re.fullmatch(r"[0-9a-f]{7,40}", row["commit"])
                or not isinstance(row["images"], list)
                or not set(row["images"]).issubset(names)
                or (not row["images"] and not str(row.get("note", "")).strip())
            ):
                return False
        return all(
            any(commit.startswith(row["commit"]) for row in coverage)
            for commit in commits
        )
    except (OSError, ValueError, TypeError, KeyError, AttributeError):
        return False


def finish():
    export, stage = ROOT / "export", ROOT / "export.tmp"
    for directory in (export, stage):
        if directory.exists():
            shutil.rmtree(directory)
    try:
        _finish()
        return True
    except Exception:
        for directory in (export, stage):
            if directory.exists():
                shutil.rmtree(directory)
        export.mkdir()
        (export / "result.json").write_text('{"verdict":"INCOMPLETE"}')
        (export / "report.md").write_text(
            "# INCOMPLETE\nArtifact export withheld: scan or export failed.\n"
        )
        (export / "export-manifest.json").write_text('{"complete":false}')
        return False


def _finish():
    state = read(ROOT / "receipt.json", {})
    report = read(OUT / "report.json")
    try:
        value = verdict(
            report, smoke=state.get("smoke"), agent_ok=state.get("agent_ok")
        )
    except (TypeError, AttributeError, KeyError):
        value = "INCOMPLETE"
    wallet = read(ROOT / "wallet.json", {})
    if not isinstance(wallet, dict):
        wallet = {}
    if value in ("PASS", "NOTE"):
        if state.get("gui_stopped") is not True or not evidence_complete():
            value = "INCOMPLETE"
        elif wallet.get("refund") not in ("sent", "not_needed", "skipped_no_address"):
            value = "INCOMPLETE"
        elif wallet.get("funding") not in ("sent", "replay"):
            value = "NOTE"
    if wallet.get("warning"):
        from wallet import REFUND_WARNING

        wallet["warning"] = REFUND_WARNING
    if (OUT / "index.html").is_file():
        page = embed_images((OUT / "index.html").read_text(), OUT)
        (OUT / "visual-changelog.html").write_text(page)
    (OUT / "result.json").write_text(
        json.dumps({"verdict": value, "receipt": state, "wallet": wallet})
    )
    if not isinstance(report, dict):
        report = {
            "scenarios": [],
            "findings": [],
            "not_reached": [],
        }
    report["summary"] = {
        "verdict": "FAIL" if value == "INCOMPLETE" else value,
        "complete": value != "INCOMPLETE",
        "controller_verdict": value,
    }
    (OUT / "report.json").write_text(json.dumps(report, indent=2))
    if not (OUT / "report.md").exists():
        (OUT / "report.md").write_text(
            f"# Release GUI QA: {value}\nThe run did not complete. See result.json.\n"
        )
    with (OUT / "report.md").open("a") as output:
        output.write(f"\nController verdict: **{value}**.\n")
        if wallet.get("warning"):
            from wallet import REFUND_WARNING

            output.write(f"\nWARNING: {REFUND_WARNING}\n")
    for path in list(ROOT.glob("*.log")) + list((ROOT / "data").glob("*.log")):
        if path.name != "agent.log":
            (OUT / ("app-" + path.name)).write_bytes(path.read_bytes())
    export_report(value)


def export_report(value):
    export, stage = ROOT / "export", ROOT / "export.tmp"
    candidates = []
    for path in OUT.rglob("*"):
        if (
            path.is_symlink()
            or not path.is_file()
            or path.suffix not in (".json", ".md", ".html", ".png", ".log")
        ):
            continue
        if (
            not path.resolve().is_relative_to(OUT.resolve())
            or path.stat().st_size > 30_000_000
        ):
            raise ValueError("Export file is unsafe or oversized")
        data = path.read_bytes()
        if path.suffix != ".png":
            text = data.decode("utf-8")
            if has_secret(text):
                raise ValueError("Suspected credential text")
            if path.suffix == ".html":
                data = sanitize_html(text, trusted_css=CSS).encode()
        candidates.append((path.relative_to(OUT), data))
    names = {str(name) for name, _ in candidates}
    required = {"result.json", "report.json", "report.md"}
    if value in ("PASS", "NOTE"):
        required |= {
            "verified-changelog.md",
            "scenarios.md",
            "story-checklist.md",
            "meta.json",
            "spec.json",
            "index.html",
            "visual-changelog.html",
        }
        for item in read(OUT / "meta.json")["images"]:
            required |= {
                f"{directory}/{item['name']}.png"
                for directory in ("new", "annotated", "annotated/clean")
            }
    if not required <= names:
        raise ValueError("Missing required export")
    stage.mkdir()
    manifest = {}
    for name, data in candidates:
        target = stage / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        manifest[str(name)] = hashlib.sha256(data).hexdigest()
    (stage / "export-manifest.json").write_text(
        json.dumps({"complete": True, "files": manifest})
    )
    stage.rename(export)


if __name__ == "__main__":
    result = {
        "init": init,
        "smoke": smoke,
        "resume": resume,
        "monitor": monitor,
        "agent": agent,
        "stop": stop,
        "finish": finish,
    }[sys.argv[1]]()
    if result is False:
        sys.exit(1)
