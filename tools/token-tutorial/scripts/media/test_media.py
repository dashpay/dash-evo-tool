"""Offline contracts for exact-script captions and input rejection."""

import importlib.util
from pathlib import Path

import pytest


@pytest.fixture
def media():
    path = Path(__file__).with_name("media.py")
    if not path.exists():
        pytest.fail("Media implementation not created yet")
    spec = importlib.util.spec_from_file_location("tutorial_media", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_timestamp_rollover(media):
    assert media.timestamp(59.9996) == "00:01:00,000"
    assert media.timestamp(3600.001, vtt=True) == "01:00:00.001"


def test_captions_preserve_labels_and_words(media):
    cues = [
        {
            "start": 0.2,
            "end": 1.7,
            "speaker": "Czatosław Klałdowski",
            "text": "Use <script> & keep punctuation.",
        }
    ]
    srt = media.captions(cues)
    assert "00:00:00,200 --> 00:00:01,700" in srt
    assert "Czatosław Klałdowski: Use <script> & keep punctuation." in " ".join(
        srt.split()
    )
    assert media.captions(cues, vtt=True).startswith("WEBVTT\n\n")


def test_template_does_not_invent_source_recordings(media, tmp_path):
    scenario = Path(__file__).resolve().parents[2] / "scenario/dialogue.json"
    target = tmp_path / "manifest.json"
    media.init_manifest(scenario, target)
    with pytest.raises(ValueError, match="Missing input"):
        media.load_plan(target)
    assert not (tmp_path / "delivery").exists()


def test_template_never_overwrites(media, tmp_path):
    source = Path(__file__).resolve().parents[2] / "scenario/dialogue.json"
    target = tmp_path / "manifest.json"
    target.write_text("original")
    with pytest.raises(FileExistsError):
        media.init_manifest(source, target)
    assert target.read_text() == "original"


def test_caption_chunks_are_readable_and_preserve_all_words(media):
    text = "A long explanation of token permissions and their consequences " * 6
    chunks = media.chunk_turn(text, "Chad G. P. Taylor", 2.0, 12.0, 0)
    assert " ".join(cue["text"] for cue in chunks).split() == text.split()
    assert chunks[0]["start"] == 2.0
    assert chunks[-1]["end"] == 14.0
    for cue in chunks:
        import textwrap

        assert len(textwrap.wrap(cue["speaker"] + ": " + cue["text"], width=42)) <= 2


def test_import_episode_one_root_audio_map_ignores_other_parts(media, tmp_path):
    import json

    scenario = Path(__file__).resolve().parents[2] / "scenario/dialogue.json"
    source = tmp_path / "media.json"
    media.init_manifest(scenario, source)
    manifest = json.loads(source.read_text())
    jobs = []
    for sid, segment in manifest["segments"].items():
        if sid in [f"{i:02}" for i in range(10)] + ["p1-intro", "p1-outro"]:
            for index in range(len(segment["audio"])):
                jobs.append(
                    {
                        "id": f"{sid}-{index:02}",
                        "segment_id": sid,
                        "turn_index": index,
                        "take": str(tmp_path / f"take-{sid}-{index}"),
                        "status": "ready",
                    }
                )
    mapping = tmp_path / "audio-map.json"
    mapping.write_text(
        json.dumps({"source_sha256": manifest["scenario_sha256"], "jobs": jobs})
    )
    output = tmp_path / "filled.json"
    media.import_audio(source, mapping, output, part=1)
    imported = json.loads(output.read_text())
    assert imported["segments"]["00"]["audio"][0].endswith("take-00-0/ready/audio.wav")
    assert imported["segments"]["10"]["audio"][0] is None


def test_standalone_transcript_includes_its_spoken_bridges(media):
    plan = {
        "segments": {
            "p1-intro": {
                "title": "Intro",
                "turns": [{"speaker": "Teacher", "text": "Start here."}],
            },
            "00": {
                "title": "Lesson",
                "turns": [{"speaker": "Learner", "text": "Why?"}],
            },
            "p1-outro": {
                "title": "Outro",
                "turns": [{"speaker": "Teacher", "text": "Continue next."}],
            },
        }
    }
    transcript = media.package_transcript(plan, ["p1-intro", "00", "p1-outro"])
    assert (
        transcript.index("Start here.")
        < transcript.index("Why?")
        < transcript.index("Continue next.")
    )


def test_subtitle_mp4_roundtrip_keeps_apostrophes_quotes_and_symbols(media, tmp_path):
    import subprocess

    cue = {
        "start": 0,
        "end": 1,
        "speaker": "Teacher",
        "text": 'I\'m using "quotes" & symbols.',
    }
    source = tmp_path / "input.srt"
    source.write_text(media.captions([cue]))
    mp4 = tmp_path / "captions.mp4"
    output = tmp_path / "roundtrip.ass"
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-n",
            "-v",
            "error",
            "-i",
            str(source),
            "-c:s",
            "mov_text",
            str(mp4),
        ],
        check=True,
    )
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-n",
            "-v",
            "error",
            "-i",
            str(mp4),
            "-map",
            "0:s:0",
            "-c:s",
            "ass",
            str(output),
        ],
        check=True,
    )
    assert 'Teacher: I\'m using "quotes" & symbols.' in " ".join(
        output.read_text().replace("\\N", " ").split()
    )


def test_short_tail_balances_words_without_changing_turn_timing(media):
    text = (
        "An active authentication key authorizes changes "
        "An active authentication key authorizes changes An active authentication"
    )
    cues = media.chunk_turn(text, "Chad G. P. Taylor", 2.0, 4.5, 0)
    assert min(cue["end"] - cue["start"] for cue in cues) >= 0.8
    assert cues[0]["start"] == 2.0
    assert cues[-1]["end"] == 6.5
    assert " ".join(cue["text"] for cue in cues).split() == text.split()
    import textwrap

    assert all(
        len(textwrap.wrap(cue["speaker"] + ": " + cue["text"], 42)) <= 2 for cue in cues
    )


def test_import_rebases_both_manifests_relative_to_the_output(media, tmp_path):
    import json

    source = tmp_path / "source"
    source.mkdir()
    destination = tmp_path / "destination"
    destination.mkdir()
    template = source / "media.json"
    template.write_text(
        json.dumps(
            {
                "scenario_sha256": "same",
                "scenario": "dialogue.json",
                "credits": "credits.txt",
                "font_license": "license.txt",
                "segments": {
                    "00": {
                        "visual": {"kind": "video", "path": "capture.mkv"},
                        "audio": [None],
                    }
                },
            }
        )
    )
    narration = source / "narration.json"
    narration.write_text(
        json.dumps(
            {
                "script_sha256": "same",
                "jobs": [
                    {
                        "id": "scene-00-turn-00",
                        "status": "ready",
                        "audio_path": "take/ready/audio.wav",
                    }
                ],
            }
        )
    )
    target = destination / "imported.json"
    media.import_audio(template, narration, target)
    result = json.loads(target.read_text())
    assert (destination / result["scenario"]).resolve() == source / "dialogue.json"
    assert (
        destination / result["segments"]["00"]["visual"]["path"]
    ).resolve() == source / "capture.mkv"
    assert (
        destination / result["segments"]["00"]["audio"][0]
    ).resolve() == source / "take/ready/audio.wav"


def test_public_export_excludes_private_input_paths(media, tmp_path, monkeypatch):
    import json
    import wave

    private_marker = "/private/operator-profile/DO_NOT_PUBLISH"
    manifest = tmp_path / "input.json"
    manifest.write_text(json.dumps({"source_path": private_marker}))
    slide = tmp_path / "slide.png"
    slide.write_bytes(b"public slide fixture")
    credits = tmp_path / "credits.txt"
    credits.write_text("Public credits")
    license_path = tmp_path / "license.txt"
    license_path.write_text("Public font license")
    wav_path = tmp_path / "audio.wav"
    with wave.open(str(wav_path), "wb") as wav:
        wav.setparams((1, 2, 48000, 0, "NONE", "not compressed"))
        wav.writeframes(b"\0\0" * 4800)
    segment = {
        "title": "Public lesson",
        "duration": 0.1,
        "editorial_hold_seconds": 0,
        "visual": {"path": slide},
        "turns": [{"speaker": "Teacher", "text": "Hello.", "path": private_marker}],
        "cues": [
            {"start": 0, "end": 0.1, "speaker": "Teacher", "text": "Hello.", "turn": 0}
        ],
    }
    plan = {
        "manifest": {"scenario_sha256": "approved", "source_path": private_marker},
        "scenario": {
            "title": "Public tutorial",
            "parts": [{"id": 1}],
            "scenes": [{"id": "00", "part": 1}],
        },
        "segments": {sid: dict(segment) for sid in ("p1-intro", "00", "p1-outro")},
        "credits": credits,
        "font_license": license_path,
        "gap": 0,
    }
    monkeypatch.setattr(media, "load_plan", lambda *_: plan)
    monkeypatch.setattr(media, "render_segment", lambda *_: (slide, wav_path))
    monkeypatch.setattr(
        media,
        "ffmpeg",
        lambda args, _: Path(args[-1]).write_bytes(b"public video fixture"),
    )
    monkeypatch.setattr(
        media, "probe", lambda _: {"format": {"duration": "0.3"}, "chapters": [{}] * 3}
    )
    output = tmp_path / "delivery"
    ready = media.build(manifest, output, part=1)
    assert not (ready / "input-manifest.json").exists()
    assert (output / "input-manifest.json").read_bytes() == manifest.read_bytes()
    assert {"delivery.json", "part-01.timeline.json", "index.html"} <= {
        p.name for p in ready.iterdir()
    }
    assert all(
        private_marker.encode() not in p.read_bytes()
        for p in ready.rglob("*")
        if p.is_file()
    )


def test_init_default_credits_and_license_exist_for_external_scenario(media, tmp_path):
    import shutil

    source = tmp_path / "external-scenario.json"
    shutil.copyfile(
        Path(__file__).resolve().parents[2] / "scenario/dialogue.json", source
    )
    target = tmp_path / "template.json"
    media.init_manifest(source, target)
    data = media.read_json(target)
    assert Path(data["credits"]).is_file()
    assert Path(data["font_license"]).is_file()


def test_concat_listing_handles_apostrophes_and_spaces(media, tmp_path):
    import subprocess
    import wave

    scratch = tmp_path / "editor's scratch"
    scratch.mkdir()
    audio = scratch / "a narrator's take.wav"
    with wave.open(str(audio), "wb") as wav:
        wav.setparams((1, 2, 48000, 0, "NONE", "not compressed"))
        wav.writeframes(b"\0\0" * 4800)
    listing = scratch / "input.ffconcat"
    listing.write_text("ffconcat version 1.0\n" + media.concat_file(audio))
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-v",
            "error",
            "-safe",
            "0",
            "-f",
            "concat",
            "-i",
            str(listing),
            "-f",
            "null",
            "-",
        ],
        check=True,
        capture_output=True,
    )
