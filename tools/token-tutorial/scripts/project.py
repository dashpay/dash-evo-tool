#!/usr/bin/python3
"""Prepare a portable edit or reuse explicitly mapped, verified narration offline."""

import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import uuid

import batch_narration as batch
import openai_voice as voice

sys.path.insert(0, str(Path(__file__).parent / "media"))
import media

ROOT = Path(__file__).resolve().parents[1]


def relative(path, base):
    return os.path.relpath(Path(path).resolve(), Path(base).resolve())


def prepare(config_path):
    config_path = Path(config_path).resolve()
    config = media.read_json(config_path)
    base = config_path.parent
    source = (base / config["scenario"]).resolve()
    work = (base / config["work"]).resolve()
    work.mkdir(parents=True, exist_ok=True, mode=0o700)
    for directory in ("slides", "captures", "evidence"):
        (work / directory).mkdir(exist_ok=True)
    target = work / "media.json"
    if target.exists() or (work / "scenario.json").exists():
        raise FileExistsError(
            "Choose a fresh work directory; keep the previous source snapshot and journal"
        )
    shutil.copyfile(source, work / "scenario.json")
    media.init_manifest(work / "scenario.json", target)
    manifest = media.read_json(target)
    manifest.update(
        scenario="scenario.json",
        title_lead_seconds=3,
        credits=relative(ROOT / "assets/ATTRIBUTION.txt", work),
        font_license=relative(
            base / config.get("font_license", "../assets/slides/montserrat-ofl.txt"),
            work,
        ),
    )
    for sid, segment in manifest["segments"].items():
        if sid == "00" or sid.endswith("-intro"):
            visual = {
                "kind": "slide",
                "path": "slides/opening-1920x1080.png",
                "fade": False,
            }
        elif sid == "53" or sid.endswith("-outro"):
            visual = {"kind": "slide", "path": "slides/closing-1920x1080.png"}
        elif sid in ("01", "02", "50"):
            visual = {"kind": "slide", "path": f"slides/scene-{sid}.png"}
        else:
            visual = {
                "kind": "video",
                "path": f"captures/{sid}.mkv",
                "offset": 0,
                "hold_last_frame": False,
            }
        segment["visual"] = visual
    target.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    return target


def transcript(source):
    document = media.read_json(source)
    lines = [document["title"], ""]
    for scene in document["scenes"]:
        lines += [f"[Scene {scene['id']}] {scene['title']}"]
        lines += [
            f"{document['speakers'][turn['speaker']]['name']}: {turn['text']}"
            for turn in scene["turns"]
        ]
        lines += [""]
    lines += ["Standalone episode bridges (excluded from combined):", ""]
    for part in document["parts"]:
        for kind in ("intro", "outro"):
            lines += [f"[Part {part['id']} {kind}]"]
            lines += [
                f"{document['speakers'][turn['speaker']]['name']}: {turn['text']}"
                for turn in part[kind]
            ]
    return "\n".join(lines) + "\n"


def reuse(source, root, old_media_path):
    """Copy only verified takes from an operator-selected segment/turn mapping."""
    source, root, old_media_path = map(
        lambda p: Path(p).resolve(), (source, root, old_media_path)
    )
    old_media = media.read_json(old_media_path)
    old_source = (old_media_path.parent / old_media["scenario"]).resolve()
    if batch.digest(old_source.read_bytes()) != old_media["scenario_sha256"]:
        raise ValueError("Reuse source manifest has a stale scenario hash")
    old_document = media.read_json(old_source)
    old_jobs = {job["id"]: job for job in batch.expand(old_document, root)}
    result = {"reused": [], "rejected": {}}
    with batch.coordinator_lock(root):
        manifest = batch.load_plan(source, root, source.read_bytes())
        for job in manifest["jobs"]:
            if job["status"] != "planned":
                continue
            sid = (
                job["scene_id"]
                if job["segment"] == "scene"
                else f"p{job['part']}-{job['segment']}"
            )
            paths = old_media.get("segments", {}).get(sid, {}).get("audio", [])
            if job["turn_index"] >= len(paths) or not paths[job["turn_index"]]:
                continue
            try:
                old_job = old_jobs[job["id"]]
                if any(
                    old_job[key] != job[key]
                    for key in ("voice", "speaker", "text_sha256")
                ):
                    raise ValueError("Source speaker, voice or text changed")
                wav = (old_media_path.parent / paths[job["turn_index"]]).resolve()
                ready = wav.parent
                metadata = media.read_json(ready / "metadata.json")
                expected_instructions = (
                    voice.LEARNER_INSTRUCTIONS
                    if job["voice"] == "nova"
                    else voice.INSTRUCTIONS
                )
                if metadata.get("instructions") != expected_instructions:
                    raise ValueError(
                        "Instructions changed or provenance is unavailable"
                    )
                raw = (ready / "response.json").read_bytes()
                _, _, audio, _ = voice.validate_response(raw, job["text"] + "\n")
                if (
                    wav.name != "audio.wav"
                    or wav.read_bytes() != audio
                    or metadata.get("voice") != job["voice"]
                    or metadata.get("script_sha256") != job["text_sha256"]
                    or metadata.get("audio_sha256") != batch.digest(audio)
                    or (ready / "script.txt").read_bytes()
                    != (job["text"] + "\n").encode()
                ):
                    raise ValueError(
                        "Mapped take differs from approved source or metadata"
                    )
            except (OSError, ValueError, KeyError, TypeError) as error:
                result["rejected"][job["id"]] = str(error)
                continue
            attempt_id = uuid.uuid4().hex
            take = root / "attempts" / job["id"] / attempt_id
            attempt = {
                "id": attempt_id,
                "take_directory": str(take),
                "request_path": str(root / "requests" / f"{attempt_id}.json"),
                "status": "running",
                "imported_from": str(ready),
            }
            job["attempts"].append(attempt)
            job["status"] = "running"
            batch.save_json(root / "manifest.json", manifest)
            try:
                # Raw provider responses remain private, alongside the reused audio.
                (take / "ready").mkdir(parents=True)
                for filename in (
                    "response.json",
                    "metadata.json",
                    "script.txt",
                    "transcript.txt",
                    "audio.wav",
                ):
                    shutil.copyfile(ready / filename, take / "ready" / filename)
                batch.inspect_attempt(job, attempt)
            except Exception:
                job["status"] = attempt["status"] = "failed"
                batch.save_json(root / "manifest.json", manifest)
                raise
            result["reused"].append(job["id"])
            batch.save_json(root / "manifest.json", manifest)
        batch.summarize(manifest)
        batch.save_json(root / "manifest.json", manifest)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    setup = sub.add_parser("prepare")
    setup.add_argument("config", type=Path)
    text = sub.add_parser("transcript")
    text.add_argument("scenario", type=Path)
    seed = sub.add_parser("reuse")
    seed.add_argument("scenario", type=Path)
    seed.add_argument("batch", type=Path)
    seed.add_argument("media_manifest", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            print(prepare(args.config))
        elif args.command == "transcript":
            print(transcript(args.scenario), end="")
        else:
            print(
                json.dumps(
                    reuse(args.scenario, args.batch, args.media_manifest), indent=2
                )
            )
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"{type(error).__name__}: {error}\n")


if __name__ == "__main__":
    main()
