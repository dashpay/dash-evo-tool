#!/usr/bin/python3
"""Synthetic A/V integration proof only; never represents real GUI or narration."""

import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import wave

from PIL import Image

import media


def run(part=None):
    root = Path(__file__).resolve().parents[2]
    fixture = Path(tempfile.mkdtemp(prefix="tutorial-media-synthetic-"))
    (fixture / "scenario").mkdir()
    scenario_path = fixture / "scenario/dialogue.json"
    shutil.copyfile(root / "scenario/dialogue.json", scenario_path)
    manifest_path = fixture / "manifest.json"
    media.init_manifest(scenario_path, manifest_path)
    manifest = media.read_json(manifest_path)
    manifest.update(
        teacher_voice="cedar",
        learner_voice="nova",
        synthetic_test=True,
        turn_gap_seconds=0.03,
        credits=str(root / "assets/ATTRIBUTION.txt"),
        font_license=str(root / "assets/slides/montserrat-ofl.txt"),
    )
    scenario = media.read_json(scenario_path)
    slide = fixture / "synthetic-slide.png"
    Image.new("RGB", (1920, 1080), "#0b1629").save(slide)
    video = fixture / "synthetic-capture.mp4"
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-n",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30",
            "-t",
            "0.2",
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-preset",
            "ultrafast",
            str(video),
        ],
        check=True,
    )
    tone = fixture / "synthetic-tone.wav"
    with wave.open(str(tone), "wb") as audio:
        audio.setparams((1, 2, 24000, 0, "NONE", "not compressed"))
        audio.writeframes(
            b"".join(
                struct.pack(
                    "<h",
                    round(
                        2000
                        * min(1, i / 480, (2879 - i) / 480)
                        * math.sin(2 * math.pi * 440 * i / 24000)
                    ),
                )
                for i in range(2880)
            )
        )
    specs = [(scene["id"], scene["turns"]) for scene in scenario["scenes"]]
    for item in scenario["parts"]:
        for kind in ("intro", "outro"):
            specs.append((f"p{item['id']}-{kind}", item[kind]))
    for sid, turns in specs:
        if part is not None:
            begin, end = media.RANGES[part - 1]
            selected = (
                sid.startswith(f"p{part}-")
                if sid.startswith("p")
                else begin <= int(sid) <= end
            )
            if not selected:
                continue
        entry = manifest["segments"][sid]
        entry["visual"] = (
            {"kind": "slide", "path": str(slide)}
            if sid in ("00", "53") or sid.startswith("p")
            else {"kind": "video", "path": str(video), "hold_last_frame": True}
        )
        for index, turn in enumerate(turns):
            ready = fixture / "audio" / sid / str(index) / "ready"
            ready.mkdir(parents=True)
            shutil.copyfile(tone, ready / "audio.wav")
            script = turn["text"] + "\n"
            (ready / "script.txt").write_text(script)
            (ready / "transcript.txt").write_text(turn["text"])
            (ready / "metadata.json").write_text(
                json.dumps(
                    {
                        "voice": "cedar" if turn["speaker"] == "teacher" else "nova",
                        "validation": "passed",
                        "synthetic_test": True,
                        "audio_sha256": hashlib.sha256(tone.read_bytes()).hexdigest(),
                        "script_sha256": hashlib.sha256(script.encode()).hexdigest(),
                    }
                )
            )
            entry["audio"][index] = str(ready / "audio.wav")
    manifest_path.write_text(json.dumps(manifest))
    # Leave unrelated part paths empty: --part must not inspect or require them.
    ready = media.build(
        manifest_path, fixture / "delivery", preset="ultrafast", part=part
    )
    if (ready / "input-manifest.json").exists():
        raise AssertionError("Private input manifest leaked into the public export")
    assert (
        ready.parent / "input-manifest.json"
    ).read_bytes() == manifest_path.read_bytes()
    for public_file in ready.iterdir():
        if public_file.suffix in (".json", ".html", ".txt", ".srt", ".vtt", ".sha256"):
            contents = public_file.read_text()
            assert str(fixture) not in contents
            assert str(root) not in contents
    delivery = media.read_json(ready / "delivery.json")
    reports = []
    for key, package in delivery["packages"].items():
        actual = media.probe(ready / package["file"])
        public_tags = json.dumps(
            [actual.get("format", {}).get("tags", {})]
            + [stream.get("tags", {}) for stream in actual["streams"]]
        )
        assert str(fixture) not in public_tags and str(root) not in public_tags
        video_stream = next(s for s in actual["streams"] if s["codec_type"] == "video")
        assert (video_stream["width"], video_stream["height"]) == (1920, 1080)
        assert {s["codec_type"] for s in actual["streams"]} >= {
            "video",
            "audio",
            "subtitle",
        }
        assert actual["chapters"]
        assert abs(package["actual_duration"] - package["planned_duration"]) <= 0.08
        assert all(
            0 <= cue["start"] < cue["end"] <= package["actual_duration"] + 0.001
            for cue in package["cues"]
        )
        if key == "combined":
            assert [item["segment"] for item in package["timeline"]] == [
                "title-lead"
            ] + [f"{i:02}" for i in range(54)]
            expected_scenes = scenario["scenes"]
            for scene in expected_scenes:
                for index, turn in enumerate(scene["turns"]):
                    matching = [
                        cue["text"]
                        for cue in package["cues"]
                        if cue["segment"] == scene["id"] and cue["turn"] == index
                    ]
                    assert " ".join(matching).split() == turn["text"].split()
        else:
            pid = int(key.split("-")[1])
            first, last = media.RANGES[pid - 1]
            assert [item["segment"] for item in package["timeline"]] == [
                "title-lead",
                f"p{pid}-intro",
            ] + [f"{i:02}" for i in range(first, last + 1)] + [f"p{pid}-outro"]
        pcm = subprocess.check_output(
            [
                "ffmpeg",
                "-v",
                "error",
                "-i",
                str(ready / package["file"]),
                "-t",
                "3",
                "-vn",
                "-f",
                "s16le",
                "-ac",
                "1",
                "-ar",
                "48000",
                "-",
            ]
        )
        samples = array.array("h", pcm)
        assert len(samples) == 144000
        assert not any(samples[: round(2.98 * 48000)])
        assert max(map(abs, samples)) <= 32
        assert min(cue["start"] for cue in package["cues"]) >= 3
        embedded = subprocess.check_output(
            [
                "ffmpeg",
                "-v",
                "error",
                "-i",
                str(ready / package["file"]),
                "-map",
                "0:s:0",
                "-f",
                "srt",
                "-",
            ]
        ).decode()
        assert embedded.strip() == (ready / f"{key}.en.srt").read_text().strip()
        reports.append(
            {
                "package": key,
                "actual_seconds": package["actual_duration"],
                "planned_seconds": package["planned_duration"],
                "caption_cues": len(package["cues"]),
                "chapters": len(actual["chapters"]),
                "width": 1920,
                "height": 1080,
            }
        )
    assert len(reports) == (1 if part else 7)
    proof = {
        "synthetic_test_only": True,
        "fixture": str(fixture),
        "ready": str(ready),
        "part": part,
        "assertions": "Exact per-turn script coverage; bounded captions; six agreed ranges; no extras in combined; real ffprobe durations/chapters/streams; unrelated inputs not required for --part",
        "packages": reports,
    }
    (fixture / "proof.json").write_text(json.dumps(proof, indent=2) + "\n")
    print(json.dumps(proof, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--part1", action="store_true")
    args = parser.parse_args()
    run(1 if args.part1 else None)
