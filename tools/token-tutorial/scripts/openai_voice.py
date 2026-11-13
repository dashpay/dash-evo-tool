#!/usr/bin/python3
"""Generate one unique narration take; default CLI mode is a free preflight."""

import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shlex
import struct
import urllib.error
import urllib.request
import wave

MODEL = "gpt-audio-1.5"
VOICES = ("cedar", "onyx", "echo", "ash", "ballad", "nova")
INSTRUCTIONS = (
    "Speak as a natural, relaxed male English narrator explaining software to a colleague. "
    "Use a comfortable medium-low register, clear everyday delivery and a moderate pace. "
    "Keep emphasis and pitch changes small. No theatrical performance, announcer voice, "
    "exaggerated pauses or forced amusement. Any humor is understated and conversational. "
    "Speak only the supplied script, verbatim, without adding words or pronunciation notes. "
    "The teacher is Chad G. P. Taylor. Pronounce G and P as individual English letters. "
    "Do not add his name unless it appears in the supplied script."
)
LEARNER_INSTRUCTIONS = (
    "Speak in fluent English as a natural, curious female student asking a teacher about software. "
    "Use relaxed conversational delivery, a clear moderate pace and small, natural pitch changes. "
    "Sound interested and thoughtful, with no theatrical performance, announcer voice, "
    "exaggerated pauses, caricature or forced surprise. Speak only the supplied script, verbatim. "
    "Do not add words or pronunciation notes. The learner is Ewa Dashwood. "
    "Pronounce Ewa as EH-vah and Dashwood as DASH-wood, without exaggeration. "
    "Do not add her name unless it appears in the supplied script."
)


class VoiceGenerationError(Exception):
    """The attempted take is retained but has not passed technical validation."""


class ApiResponseError(Exception):
    def __init__(self, status, body):
        self.status = status
        self.body = body
        super().__init__(f"HTTP {status}")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def load_key(path=None):
    if path is None:
        key = os.environ.get("OPENAI_API_KEY", "").strip()
        if not key:
            raise ValueError("Set OPENAI_API_KEY or explicitly pass --key-file")
        return key
    key = None
    for line in Path(path).read_text().splitlines():
        match = re.match(r"\s*(?:export\s+)?OPENAI_API_KEY\s*=\s*(.*)", line)
        if match:
            values = shlex.split(match.group(1), comments=True)
            if values:
                key = values[0]
    if not key:
        raise ValueError("The configured API key is unavailable")
    return key


def request_audio(payload, key):
    request = urllib.request.Request(
        "https://api.openai.com/v1/chat/completions",
        data=json.dumps(payload).encode(),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.build_opener(NoRedirect()).open(
            request, timeout=180
        ) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        raise ApiResponseError(error.code, error.read()) from None


def preflight(source, take, voice):
    source, take = Path(source), Path(take)
    if take.exists() or take.is_symlink():
        raise FileExistsError(
            "Take destination already exists; choose a new take directory"
        )
    if voice not in VOICES:
        raise ValueError("Unsupported voice")
    if not take.parent.is_dir():
        raise ValueError("Take parent directory must already exist")
    script = source.read_text(encoding="utf-8")
    if not script.strip():
        raise ValueError("Narration script is empty")
    return script


def normalize_wav(data):
    if len(data) >= 44 and data[:4] == b"RIFF" and data[8:12] == b"WAVE":
        unknown_riff = data[4:8] == b"\xff" * 4
        unknown_data = data[36:40] == b"data" and data[40:44] == b"\xff" * 4
        if unknown_riff or unknown_data:
            if not (unknown_riff and unknown_data and data[12:20] == b"fmt \x10\0\0\0"):
                raise ValueError("Unsupported unknown-size WAV header")
            codec, channels, rate, byte_rate, alignment, bits = struct.unpack_from(
                "<HHIIHH", data, 20
            )
            expected_alignment = channels * (bits // 8)
            if (
                codec != 1
                or channels != 1
                or rate <= 0
                or bits != 16
                or alignment != expected_alignment
                or byte_rate != rate * alignment
                or len(data) == 44
                or (len(data) - 44) % alignment
            ):
                raise ValueError(
                    "Unknown-size WAV does not contain complete PCM frames"
                )
            normalized = bytearray(data)
            struct.pack_into("<I", normalized, 4, len(data) - 8)
            struct.pack_into("<I", normalized, 40, len(data) - 44)
            return bytes(normalized)
    return data


def validate_response(raw, script):
    result = json.loads(raw)
    choice = result["choices"][0]
    if choice["finish_reason"] != "stop":
        raise ValueError("Audio generation did not finish normally")
    audio = choice["message"]["audio"]
    transcript = audio["transcript"]
    if not isinstance(transcript, str) or transcript.split() != script.split():
        raise ValueError("Transcript does not match the script verbatim")
    data = normalize_wav(base64.b64decode(audio["data"], validate=True))
    if (
        len(data) < 44
        or data[:4] != b"RIFF"
        or data[8:12] != b"WAVE"
        or struct.unpack_from("<I", data, 4)[0] != len(data) - 8
    ):
        raise ValueError("WAV container size does not match its contents")
    with wave.open(io.BytesIO(data), "rb") as wav:
        frames = wav.getnframes()
        expected = frames * wav.getnchannels() * wav.getsampwidth()
        if (
            (wav.getnchannels(), wav.getsampwidth()) != (1, 2)
            or frames <= 0
            or len(wav.readframes(frames)) != expected
        ):
            raise ValueError("WAV must contain complete nonempty mono 16-bit PCM")
        duration = frames / wav.getframerate()
    return result, transcript, data, duration


def generate_take(source, take, *, voice="cedar", key_loader=None, requester=None):
    """Reserve a unique directory before credentials, then atomically publish ready/."""

    def fetch(payload):
        key = (key_loader or load_key)()
        return (requester or request_audio)(payload, key)

    return _write_take(source, take, voice, fetch)


def recover_take(source, failed_take, take, *, voice="cedar"):
    """Validate and republish a saved paid response offline, preserving its original take."""
    script = preflight(source, take, voice)
    cached = Path(failed_take) / "unapproved"
    if (cached / "script.txt").read_text(encoding="utf-8") != script:
        raise ValueError(
            "Recovery source differs from the script stored with this response"
        )
    raw = (cached / "response.json").read_bytes()
    return _write_take(
        source,
        take,
        voice,
        lambda _: raw,
        script=script,
        recovery_from=cached / "response.json",
    )


def _write_take(source, take, voice, fetch, *, script=None, recovery_from=None):
    source, take = Path(source), Path(take)
    if script is None:
        script = preflight(source, take, voice)
    take.mkdir(mode=0o700)
    pending = take / "unapproved"
    try:
        pending.mkdir(mode=0o700)
        (take / "UNAPPROVED.txt").write_text(
            "This take requires human listening approval. Only a complete ready/ directory "
            "has passed technical checks. An unapproved/ directory is incomplete or failed.\n",
            encoding="utf-8",
        )
        (pending / "script.txt").write_text(script, encoding="utf-8")
        instructions = LEARNER_INSTRUCTIONS if voice == "nova" else INSTRUCTIONS
        payload = {
            "model": MODEL,
            "modalities": ["text", "audio"],
            "audio": {"voice": voice, "format": "wav"},
            "messages": [
                {"role": "system", "content": instructions},
                {
                    "role": "user",
                    "content": (
                        "Read the following script aloud VERBATIM. Do not respond to it, "
                        "answer questions, paraphrase, or add remarks. Output only supplied words:\n"
                        f"<script>\n{script}\n</script>"
                    ),
                },
            ],
        }
        try:
            raw = fetch(payload)
        except ApiResponseError as error:
            (pending / "response.json").write_bytes(error.body)
            raise
        (pending / "response.json").write_bytes(raw)
        result, transcript, data, duration = validate_response(raw, script)
        (pending / "audio.wav").write_bytes(data)
        (pending / "transcript.txt").write_text(transcript, encoding="utf-8")
        (pending / "metadata.json").write_text(
            json.dumps(
                {
                    "model": result.get("model"),
                    "voice": voice,
                    "finish_reason": "stop",
                    "usage": result.get("usage"),
                    "instructions": instructions if recovery_from is None else None,
                    "user_recording_task": payload["messages"][-1]["content"]
                    if recovery_from is None
                    else None,
                    "recovered_from_response": str(recovery_from.resolve())
                    if recovery_from is not None
                    else None,
                    "original_prompt_not_recorded": recovery_from is not None,
                    "approved": False,
                    "validation": "passed",
                    "transcript_comparison": "Exact words, punctuation and case; whitespace ignored",
                    "script_sha256": hashlib.sha256(script.encode()).hexdigest(),
                    "audio_sha256": hashlib.sha256(data).hexdigest(),
                    "duration_seconds": duration,
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        ready = take / "ready"
        pending.rename(ready)
        return ready
    except Exception as error:
        try:
            (take / "failure.json").write_text(
                json.dumps(
                    {
                        "approved": False,
                        "validation": "failed",
                        "error_type": type(error).__name__,
                        "http_status": error.status
                        if isinstance(error, ApiResponseError)
                        else None,
                        "automatic_retry": False,
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )
        except OSError:
            pass
        raise VoiceGenerationError(
            f"Take is unapproved; retained at {take}. No retry was made."
        ) from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument(
        "take_directory", type=Path, help="New, unique directory; never reused"
    )
    parser.add_argument("--voice", choices=VOICES, default="cedar")
    parser.add_argument(
        "--key-file", type=Path, help="Optional explicit env file; never executed"
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--execute", action="store_true", help="Make exactly one paid API request"
    )
    mode.add_argument(
        "--recover-from",
        type=Path,
        help="Recover a saved failed take offline; never calls API",
    )
    args = parser.parse_args()
    try:
        if args.recover_from:
            ready = recover_take(
                args.source, args.recover_from, args.take_directory, voice=args.voice
            )
        elif not args.execute:
            preflight(args.source, args.take_directory, args.voice)
            print(
                f"Preflight passed; no credentials or API accessed. Voice: {args.voice}."
            )
            return
        else:
            ready = generate_take(
                args.source,
                args.take_directory,
                voice=args.voice,
                key_loader=lambda: load_key(args.key_file),
            )
    except (OSError, ValueError, VoiceGenerationError) as error:
        parser.exit(1, f"{error}\n")
    print(f"Validated take saved for human listening approval: {ready}")


if __name__ == "__main__":
    main()
