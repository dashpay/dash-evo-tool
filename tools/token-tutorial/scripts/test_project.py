"""Portable project setup and free reuse must never reach a paid requester."""

import json
from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import project
import openai_voice as voice
from test_openai_voice import response


def scenario(tmp_path):
    document = json.loads((project.ROOT / "scenario/dialogue.json").read_text())
    path = tmp_path / "dialogue.json"
    path.write_text(json.dumps(document))
    return path


def test_prepare_resolves_relative_to_config_not_working_directory(
    tmp_path, monkeypatch
):
    config = tmp_path / "project.json"
    config.write_text(
        json.dumps(
            {
                "scenario": str(project.ROOT / "scenario/dialogue.json"),
                "work": "private",
            }
        )
    )
    monkeypatch.chdir(tmp_path.parent)
    manifest = project.prepare(config)
    data = json.loads(manifest.read_text())
    assert manifest == tmp_path / "private/media.json"
    assert data["title_lead_seconds"] == 3
    assert not Path(data["scenario"]).is_absolute()
    assert (manifest.parent / data["scenario"]).resolve().is_file()
    assert all(
        not Path(s["visual"]["path"]).is_absolute() for s in data["segments"].values()
    )


def test_reuse_only_exact_approved_turn_and_no_paid_calls(tmp_path, monkeypatch):
    source = scenario(tmp_path)
    original = json.loads(source.read_text())
    text = original["scenes"][0]["turns"][0]["text"]
    old_script = tmp_path / "old.txt"
    old_script.write_text(text + "\n")
    take = tmp_path / "take"
    ready = voice.generate_take(
        old_script,
        take,
        voice="cedar",
        key_loader=lambda: "fake",
        requester=lambda *_: response(transcript=text),
    )
    media = tmp_path / "old-media.json"
    media.write_text(
        json.dumps(
            {
                "scenario": str(source),
                "scenario_sha256": project.batch.digest(source.read_bytes()),
                "segments": {"00": {"audio": [str(ready / "audio.wav")]}},
            }
        )
    )
    changed = tmp_path / "changed.json"
    changed.write_text(json.dumps(original))
    monkeypatch.setattr(
        voice, "request_audio", lambda *_: pytest.fail("Paid API access")
    )
    root = tmp_path / "reused"
    result = project.reuse(changed, root, media)
    assert result["reused"] == ["scene-00-turn-00"]
    assert project.batch.run_batch(changed, root)["summary"]["ready"] == 1
    # A different instruction, voice or text cannot inherit the paid take.
    original_metadata = json.loads((ready / "metadata.json").read_text())
    for change in ("text", "voice", "instructions"):
        changed.write_text(json.dumps(original))
        (ready / "metadata.json").write_text(json.dumps(original_metadata))
        metadata = json.loads((ready / "metadata.json").read_text())
        if change == "text":
            doc = json.loads(changed.read_text())
            doc["scenes"][0]["turns"][0]["text"] += " Changed."
            changed.write_text(json.dumps(doc))
        else:
            metadata[change] = "different"
            (ready / "metadata.json").write_text(json.dumps(metadata))
        refused = project.reuse(changed, tmp_path / change, media)
        assert refused["reused"] == []
        assert refused["rejected"]["scene-00-turn-00"]


def test_transcript_is_derived_from_canonical_speaker_names(tmp_path):
    source = scenario(tmp_path)
    text = project.transcript(source)
    assert "Chad G. P. Taylor:" in text
    assert "Ewa Dashwood:" in text
    assert "[Scene 53]" in text


def test_current_source_inventory_provenance_and_readable_rendering():
    source = project.ROOT / "scenario/dialogue.json"
    data = json.loads(source.read_text())
    plan = source.parent / data["source"]["scenario"]
    assert project.batch.digest(plan.read_bytes()) == data["source"]["scenario_sha256"]
    assert data["source"]["det_revision"] == "975993070d0061f506c0b6a2ec650df4ec18f555"
    assert data["source"]["platform_revision"] == (
        "575d961fe0c54bd7e01a3cf3823494e58c6c4fdb"
    )
    assert [s["id"] for s in data["scenes"]] == [f"{i:02}" for i in range(54)]
    assert [(p["start"], p["end"]) for p in data["parts"]] == [
        ("00", "09"),
        ("10", "18"),
        ("19", "29"),
        ("30", "44"),
        ("45", "49"),
        ("50", "53"),
    ]
    assert sum(len(p[k]) for p in data["parts"] for k in ("intro", "outro")) == 12
    assert data["packaging"]["opening_silence_seconds"] == 3
    assert project.transcript(source) == (source.parent / "dialogue.txt").read_text()
    assert data["word_count"]["scenes"] == sum(
        len(turn["text"].split())
        for scene in data["scenes"]
        for turn in scene["turns"]
    )


def test_historical_media_remains_archival_and_rejects_current_source(tmp_path):
    source = project.ROOT / "scenario/dialogue.json"
    archive = source.parent / "accepted-media.json"
    subtitles = source.parent / "accepted-combined.en.srt"
    assert project.batch.digest(archive.read_bytes()) == (
        "89934453dba4f878565902665e7043e3f0b8ae12f0cf5ef87582712ab539a343"
    )
    assert project.batch.digest(subtitles.read_bytes()) == (
        "06cfdb028adce0b52fc2668346412e8e1d3e07737e2cf1330aba9fbc87fba252"
    )
    assert json.loads(archive.read_text())["scenario_sha256"] != project.batch.digest(
        source.read_bytes()
    )
    with pytest.raises(ValueError, match="stale scenario hash"):
        project.reuse(source, tmp_path / "must-not-reuse", archive)
    assert not (tmp_path / "must-not-reuse").exists()
    assert "00:00:03,000 -->" in subtitles.read_text()
