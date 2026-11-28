"""Offline regression contracts for tutorial production helpers."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]


def load(relative):
    spec = importlib.util.spec_from_file_location(
        "revision_" + Path(relative).stem, ROOT / relative
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_preflight_preserves_crlf_hash(tmp_path):
    voice = load("scripts/openai_voice.py")
    source = tmp_path / "script.txt"
    source.write_bytes(b"Hello.\r\n")
    assert (
        voice.preflight(source, tmp_path / "take", "cedar").encode()
        == source.read_bytes()
    )


@pytest.mark.parametrize("choices", [[], None, {}, [None]])
def test_empty_choices_is_domain_error(choices):
    with pytest.raises(ValueError):
        load("scripts/openai_voice.py").validate_response(
            json.dumps({"choices": choices}), "Hello."
        )


def test_wait_absent_multiple_matches(tmp_path, monkeypatch):
    mod = load("scripts/gui/gui.py")
    gui = mod.Gui.__new__(mod.Gui)
    matches = iter([[1, 2], [], []])
    gui.match = lambda _: next(matches)
    monkeypatch.setattr(mod.time, "sleep", lambda _: None)
    assert gui.wait({"name": "Gone"}, absent=True) is None


@pytest.mark.parametrize(
    "step",
    [
        {"action": "hold"},
        {"action": "hold", "seconds": "nan"},
        {"action": "screenshot"},
        {"action": "screenshot", "name": "../escape"},
        {"action": "wait", "target": {"name": "Ready"}, "timeout": -1},
    ],
)
def test_invalid_scene_rejected_before_gui(tmp_path, monkeypatch, step):
    mod = load("scripts/gui/gui.py")
    source = tmp_path / "scene.json"
    source.write_text(
        json.dumps({"steps": [{"action": "click", "target": {"name": "OK"}}, step]})
    )
    monkeypatch.setattr(sys, "argv", ["gui.py", "scene", str(source)])
    with pytest.raises((ValueError, SystemExit)):
        mod.main()


def test_check_rejects_remote_display(tmp_path):
    import hashlib

    binary = tmp_path / "binary"
    binary.write_bytes(b"fixture")
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/gui/launch.py"),
            "--check",
            "--binary",
            str(binary),
            "--binary-sha256",
            hashlib.sha256(binary.read_bytes()).hexdigest(),
            "--profile",
            str(tmp_path / "profile"),
            "--config",
            str(binary),
            "--evidence",
            str(tmp_path / "evidence"),
            "--display",
            "localhost:100",
        ],
        capture_output=True,
    )
    assert result.returncode != 0


def test_private_bus_ignores_foreign_addresses(tmp_path):
    mod = load("scripts/gui/launch.py")
    env = {"DBUS_SESSION_BUS_ADDRESS": "foreign", "AT_SPI_BUS_ADDRESS": "foreign"}
    mod.prepare_environment(env, ":100", tmp_path, tmp_path / "auth")
    assert "AT_SPI_BUS_ADDRESS" not in env


@pytest.mark.parametrize(
    "expected",
    [
        "Do not run `import-audio` again",
        "periodic sampling",
        "Rebase relative paths",
        "scene 50",
        "does not protect every frame",
    ],
)
def test_documented_operator_contract(expected):
    assert expected in (ROOT / "README.md").read_text()


def test_private_bus_lifecycle_and_foreign_registration(monkeypatch):
    mod = load("scripts/gui/launch.py")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "foreign")
    monkeypatch.setenv("AT_SPI_BUS_ADDRESS", "foreign")
    actual = subprocess.Popen
    captured = {}

    def run(command, env, **kwargs):
        assert command[:2] == ["dbus-run-session", "--"]
        assert "DBUS_SESSION_BUS_ADDRESS" not in env
        assert "AT_SPI_BUS_ADDRESS" not in env
        child = actual(
            [
                "dbus-run-session",
                "--",
                sys.executable,
                "-c",
                'import os; print(os.environ["DBUS_SESSION_BUS_ADDRESS"])',
            ],
            env=env,
            stdout=subprocess.PIPE,
            text=True,
            **kwargs,
        )
        captured["process"] = child
        return child

    monkeypatch.setattr(mod.subprocess, "Popen", run)
    assert mod.run_private_session(["--check"]) == 0
    monkeypatch.setattr(mod.subprocess, "Popen", actual)
    address = captured["process"].stdout.read().strip()
    result = subprocess.run(
        [
            "gdbus",
            "call",
            "--address",
            address,
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.ListNames",
        ],
        capture_output=True,
    )
    assert result.returncode != 0


def minimal_media(tmp_path, synthetic=False):
    import hashlib
    import wave

    mod = load("scripts/media/media.py")
    source = tmp_path / "scenario.json"
    source.write_text(
        json.dumps(
            {
                "speakers": {"teacher": {"voice": "cedar", "name": "Teacher"}},
                "scenes": [
                    {
                        "id": f"{i:02}",
                        "title": "Fixture",
                        "turns": [{"speaker": "teacher", "text": "Hello."}],
                    }
                    for i in range(54)
                ],
                "chapters": [{"id": 1, "title": "Fixture", "start": "00", "end": "53"}],
            }
        )
    )
    manifest = tmp_path / "media.json"
    mod.init_manifest(source, manifest)
    data = mod.read_json(manifest)
    wav = tmp_path / "audio.wav"
    with wave.open(str(wav), "wb") as out:
        out.setparams((1, 2, 48000, 0, "NONE", "not compressed"))
        out.writeframes(b"\0\0" * 3200)
    (tmp_path / "script.txt").write_text("Hello.")
    (tmp_path / "transcript.txt").write_text("Hello.")
    (tmp_path / "metadata.json").write_text(
        json.dumps(
            {
                "voice": "cedar",
                "validation": "passed",
                "synthetic_test": synthetic,
                "audio_sha256": hashlib.sha256(wav.read_bytes()).hexdigest(),
                "script_sha256": hashlib.sha256(b"Hello.").hexdigest(),
            }
        )
    )
    for segment in data["segments"].values():
        segment["audio"] = [str(wav)]
        segment["visual"] = {"kind": "slide", "path": str(wav)}
    manifest.write_text(json.dumps(data))
    return mod, manifest, data


def test_synthetic_take_requires_synthetic_manifest(tmp_path, monkeypatch):
    mod, manifest, _ = minimal_media(tmp_path, synthetic=True)
    monkeypatch.setattr(
        mod,
        "probe",
        lambda _: {"streams": [{"codec_type": "video", "width": 1920, "height": 1080}]},
    )
    with pytest.raises(ValueError, match="[Ss]ynthetic"):
        mod.load_plan(manifest)


def test_incomplete_capture_is_rejected(tmp_path, monkeypatch):
    mod, manifest, data = minimal_media(tmp_path)
    partial = tmp_path / "03.incomplete.mkv"
    partial.touch()
    data["segments"]["00"]["visual"]["path"] = str(partial)
    manifest.write_text(json.dumps(data))
    monkeypatch.setattr(
        mod,
        "probe",
        lambda _: {"streams": [{"codec_type": "video", "width": 1920, "height": 1080}]},
    )
    with pytest.raises(ValueError, match="[Ii]ncomplete"):
        mod.load_plan(manifest)


def test_short_segment_cannot_lose_a_frame(tmp_path, monkeypatch):
    import shutil

    mod, manifest, _ = minimal_media(tmp_path)
    monkeypatch.setattr(
        mod,
        "probe",
        lambda _: {
            "streams": [
                {"codec_type": "video", "width": 1920, "height": 1080, "nb_frames": "1"}
            ],
            "format": {"duration": str(1 / 30)},
        },
    )
    segment = mod.load_plan(manifest)["segments"]["00"]

    def ffmpeg(args, log):
        target = Path(args[-1])
        if target.suffix == ".wav":
            shutil.copyfile(args[1], target)
        else:
            target.touch()

    monkeypatch.setattr(mod, "ffmpeg", ffmpeg)
    with pytest.raises(ValueError, match="frame"):
        mod.render_segment(segment, tmp_path, 0, "ultrafast", None)


def test_alternate_font_family_embedded():
    import ast
    from types import SimpleNamespace

    source = ROOT / "assets/emission-examples/generate.py"
    tree = ast.parse(source.read_text())
    render = next(
        node
        for node in tree.body
        if isinstance(node, ast.FunctionDef) and node.name == "render"
    )
    assignments = [
        node
        for node in render.body
        if isinstance(node, ast.Assign)
        and any(
            isinstance(t, ast.Name) and t.id in ("font_family", "embed")
            for t in node.targets
        )
    ]
    namespace = {
        "font_uri": "file:///fixture.ttf",
        "logo_uri": "fixture",
        "FONT": "fixture.ttf",
        "font_manager": SimpleNamespace(
            FontProperties=lambda **_: SimpleNamespace(get_name=lambda: "DejaVu Sans")
        ),
    }
    exec(
        compile(ast.Module(body=assignments, type_ignores=[]), str(source), "exec"),
        namespace,
    )
    assert "font-family:DejaVu Sans;" in namespace["embed"]


def audio_proof(tone):
    import array
    import ast
    from types import SimpleNamespace

    source = ROOT / "scripts/media/integration_check.py"
    tree = ast.parse(source.read_text())
    run = next(
        n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "run"
    )
    start = next(
        i
        for i, n in enumerate(run.body)
        if isinstance(n, ast.Assign)
        and any(isinstance(t, ast.Name) and t.id == "pcm" for t in n.targets)
    )
    end = next(
        i
        for i, n in enumerate(run.body)
        if isinstance(n, ast.Assign)
        and any(isinstance(t, ast.Name) and t.id == "embedded" for t in n.targets)
    )

    def output(args):
        count = round(float(args[args.index("-t") + 1]) * 48000)
        return array.array(
            "h", [0 if i < 144000 else tone for i in range(count)]
        ).tobytes()

    namespace = {
        "subprocess": SimpleNamespace(check_output=output),
        "array": array,
        "ready": Path("/fixture"),
        "package": {"file": "fixture.mp4", "cues": [{"start": 3}]},
    }
    exec(
        compile(
            ast.Module(body=run.body[start:end], type_ignores=[]), str(source), "exec"
        ),
        namespace,
    )


def test_audio_proof_rejects_missing_first_tone():
    with pytest.raises(AssertionError):
        audio_proof(0)


def test_audio_proof_accepts_first_tone():
    audio_proof(2000)
