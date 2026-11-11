#!/usr/bin/python3
"""Plan or explicitly execute unique narration takes; never retry automatically."""

import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import uuid

import openai_voice as voice

ROOT = Path(__file__).resolve().parents[1]
VOICES = {"teacher": "cedar", "learner": "nova"}
BASELINE_USD = 0.0


def digest(data):
    return hashlib.sha256(data).hexdigest()


def now():
    return datetime.now(timezone.utc).isoformat()


def save_json(path, data):
    """Persist the coordinator journal before any worker may make a paid request."""
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as output:
        json.dump(data, output, ensure_ascii=False, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
        temporary = output.name
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


@contextmanager
def coordinator_lock(root):
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (root / ".batch.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError(
                "A batch coordinator is already using this directory"
            ) from None
        yield


def expand(document, root):
    jobs = []
    part_ids = [p["id"] for p in document["parts"]]
    if any(type(p) is not int or p < 1 for p in part_ids) or len(set(part_ids)) != len(
        part_ids
    ):
        raise ValueError("Part identifiers must be unique positive integers")

    def add(turns, prefix, part, segment, scene_id=None):
        if not turns:
            raise ValueError("A scene or standalone segment has no turns")
        for index, turn in enumerate(turns):
            speaker, text = turn["speaker"], turn["text"]
            if speaker not in VOICES or not isinstance(text, str) or not text.strip():
                raise ValueError(
                    "Every turn needs a supported speaker and nonempty text"
                )
            job_id = f"{prefix}-{index:02d}"
            jobs.append(
                {
                    "id": job_id,
                    "scene_id": scene_id,
                    "part": part,
                    "segment": segment,
                    "turn_index": index,
                    "speaker": speaker,
                    "voice": VOICES[speaker],
                    "text": text,
                    "text_sha256": digest((text + "\n").encode()),
                    "source_path": str(root / "sources" / f"{job_id}.txt"),
                    "status": "planned",
                    "attempts": [],
                }
            )

    for scene in document["scenes"]:
        if not re.fullmatch(r"\d{2}", scene["id"]) or scene["part"] not in part_ids:
            raise ValueError("Invalid scene identifier or part reference")
        add(
            scene["turns"],
            f"scene-{scene['id']}-turn",
            scene["part"],
            "scene",
            scene["id"],
        )
    for part in document["parts"]:
        for segment in ("intro", "outro"):
            add(part[segment], f"part-{part['id']:02d}-{segment}", part["id"], segment)
    if len({j["id"] for j in jobs}) != len(jobs):
        raise ValueError("Duplicate narration job identifier")
    return jobs


def usage_summary(attempts):
    totals = {
        "input_text_tokens": 0,
        "input_audio_tokens": 0,
        "output_text_tokens": 0,
        "output_audio_tokens": 0,
    }
    seen = set()
    unknown = 0
    for attempt in attempts:
        key = attempt.get("response_id") or "attempt:" + attempt["id"]
        if key in seen:
            continue
        seen.add(key)
        usage = attempt.get("usage")
        if not isinstance(usage, dict):
            unknown += 1
            continue
        incoming = (usage.get("prompt_tokens_details") or {}).get("audio_tokens", 0)
        outgoing = (usage.get("completion_tokens_details") or {}).get("audio_tokens", 0)
        totals["input_audio_tokens"] += incoming
        totals["output_audio_tokens"] += outgoing
        totals["input_text_tokens"] += max(0, usage.get("prompt_tokens", 0) - incoming)
        totals["output_text_tokens"] += max(
            0, usage.get("completion_tokens", 0) - outgoing
        )
    estimate = (
        totals["input_text_tokens"] * 2.5
        + totals["input_audio_tokens"] * 32
        + totals["output_text_tokens"] * 10
        + totals["output_audio_tokens"] * 64
    ) / 1_000_000
    return {
        **totals,
        "unique_responses": len(seen),
        "responses_without_usage": unknown,
        "estimated_batch_usd": round(estimate, 8),
        "baseline_usd": BASELINE_USD,
        "estimated_session_usd": round(BASELINE_USD + estimate, 8),
        "estimate_note": "Informative estimate; missing usage is not zero cost. Cached-input discounts are not applied. Baseline excludes previous production sessions; verify current provider prices before spending.",
        "pricing_source": "https://developers.openai.com/api/docs/models/gpt-audio-1.5",
    }


def summarize(manifest):
    jobs = manifest["jobs"]
    attempts = [a for j in jobs for a in j["attempts"]]
    manifest["summary"] = {
        **{
            s: sum(j["status"] == s for j in jobs)
            for s in ("planned", "running", "ready", "failed")
        },
        "failed_ids": [j["id"] for j in jobs if j["status"] == "failed"],
        "request_attempts": sum(Path(a["request_path"]).exists() for a in attempts),
        "usage": usage_summary(
            [a for a in attempts if Path(a["request_path"]).exists()]
        ),
    }
    manifest["updated_at"] = now()


def inspect_attempt(job, attempt):
    """Revalidate cached audio against its source and provider response, without HTTP."""
    take = Path(attempt["take_directory"])
    ready = take / "ready"
    response_path = (ready if ready.is_dir() else take / "unapproved") / "response.json"
    if response_path.is_file():
        raw = response_path.read_bytes()
        attempt["response_path"] = str(response_path)
        attempt["response_sha256"] = digest(raw)
        try:
            result = json.loads(raw)
            if isinstance(result, dict):
                attempt["response_id"] = result.get("id")
                attempt["usage"] = result.get("usage")
        except (ValueError, UnicodeError):
            pass
    if not ready.is_dir():
        raise ValueError("Attempt has no technically validated ready directory")
    script = Path(job["source_path"]).read_text(encoding="utf-8")
    if (ready / "script.txt").read_text(encoding="utf-8") != script:
        raise ValueError("Cached source does not match the narration job")
    result, transcript, audio, duration = voice.validate_response(
        response_path.read_bytes(), script
    )
    metadata = json.loads((ready / "metadata.json").read_text())
    if (
        metadata.get("voice") != job["voice"]
        or metadata.get("validation") != "passed"
        or metadata.get("script_sha256") != job["text_sha256"]
        or metadata.get("audio_sha256") != digest(audio)
        or (ready / "audio.wav").read_bytes() != audio
        or (ready / "transcript.txt").read_text() != transcript
    ):
        raise ValueError("Cached take fails source, voice or file consistency checks")
    attempt.update(
        status="ready",
        response_id=result.get("id"),
        usage=result.get("usage"),
        ready_path=str(ready),
        duration_seconds=duration,
    )
    job.update(
        status="ready",
        selected_attempt=attempt["id"],
        audio_path=str(ready / "audio.wav"),
        metadata_path=str(ready / "metadata.json"),
        ready_path=str(ready),
    )


def load_plan(source, root, source_bytes):
    expected = expand(json.loads(source_bytes), root)
    helper_sha = digest(Path(voice.__file__).read_bytes())
    path = root / "manifest.json"
    if path.exists():
        manifest = json.loads(path.read_text())
        if (
            manifest.get("script_sha256") != digest(source_bytes)
            or manifest.get("generator_sha256") != helper_sha
        ):
            raise ValueError(
                "Source or generator changed; use a separate batch directory"
            )
        if len(manifest["jobs"]) != len(expected):
            raise ValueError("Manifest job inventory differs from the source")
        for job, wanted in zip(manifest["jobs"], expected, strict=True):
            for key in wanted.keys() - {"status", "attempts"}:
                if job.get(key) != wanted[key]:
                    raise ValueError("Manifest job mapping differs from the source")
            if job["status"] not in ("planned", "running", "ready", "failed"):
                raise ValueError("Unknown manifest job status")
            if job["status"] == "planned" and job["attempts"]:
                raise ValueError("An attempted job cannot be reset to planned")
            for attempt in job["attempts"]:
                if not re.fullmatch(r"[0-9a-f]{32}", attempt["id"]):
                    raise ValueError("Invalid attempt identifier")
                if attempt["take_directory"] != str(
                    root / "attempts" / job["id"] / attempt["id"]
                ):
                    raise ValueError("Attempt path differs from its reserved mapping")
                if attempt["request_path"] != str(
                    root / "requests" / f"{attempt['id']}.json"
                ):
                    raise ValueError("Request path differs from its reserved mapping")
    else:
        if (root / "attempts").exists() or (root / "requests").exists():
            raise ValueError(
                "Existing attempts lack a manifest; reconcile them before proceeding"
            )
        manifest = {
            "schema_version": 1,
            "source": str(source),
            "script_sha256": digest(source_bytes),
            "generator_sha256": helper_sha,
            "created_at": now(),
            "automatic_retry": False,
            "jobs": expected,
        }
    (root / "sources").mkdir(exist_ok=True)
    for job in manifest["jobs"]:
        source_path = Path(job["source_path"])
        if source_path.exists():
            if digest(source_path.read_bytes()) != job["text_sha256"]:
                raise ValueError("A saved job source has changed")
        else:
            with source_path.open("x", encoding="utf-8") as output:
                output.write(job["text"] + "\n")
        if job["status"] in ("running", "ready"):
            try:
                inspect_attempt(job, job["attempts"][-1])
            except Exception as error:
                job["status"] = "failed"
                job["attempts"][-1].update(
                    status="failed", error_type=type(error).__name__
                )
                for key in (
                    "audio_path",
                    "metadata_path",
                    "ready_path",
                    "selected_attempt",
                ):
                    job.pop(key, None)
    return manifest


def run_batch(
    source,
    root,
    *,
    execute=False,
    approved_script_sha256=None,
    ids=None,
    retry_failed=None,
    part=None,
    workers=5,
    plain_prompt=False,
):
    source, root = Path(source).resolve(), Path(root).resolve()
    source_bytes = source.read_bytes()
    if execute and approved_script_sha256 != digest(source_bytes):
        raise ValueError(
            "Execution requires the exact independently approved script SHA-256"
        )
    if not 1 <= workers <= 5:
        raise ValueError("Workers must be between one and five")
    if sum(value is not None for value in (ids, retry_failed, part)) > 1:
        raise ValueError("Select only one of ids, retry-failed or part")
    with coordinator_lock(root):
        manifest = load_plan(source, root, source_bytes)
        all_ids = {j["id"] for j in manifest["jobs"]}
        selected = set(
            retry_failed
            if retry_failed is not None
            else ids
            if ids is not None
            else all_ids
        )
        if not selected <= all_ids:
            raise ValueError("Selection contains an unknown job identifier")
        if part is not None:
            selected = {j["id"] for j in manifest["jobs"] if j["part"] == part}
            if not selected:
                raise ValueError("No jobs belong to the requested part")
        if retry_failed is not None and any(
            j["status"] != "failed" for j in manifest["jobs"] if j["id"] in selected
        ):
            raise ValueError("Explicit retry accepts only failed job identifiers")
        candidates = [
            j
            for j in manifest["jobs"]
            if j["id"] in selected
            and j["status"] == ("failed" if retry_failed is not None else "planned")
        ]
        manifest["selection"] = {
            "ids": sorted(selected),
            "pending_requests": len(candidates),
            "explicit_failed_retry": retry_failed is not None,
            "plain_prompt": plain_prompt,
        }
        summarize(manifest)
        save_json(root / "manifest.json", manifest)
        if not execute:
            return manifest
        manifest["approved_script_sha256"] = approved_script_sha256
        (root / "requests").mkdir(exist_ok=True)

        def perform(job, attempt):
            def request(payload, key):
                if attempt["prompt_mode"] == "plain":
                    # Mutate the helper's payload so its metadata also records the actual task.
                    payload["messages"][-1]["content"] = (
                        "Read the following passage aloud verbatim. Do not answer questions, "
                        "explain, paraphrase, or add any words. Read only the passage "
                        "after this blank line:\n\n" + job["text"] + "\n"
                    )
                request_path = Path(attempt["request_path"])
                with request_path.open("x", encoding="utf-8") as output:
                    json.dump(
                        {
                            "created_at": now(),
                            "prompt_mode": attempt["prompt_mode"],
                            "payload": payload,
                        },
                        output,
                        ensure_ascii=False,
                    )
                    output.flush()
                    os.fsync(output.fileno())
                return voice.request_audio(payload, key)

            try:
                voice.generate_take(
                    job["source_path"],
                    attempt["take_directory"],
                    voice=job["voice"],
                    requester=request,
                )
                return None
            except Exception as error:
                return type(error).__name__

        pending = {}
        queue = iter(candidates)

        def finish(future):
            job, attempt = pending.pop(future)
            error_type = future.result()
            try:
                inspect_attempt(job, attempt)
            except Exception as error:
                job["status"] = "failed"
                attempt.update(
                    status="failed", error_type=error_type or type(error).__name__
                )
            attempt["finished_at"] = now()
            summarize(manifest)
            save_json(root / "manifest.json", manifest)

        executor = ThreadPoolExecutor(max_workers=workers)
        try:
            exhausted = False
            while pending or not exhausted:
                while len(pending) < workers and not exhausted:
                    job = next(queue, None)
                    if job is None:
                        exhausted = True
                        break
                    attempt_id = uuid.uuid4().hex
                    take = root / "attempts" / job["id"] / attempt_id
                    take.parent.mkdir(parents=True, exist_ok=True)
                    if take.exists() or take.is_symlink():
                        raise FileExistsError(
                            "Attempt destination already exists; it will not be reused"
                        )
                    attempt = {
                        "id": attempt_id,
                        "take_directory": str(take),
                        "request_path": str(root / "requests" / f"{attempt_id}.json"),
                        "status": "running",
                        "prompt_mode": "plain" if plain_prompt else "helper-default",
                        "started_at": now(),
                    }
                    job["attempts"].append(attempt)
                    job["status"] = "running"
                    summarize(manifest)
                    save_json(root / "manifest.json", manifest)
                    pending[executor.submit(perform, job.copy(), attempt.copy())] = (
                        job,
                        attempt,
                    )
                if pending:
                    completed, _ = wait(pending, return_when=FIRST_COMPLETED)
                    for future in completed:
                        finish(future)
        finally:
            executor.shutdown(wait=True, cancel_futures=True)
            for future in list(pending):
                if not future.cancelled():
                    finish(future)
        return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "source", type=Path, nargs="?", default=ROOT / "scenario/dialogue.json"
    )
    parser.add_argument("--output", type=Path, default=ROOT / "local/narration")
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--approved-script-sha256")
    parser.add_argument("--workers", type=int, default=5)
    parser.add_argument(
        "--plain-prompt",
        action="store_true",
        help="Use a verbatim-reading instruction without script delimiters",
    )
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--ids", nargs="+")
    selection.add_argument("--retry-failed", nargs="+")
    selection.add_argument("--part", type=int)
    args = parser.parse_args()
    try:
        manifest = run_batch(
            args.source,
            args.output,
            execute=args.execute,
            approved_script_sha256=args.approved_script_sha256,
            workers=args.workers,
            ids=args.ids,
            retry_failed=args.retry_failed,
            part=args.part,
            plain_prompt=args.plain_prompt,
        )
    except (OSError, ValueError, RuntimeError, KeyError, TypeError) as error:
        parser.exit(
            1,
            f"Batch stopped ({type(error).__name__}); inspect the manifest and selection. No automatic retry.\n",
        )
    print(
        json.dumps(
            {
                "manifest": str(args.output.resolve() / "manifest.json"),
                "selection": manifest["selection"],
                "summary": manifest["summary"],
            },
            indent=2,
        )
    )
    if any(
        j["status"] == "failed"
        for j in manifest["jobs"]
        if j["id"] in manifest["selection"]["ids"]
    ):
        parser.exit(1)


if __name__ == "__main__":
    main()
