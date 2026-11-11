#!/usr/bin/python3
"""Manifest-driven Full HD tutorial delivery; no GUI, API, or invented input footage."""

import argparse
import hashlib
import html
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import wave

FPS = 30
RANGES = [(0, 9), (10, 18), (19, 29), (30, 44), (45, 49), (50, 53)]
VOICES = {"cedar", "onyx", "echo", "ash", "ballad"}
NOTICE = "This presentation was generated entirely by AI and may contain mistakes. Dash Testnet demonstration."


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def probe(path):
    return json.loads(
        subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-show_chapters",
                "-of",
                "json",
                str(path),
            ],
            text=True,
        )
    )


def timestamp(seconds, vtt=False):
    value = round(seconds * 1000)
    hours, value = divmod(value, 3600000)
    minutes, value = divmod(value, 60000)
    secs, millis = divmod(value, 1000)
    return f"{hours:02}:{minutes:02}:{secs:02}{'.' if vtt else ','}{millis:03}"


def captions(cues, vtt=False):
    blocks = ["WEBVTT\n"] if vtt else []
    for index, cue in enumerate(cues, 1):
        label = cue["speaker"] + ": " + cue["text"]
        text = textwrap.fill(
            label, width=42, break_long_words=False, break_on_hyphens=False
        )
        if vtt:
            text = html.escape(text, quote=False)
        blocks.append(
            f"{index}\n{timestamp(cue['start'], vtt)} --> {timestamp(cue['end'], vtt)}\n{text}\n"
        )
    return "\n".join(blocks) + "\n"


def chunk_turn(text, speaker, start, duration, turn_index):
    words, chunks, current = text.split(), [], []
    for word in words:
        proposed = current + [word]
        lines = textwrap.wrap(
            speaker + ": " + " ".join(proposed),
            width=42,
            break_long_words=False,
            break_on_hyphens=False,
        )
        if len(lines) > 2 and current:
            chunks.append(current)
            current = [word]
        else:
            current = proposed
    if current:
        chunks.append(current)
    if len(chunks) > 1:
        minimum_words = math.ceil(0.8 * len(words) / duration - 1e-9)
        while len(chunks[-1]) < minimum_words:
            previous, tail = chunks[-2], chunks[-1]
            candidate = [previous[-1]] + tail
            fits = (
                len(
                    textwrap.wrap(
                        speaker + ": " + " ".join(candidate),
                        width=42,
                        break_long_words=False,
                        break_on_hyphens=False,
                    )
                )
                <= 2
            )
            if len(previous) <= minimum_words or not fits:
                break
            chunks[-2] = previous[:-1]
            chunks[-1] = candidate
    result, offset = [], 0
    for chunk in chunks:
        count = len(chunk)
        result.append(
            {
                "start": start + duration * offset / len(words),
                "end": start + duration * (offset + count) / len(words),
                "speaker": speaker,
                "text": " ".join(chunk),
                "turn": turn_index,
            }
        )
        offset += count
    return result


def rebalance_cues(cues):
    groups = []
    for cue in cues:
        key = (cue["segment"], cue["turn"], cue["speaker"])
        if not groups or groups[-1][0] != key:
            groups.append((key, []))
        groups[-1][1].append(cue)
    balanced = []
    for (segment, turn, speaker), group in groups:
        text = " ".join(cue["text"] for cue in group)
        start, end = group[0]["start"], group[-1]["end"]
        balanced.extend(
            dict(cue, segment=segment)
            for cue in chunk_turn(text, speaker, start, end - start, turn)
        )
    return balanced


def init_manifest(scenario_path, target):
    scenario_path, target = Path(scenario_path).resolve(), Path(target)
    scenario = read_json(scenario_path)
    root = Path(__file__).resolve().parents[2]
    slides = root / "assets/slides"
    manifest = {
        "schema_version": 1,
        "scenario": str(scenario_path),
        "scenario_sha256": hashlib.sha256(scenario_path.read_bytes()).hexdigest(),
        "teacher_voice": "cedar",
        "learner_voice": "nova",
        "turn_gap_seconds": 0.2,
        "title_lead_seconds": 3,
        "synthetic_test": False,
        "credits": str(root / "assets/ATTRIBUTION.txt"),
        "font_license": str(slides / "montserrat-ofl.txt"),
        "segments": {},
    }
    for scene in scenario["scenes"]:
        sid = scene["id"]
        visual = {"kind": "video", "path": None, "offset": 0, "hold_last_frame": False}
        if sid in ("00", "53"):
            visual = {
                "kind": "slide",
                "path": str(
                    slides
                    / (
                        "opening-1920x1080.png"
                        if sid == "00"
                        else "closing-1920x1080.png"
                    )
                ),
            }
        manifest["segments"][sid] = {
            "visual": visual,
            "audio": [None] * len(scene["turns"]),
        }
    for part in scenario["parts"]:
        for kind in ("intro", "outro"):
            manifest["segments"][f"p{part['id']}-{kind}"] = {
                "visual": {
                    "kind": "slide",
                    "path": str(
                        slides
                        / (
                            "opening-1920x1080.png"
                            if kind == "intro"
                            else "closing-1920x1080.png"
                        )
                    ),
                },
                "audio": [None] * len(part[kind]),
            }
    with target.open("x", encoding="utf-8") as out:
        json.dump(manifest, out, ensure_ascii=False, indent=2)
        out.write("\n")


def load_plan(manifest_path, part=None):
    manifest_path = Path(manifest_path).resolve()
    manifest = read_json(manifest_path)
    if manifest.get("teacher_voice") not in VOICES:
        raise ValueError("Set teacher_voice to the selected supported teacher voice")
    if manifest.get("learner_voice") != "nova":
        raise ValueError("learner_voice must be nova")
    base = manifest_path.parent

    def existing(value, context):
        if not isinstance(value, str) or not value:
            raise ValueError(f"Missing input: {context}")
        path = (base / value).resolve()
        if not path.is_file():
            raise ValueError(f"Missing input: {context}: {path}")
        return path

    scenario_path = existing(manifest.get("scenario"), "scenario")
    if hashlib.sha256(scenario_path.read_bytes()).hexdigest() != manifest.get(
        "scenario_sha256"
    ):
        raise ValueError("Scenario hash changed; review and regenerate the manifest")
    scenario = read_json(scenario_path)
    if [s["id"] for s in scenario["scenes"]] != [f"{i:02}" for i in range(54)]:
        raise ValueError("Expected exactly scenes 00 through 53 in order")
    if [(int(p["start"]), int(p["end"])) for p in scenario["parts"]] != RANGES:
        raise ValueError("The six part ranges do not match the agreed series")
    credits = existing(manifest.get("credits"), "official brand credits")
    license_path = existing(manifest.get("font_license"), "font license")
    gap = float(manifest.get("turn_gap_seconds", 0.2))
    if not math.isfinite(gap) or not 0 <= gap <= 5:
        raise ValueError("turn_gap_seconds must be between zero and five")
    specs = [(s["id"], s["title"], s["turns"]) for s in scenario["scenes"]]
    if part is not None:
        begin, end = RANGES[part - 1]
        specs = [spec for spec in specs if begin <= int(spec[0]) <= end]
    for part_spec in scenario["parts"]:
        if part is not None and part_spec["id"] != part:
            continue
        for kind in ("intro", "outro"):
            specs.append(
                (
                    f"p{part_spec['id']}-{kind}",
                    f"Part {part_spec['id']}: {part_spec['title']} — {kind}",
                    part_spec[kind],
                )
            )
    segments = {}
    for sid, title, turns in specs:
        entry = manifest.get("segments", {}).get(sid)
        if not isinstance(entry, dict) or len(entry.get("audio", [])) != len(turns):
            raise ValueError(
                f"Segment {sid}: audio count must equal the script turn count"
            )
        audio, cues, elapsed, params = [], [], 0.0, None
        for index, (turn, value) in enumerate(zip(turns, entry["audio"], strict=True)):
            if "<" in turn["text"] or ">" in turn["text"]:
                raise ValueError(
                    f"Segment {sid}: literal angle brackets need a reviewed subtitle encoding extension; SRT may drop them"
                )
            path = existing(value, f"segment {sid}, turn {index + 1} audio")
            stored_script = existing(
                str(path.with_name("script.txt")), f"{sid} turn script"
            )
            transcript = existing(
                str(path.with_name("transcript.txt")), f"{sid} turn transcript"
            )
            metadata = read_json(
                existing(str(path.with_name("metadata.json")), f"{sid} turn metadata")
            )
            expected = turn["text"].split()
            if (
                stored_script.read_text().split() != expected
                or transcript.read_text().split() != expected
            ):
                raise ValueError(
                    f"Segment {sid}, turn {index + 1}: script/transcript mismatch"
                )
            expected_voice = (
                manifest["teacher_voice"] if turn["speaker"] == "teacher" else "nova"
            )
            if (
                metadata.get("validation") != "passed"
                or metadata.get("voice") != expected_voice
            ):
                raise ValueError(
                    f"Segment {sid}, turn {index + 1}: voice or validation mismatch"
                )
            if (
                metadata.get("audio_sha256")
                != hashlib.file_digest(path.open("rb"), "sha256").hexdigest()
                or metadata.get("script_sha256")
                != hashlib.sha256(stored_script.read_bytes()).hexdigest()
            ):
                raise ValueError(
                    f"Segment {sid}, turn {index + 1}: audio/script checksum mismatch"
                )
            with wave.open(str(path), "rb") as wav:
                current = (wav.getnchannels(), wav.getsampwidth(), wav.getframerate())
                count = wav.getnframes()
                if (
                    current[:2] != (1, 2)
                    or count <= 0
                    or len(wav.readframes(count)) != count * 2
                ):
                    raise ValueError(
                        f"Segment {sid}: expected a complete nonempty mono 16-bit PCM WAV"
                    )
                if params is not None and current != params:
                    raise ValueError(f"Segment {sid}: turn WAV sample formats differ")
                params = current
                duration = count / current[2]
            speaker = scenario["speakers"][turn["speaker"]]["name"]
            cues += chunk_turn(turn["text"], speaker, elapsed, duration, index)
            audio.append(
                {
                    "path": path,
                    "duration": duration,
                    "frames": count,
                    "text": turn["text"],
                    "speaker": speaker,
                }
            )
            elapsed += duration + (gap if index < len(turns) - 1 else 0)
        frames = math.ceil(elapsed * FPS - 1e-8)
        duration = frames / FPS
        visual = dict(entry["visual"])
        visual["path"] = existing(visual.get("path"), f"segment {sid} visual")
        media = probe(visual["path"])
        streams = [
            stream for stream in media["streams"] if stream["codec_type"] == "video"
        ]
        if not streams:
            raise ValueError(f"Segment {sid}: visual has no video/image stream")
        if visual["kind"] == "slide":
            if (streams[0]["width"], streams[0]["height"]) != (1920, 1080):
                raise ValueError(f"Segment {sid}: slide must be native 1920×1080")
            hold = 0
        elif visual["kind"] == "video":
            offset = float(visual.get("offset", 0))
            available = float(media["format"]["duration"]) - offset
            if not math.isfinite(offset) or offset < 0 or available <= 0:
                raise ValueError(f"Segment {sid}: invalid video offset/duration")
            hold = max(0, duration - available)
            if hold > 0.04 and visual.get("hold_last_frame") is not True:
                raise ValueError(
                    f"Segment {sid}: recording is {hold:.3f}s too short; supply footage or explicitly permit a real last-frame hold"
                )
        else:
            raise ValueError(f"Segment {sid}: visual kind must be video or slide")
        segments[sid] = {
            "id": sid,
            "title": title,
            "turns": audio,
            "params": params,
            "duration": duration,
            "frames": frames,
            "cues": cues,
            "visual": visual,
            "editorial_hold_seconds": hold,
        }
    lead = float(manifest.get("title_lead_seconds", 0))
    if (
        not math.isfinite(lead)
        or not 0 <= lead <= 30
        or abs(lead * FPS - round(lead * FPS)) > 1e-6
    ):
        raise ValueError(
            "title_lead_seconds must be between zero and 30 and use whole frames"
        )
    if lead:
        first = segments[f"p{part}-intro" if part is not None else "00"]
        if first["visual"]["kind"] != "slide":
            raise ValueError("A title lead requires an opening slide")
        segments["title-lead"] = {
            "id": "title-lead",
            "title": "AI disclosure",
            "turns": [],
            "params": (1, 2, 48000),
            "duration": lead,
            "frames": round(lead * FPS),
            "cues": [],
            "visual": dict(first["visual"], fade=False),
            "editorial_hold_seconds": lead,
        }
    return {
        "manifest": manifest,
        "scenario": scenario,
        "segments": segments,
        "credits": credits,
        "font_license": license_path,
        "gap": gap,
        "selected_part": part,
    }


def import_audio(media_manifest, audio_manifest, output, part=None):
    media_manifest, audio_manifest, output = map(
        lambda p: Path(p).resolve(), (media_manifest, audio_manifest, output)
    )
    manifest, narration = read_json(media_manifest), read_json(audio_manifest)

    def rebase(value, base):
        return (
            os.path.relpath((base / value).resolve(), output.parent) if value else value
        )

    for key in ("scenario", "credits", "font_license"):
        if key in manifest:
            manifest[key] = rebase(manifest[key], media_manifest.parent)
    for segment in manifest["segments"].values():
        segment["visual"]["path"] = rebase(
            segment["visual"].get("path"), media_manifest.parent
        )
        segment["audio"] = [
            rebase(value, media_manifest.parent) for value in segment["audio"]
        ]
    if (narration.get("script_sha256") or narration.get("source_sha256")) != manifest[
        "scenario_sha256"
    ]:
        raise ValueError("Narration and media scenario hashes differ")
    jobs = {}
    for job in narration["jobs"]:
        job_id = job["id"]
        if "segment_id" in job:
            sid, index = job["segment_id"], int(job["turn_index"])
            if sid.startswith("p"):
                pid, kind = sid[1:].split("-")
                job_id = f"part-{int(pid):02}-{kind}-{index:02}"
            else:
                job_id = f"scene-{sid}-turn-{index:02}"
        if job_id in jobs:
            raise ValueError(f"Duplicate narration job: {job_id}")
        jobs[job_id] = job
    for sid, segment in manifest["segments"].items():
        if part is not None:
            if sid.startswith("p"):
                if not sid.startswith(f"p{part}-"):
                    continue
            else:
                begin, end = RANGES[part - 1]
                if not begin <= int(sid) <= end:
                    continue
        for index in range(len(segment["audio"])):
            if sid.startswith("p"):
                pid, kind = sid[1:].split("-")
                job_id = f"part-{int(pid):02}-{kind}-{index:02}"
            else:
                job_id = f"scene-{sid}-turn-{index:02}"
            job = jobs.get(job_id)
            if not job or job.get("status") != "ready":
                raise ValueError(f"Narration job is not ready: {job_id}")
            audio_path = job.get("audio_path")
            if not audio_path and job.get("take"):
                audio_path = str(Path(job["take"]) / "ready/audio.wav")
            if not audio_path:
                raise ValueError(f"Narration job has no audio path: {job_id}")
            segment["audio"][index] = rebase(audio_path, audio_manifest.parent)
    with Path(output).open("x", encoding="utf-8") as out:
        json.dump(manifest, out, ensure_ascii=False, indent=2)
        out.write("\n")


def ffmpeg(args, log):
    command = [
        "ffmpeg",
        "-nostdin",
        "-n",
        "-hide_banner",
        "-loglevel",
        "warning",
        "-threads",
        "2",
        "-filter_threads",
        "2",
        "-filter_complex_threads",
        "2",
    ] + list(map(str, args))
    log.write(json.dumps(command) + "\n")
    log.flush()
    subprocess.run(command, check=True, stdout=log, stderr=log, timeout=7200)


def render_segment(segment, scratch, gap, preset, log):
    sid, duration = segment["id"], segment["duration"]
    rate = segment["params"][2]
    raw = scratch / f"{sid}-input.wav"
    with wave.open(str(raw), "wb") as out:
        out.setparams((1, 2, rate, 0, "NONE", "not compressed"))
        written = 0
        for index, turn in enumerate(segment["turns"]):
            with wave.open(str(turn["path"]), "rb") as audio:
                out.writeframesraw(audio.readframes(audio.getnframes()))
            written += turn["frames"]
            if index < len(segment["turns"]) - 1:
                silence = round(rate * gap)
                out.writeframesraw(bytes(silence * 2))
                written += silence
        out.writeframesraw(bytes(max(0, round(duration * rate) - written) * 2))
    canonical = scratch / f"{sid}.wav"
    ffmpeg(["-i", raw, "-ar", "48000", "-ac", "1", "-c:a", "pcm_s16le", canonical], log)
    visual = segment["visual"]
    args = (
        ["-loop", "1", "-framerate", str(FPS), "-i", visual["path"]]
        if visual["kind"] == "slide"
        else ["-ss", str(visual.get("offset", 0)), "-i", visual["path"]]
    )
    filters = "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=0x0b1629,setsar=1,fps=30"
    if visual["kind"] == "slide" and visual.get("fade", True):
        fade = min(0.4, duration / 4)
        filters += f",fade=t=in:st=0:d={fade},fade=t=out:st={duration - fade}:d={fade}"
    elif visual.get("hold_last_frame") is True:
        filters += f",tpad=stop_mode=clone:stop_duration={duration}"
    video = scratch / f"{sid}.mp4"
    ffmpeg(
        args
        + [
            "-vf",
            filters,
            "-frames:v",
            str(segment["frames"]),
            "-an",
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-preset",
            preset,
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            video,
        ],
        log,
    )
    actual = probe(video)
    if abs(float(actual["format"]["duration"]) - duration) > 0.04:
        raise ValueError(
            f"Segment {sid}: rendered video duration differs from narration plan"
        )
    with wave.open(str(canonical), "rb") as audio:
        if abs(audio.getnframes() / 48000 - duration) > 0.001:
            raise ValueError(
                f"Segment {sid}: narration resampling changed the timeline"
            )
    return video, canonical


def metadata_escape(value):
    return (
        str(value)
        .replace("\\", "\\\\")
        .replace("\n", " ")
        .replace("=", "\\=")
        .replace(";", "\\;")
        .replace("#", "\\#")
    )


def package_transcript(plan, ids):
    return (
        "\n\n".join(
            plan["segments"][sid]["title"]
            + "\n"
            + "\n".join(
                turn["speaker"] + ": " + turn["text"]
                for turn in plan["segments"][sid]["turns"]
            )
            for sid in ids
        )
        + "\n"
    )


def concat_file(path):
    """Quote one ffconcat token using FFmpeg's own single-quote grammar."""
    value = str(path)
    if "\n" in value or "\r" in value:
        raise ValueError("ffconcat paths cannot contain line breaks")
    return "file '" + value.replace("'", "'\\''") + "'\n"


def package_video(name, ids, plan, rendered, scratch, dest, log):
    (dest / f"{name}.transcript.txt").write_text(
        package_transcript(plan, ids), encoding="utf-8"
    )
    if "title-lead" in plan["segments"]:
        ids = ["title-lead", *ids]
    timeline, cues, chapters, offset = (
        [],
        [],
        [";FFMETADATA1", f"title={metadata_escape(name)}", f"comment={NOTICE}"],
        0.0,
    )
    listing, joined = scratch / f"{name}.ffconcat", scratch / f"{name}.wav"
    listing.write_text(
        "ffconcat version 1.0\n" + "".join(concat_file(rendered[sid][0]) for sid in ids)
    )
    with wave.open(str(joined), "wb") as out:
        out.setparams((1, 2, 48000, 0, "NONE", "not compressed"))
        for sid in ids:
            segment = plan["segments"][sid]
            with wave.open(str(rendered[sid][1]), "rb") as audio:
                out.writeframesraw(audio.readframes(audio.getnframes()))
            cues += [
                dict(
                    cue,
                    start=cue["start"] + offset,
                    end=cue["end"] + offset,
                    segment=sid,
                )
                for cue in segment["cues"]
            ]
            end = offset + segment["duration"]
            chapters += [
                "[CHAPTER]",
                "TIMEBASE=1/1000",
                f"START={round(offset * 1000)}",
                f"END={round(end * 1000)}",
                f"title={metadata_escape(segment['title'])}",
            ]
            timeline.append(
                {
                    "segment": sid,
                    "start": offset,
                    "end": end,
                    "editorial_hold_seconds": segment["editorial_hold_seconds"],
                }
            )
            offset = end
    srt, vtt, meta = (
        dest / f"{name}.en.srt",
        dest / f"{name}.en.vtt",
        scratch / f"{name}.ffmeta",
    )
    srt.write_text(captions(cues), encoding="utf-8")
    vtt.write_text(captions(cues, vtt=True), encoding="utf-8")
    meta.write_text("\n".join(chapters) + "\n", encoding="utf-8")
    output = dest / f"{name}.mp4"
    ffmpeg(
        [
            "-safe",
            "0",
            "-f",
            "concat",
            "-i",
            listing,
            "-i",
            joined,
            "-i",
            srt,
            "-f",
            "ffmetadata",
            "-i",
            meta,
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-map",
            "2:s:0",
            "-map_metadata",
            "3",
            "-map_chapters",
            "3",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-threads",
            "2",
            "-c:s",
            "mov_text",
            "-metadata:s:s:0",
            "language=eng",
            "-metadata:s:s:0",
            "title=English — speaker labels",
            "-movflags",
            "+faststart",
            output,
        ],
        log,
    )
    info = probe(output)
    actual = float(info["format"]["duration"])
    if abs(actual - offset) > 0.08 or max(cue["end"] for cue in cues) > actual + 0.001:
        raise ValueError(f"{name}: final duration/captions drift exceeds tolerance")
    result = {
        "file": output.name,
        "planned_duration": offset,
        "actual_duration": actual,
        "timeline": timeline,
        "cues": cues,
        "chapters": len(info["chapters"]),
    }
    (dest / f"{name}.timeline.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    return result


def watch_page(plan, packages, dest):
    esc = html.escape
    synthetic = plan["manifest"].get("synthetic_test", False)
    title = ("SYNTHETIC PIPELINE CHECK — " if synthetic else "") + plan["scenario"][
        "title"
    ]
    cards = []
    default_key = "combined" if "combined" in packages else next(iter(packages))
    for key, item in packages.items():
        label = "Full tutorial" if key == "combined" else f"Part {key.split('-')[-1]}"
        cards.append(
            f'<button data-video="{item["file"]}" data-captions="{key}.en.vtt">{label}</button> <a download href="{item["file"]}">MP4</a> <a download href="{key}.en.srt">SRT</a> <a download href="{key}.en.vtt">VTT</a>'
        )
    chapter_links = "".join(
        f'<li><button data-seek="{item["start"]:.6f}">{esc(plan["segments"][item["segment"]]["title"])}</button></li>'
        for item in packages[default_key]["timeline"]
    )
    page = f"""<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{esc(title)}</title>
+<style>body{{margin:0;background:#0b1629;color:#ecf5ff;font:17px system-ui}}main{{max-width:1180px;margin:auto;padding:32px}}h1{{font-size:32px}}video{{width:100%;border-radius:12px;background:#000}}button,a{{font:inherit;color:#b8e6ff}}button{{border:1px solid #356081;background:#122944;padding:8px 12px;border-radius:8px;cursor:pointer}}nav{{display:grid;gap:12px;margin:22px 0}}li{{margin:8px 0}}small{{color:#b6cce5}}a{{margin:0 8px}}</style>
+<main><h1>{esc(title)}</h1><p>{esc(NOTICE)}</p><video controls preload="metadata" src="combined.mp4"><track default kind="subtitles" label="English — speaker labels" srclang="en" src="combined.en.vtt"></video><nav>{"".join("<div>" + card + "</div>" for card in cards)}</nav><details><summary>Full tutorial chapters</summary><ol>{chapter_links}</ol></details><p><a href="transcript.txt" download>Transcript</a><a href="credits.txt">Brand and font credits</a><a href="checksums.sha256">Checksums</a></p><small>Dash Evolution logo © Dash, <a href="https://creativecommons.org/licenses/by/4.0/">CC BY 4.0</a>, <a href="https://www.dash.org/brand-guidelines/">Dash brand guidelines</a>.</small></main>
+<script>const video=document.querySelector('video');document.querySelectorAll('[data-video]').forEach(b=>b.onclick=()=>{{video.src=b.dataset.video;video.querySelector('track').src=b.dataset.captions;video.load()}});document.querySelectorAll('[data-seek]').forEach(b=>b.onclick=()=>{{if(!video.getAttribute('src').endsWith('combined.mp4')){{video.src='combined.mp4';video.querySelector('track').src='combined.en.vtt';video.load();video.addEventListener('loadedmetadata',()=>{{video.currentTime=+b.dataset.seek;video.play()}},{{once:true}})}}else{{video.currentTime=+b.dataset.seek;video.play()}}}})</script></html>"""
    page = page.replace("combined.mp4", f"{default_key}.mp4").replace(
        "combined.en.vtt", f"{default_key}.en.vtt"
    )
    (dest / "index.html").write_text(page.replace("\n+", "\n"), encoding="utf-8")


def build(manifest_path, output, preset="veryfast", part=None):
    plan = load_plan(manifest_path, part)
    output = Path(output)
    output.mkdir(mode=0o700)
    pending = output / "pending"
    pending.mkdir()
    (output / "INCOMPLETE.txt").write_text(
        "Only a ready/ directory represents a complete delivery.\n"
    )
    with (
        (output / "ffmpeg.log").open("w") as log,
        tempfile.TemporaryDirectory(prefix="tutorial-media-") as work,
    ):
        scratch, rendered = Path(work), {}
        for sid, segment in plan["segments"].items():
            print(f"Rendering {sid} ({segment['duration']:.3f}s)", flush=True)
            rendered[sid] = render_segment(segment, scratch, plan["gap"], preset, log)
        packages = {}
        for part_spec in plan["scenario"]["parts"]:
            pid = part_spec["id"]
            if part is not None and pid != part:
                continue
            ids = (
                [f"p{pid}-intro"]
                + [s["id"] for s in plan["scenario"]["scenes"] if s["part"] == pid]
                + [f"p{pid}-outro"]
            )
            packages[f"part-{pid:02}"] = package_video(
                f"part-{pid:02}", ids, plan, rendered, scratch, pending, log
            )
        if part is None:
            packages["combined"] = package_video(
                "combined",
                [s["id"] for s in plan["scenario"]["scenes"]],
                plan,
                rendered,
                scratch,
                pending,
                log,
            )
        shutil.copyfile(manifest_path, output / "input-manifest.json")
        shutil.copyfile(plan["credits"], pending / "credits.txt")
        shutil.copyfile(plan["font_license"], pending / "font-license.txt")
        slide_ids = (
            (("opening", "00"), ("closing", "53"))
            if part is None
            else (("opening", f"p{part}-intro"), ("closing", f"p{part}-outro"))
        )
        for kind, sid in slide_ids:
            shutil.copyfile(
                plan["segments"][sid]["visual"]["path"], pending / f"{kind}-slide.png"
            )
        default_package = "combined" if "combined" in packages else next(iter(packages))
        shutil.copyfile(
            pending / f"{default_package}.transcript.txt", pending / "transcript.txt"
        )
        watch_page(plan, packages, pending)
        report = {
            "synthetic_test": plan["manifest"].get("synthetic_test", False),
            "scenario_sha256": plan["manifest"]["scenario_sha256"],
            "caption_timing": "Actual WAV turn boundaries; within-turn chunks proportional to word counts, not word-aligned",
            "packages": packages,
        }
        (pending / "delivery.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2) + "\n"
        )
        checksums = "".join(
            f"{hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()}  {path.name}\n"
            for path in sorted(pending.iterdir())
            if path.is_file()
        )
        (pending / "checksums.sha256").write_text(checksums)
    pending.rename(output / "ready")
    print(f"Delivery complete: {output / 'ready'}", flush=True)
    return output / "ready"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    init = sub.add_parser("init")
    init.add_argument("scenario", type=Path)
    init.add_argument("manifest", type=Path)
    validate = sub.add_parser("plan")
    validate.add_argument("manifest", type=Path)
    imported = sub.add_parser("import-audio")
    imported.add_argument("manifest", type=Path)
    imported.add_argument("narration_manifest", type=Path)
    imported.add_argument("output", type=Path)
    render = sub.add_parser("build")
    render.add_argument("manifest", type=Path)
    render.add_argument("output", type=Path)
    render.add_argument(
        "--preset", choices=("ultrafast", "veryfast", "fast"), default="veryfast"
    )
    for command_parser in (validate, imported, render):
        selection = command_parser.add_mutually_exclusive_group()
        selection.add_argument("--part", type=int, choices=range(1, 7))
        selection.add_argument("--part1", dest="part", action="store_const", const=1)
    args = parser.parse_args()
    try:
        if args.command == "init":
            init_manifest(args.scenario, args.manifest)
        elif args.command == "plan":
            plan = load_plan(args.manifest, args.part)
            print(
                json.dumps(
                    {
                        key: {
                            "duration": value["duration"],
                            "turns": len(value["turns"]),
                            "caption_chunks": len(value["cues"]),
                            "editorial_hold_seconds": value["editorial_hold_seconds"],
                        }
                        for key, value in plan["segments"].items()
                    },
                    indent=2,
                )
            )
        elif args.command == "import-audio":
            import_audio(args.manifest, args.narration_manifest, args.output, args.part)
        else:
            build(args.manifest, args.output, args.preset, args.part)
    except (
        OSError,
        ValueError,
        KeyError,
        wave.Error,
        subprocess.SubprocessError,
    ) as error:
        parser.exit(1, f"Media pipeline failed: {error}\n")


if __name__ == "__main__":
    main()
