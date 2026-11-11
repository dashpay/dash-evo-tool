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
