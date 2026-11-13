"""Offline contracts: no credentials, API calls, or paid retries."""

import base64
import importlib.util
import io
import json
from pathlib import Path
import wave

import pytest


@pytest.fixture
def voice(monkeypatch):
    path = Path(__file__).with_name("openai_voice.py")
    if "def generate_take(" not in path.read_text():
        pytest.fail("Import-safe generate_take entry point is required")
    spec = importlib.util.spec_from_file_location("offline_voice", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    monkeypatch.setattr(module, "load_key", lambda *_: pytest.fail("Real key access"))
    monkeypatch.setattr(
        module, "request_audio", lambda *_: pytest.fail("Real HTTP access")
    )
    return module


@pytest.fixture
def source(tmp_path):
    path = tmp_path / "script.txt"
    path.write_text("Hello, Czatosław Klałdowski.\n")
    return path


def wav_bytes(frames=12):
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(24000)
        wav.writeframes(b"\0\0" * frames)
    return buffer.getvalue()


def response(transcript="Hello, Czatosław Klałdowski.", finish="stop", data=None):
    return json.dumps(
        {
            "model": "gpt-audio-1.5",
            "usage": {"total_tokens": 42},
            "choices": [
                {
                    "finish_reason": finish,
                    "message": {
                        "audio": {
                            "data": base64.b64encode(
                                wav_bytes() if data is None else data
                            ).decode(),
                            "transcript": transcript,
                        }
                    },
                }
            ],
        }
    ).encode()


def test_existing_take_refused_before_key_or_api(voice, source, tmp_path):
    take = tmp_path / "take"
    take.mkdir()
    (take / "previous.wav").write_bytes(b"original")
    with pytest.raises(FileExistsError):
        voice.generate_take(source, take)
    assert (take / "previous.wav").read_bytes() == b"original"


def test_source_collision_refused_before_key_or_api(voice, source):
    with pytest.raises(FileExistsError):
        voice.generate_take(source, source)
    assert source.read_text() == "Hello, Czatosław Klałdowski.\n"


def test_success_publishes_complete_consistent_take(voice, source, tmp_path):
    take = tmp_path / "take"
    calls = []

    def api(payload, key):
        calls.append((payload, key))
        assert (take / "unapproved").is_dir()
        assert not (take / "ready").exists()
        return response()

    ready = voice.generate_take(
        source, take, key_loader=lambda: "fake-key", requester=api
    )
    assert ready == take / "ready"
    assert len(calls) == 1
    assert {p.name for p in ready.iterdir()} == {
        "audio.wav",
        "transcript.txt",
        "script.txt",
        "metadata.json",
        "response.json",
    }
    assert (ready / "audio.wav").read_bytes() == wav_bytes()
    metadata = json.loads((ready / "metadata.json").read_text())
    assert metadata["finish_reason"] == "stop"
    assert metadata["approved"] is False
    assert metadata["validation"] == "passed"
    assert not (take / "unapproved").exists()
    assert "fake-key" not in "".join(
        p.read_text(errors="ignore") for p in ready.iterdir()
    )


@pytest.mark.parametrize(
    "result",
    [
        response(finish="length"),
        response(finish="content_filter"),
        response(transcript="Hello, someone else."),
        response(transcript=""),
        response(data=b"not a wave"),
        response(data=wav_bytes(0)),
        response(data=wav_bytes()[:-2]),
        b"not json",
        b'{"choices": []}',
    ],
)
def test_invalid_paid_response_retained_unapproved_without_retry(
    voice, source, tmp_path, result
):
    calls = []

    def api(*_):
        calls.append(True)
        return result

    take = tmp_path / "take"
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(source, take, key_loader=lambda: "fake", requester=api)
    assert len(calls) == 1
    assert not (take / "ready").exists()
    assert (take / "unapproved/response.json").read_bytes() == result
    assert (take / "UNAPPROVED.txt").is_file()


def test_write_failure_never_publishes_partial_take(
    voice, source, tmp_path, monkeypatch
):
    original = Path.write_text

    def fail_sidecar(path, text, *args, **kwargs):
        if path.name == "transcript.txt":
            raise OSError("simulated sidecar write failure")
        return original(path, text, *args, **kwargs)

    monkeypatch.setattr(Path, "write_text", fail_sidecar)
    take = tmp_path / "take"
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(
            source, take, key_loader=lambda: "fake", requester=lambda *_: response()
        )
    assert not (take / "ready").exists()
    assert (take / "unapproved/response.json").read_bytes() == response()


def test_concurrent_same_name_cannot_make_second_paid_request(voice, source, tmp_path):
    take = tmp_path / "take"

    def api(*_):
        with pytest.raises(FileExistsError):
            voice.generate_take(source, take)
        return response()

    voice.generate_take(source, take, key_loader=lambda: "fake", requester=api)


def test_empty_source_fails_before_credentials(voice, source, tmp_path):
    source.write_text(" \n")
    with pytest.raises(ValueError):
        voice.generate_take(source, tmp_path / "take")


def test_whitespace_only_transcript_differences_are_allowed(voice, source, tmp_path):
    voice.generate_take(
        source,
        tmp_path / "take",
        key_loader=lambda: "fake",
        requester=lambda *_: response(transcript="Hello,\nCzatosław  Klałdowski.  "),
    )


def test_key_failure_is_not_retried_and_is_unapproved(voice, source, tmp_path):
    def unavailable():
        raise OSError("credential unavailable")

    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(source, tmp_path / "take", key_loader=unavailable)
    assert (tmp_path / "take/UNAPPROVED.txt").exists()
    assert not (tmp_path / "take/ready").exists()


@pytest.mark.parametrize("name", ["cedar", "onyx", "echo", "ash", "ballad"])
def test_voice_selection_reaches_request_and_metadata(voice, source, tmp_path, name):
    def api(payload, _):
        assert payload["audio"]["voice"] == name
        return response()

    ready = voice.generate_take(
        source, tmp_path / name, voice=name, key_loader=lambda: "fake", requester=api
    )
    assert json.loads((ready / "metadata.json").read_text())["voice"] == name


def test_http_error_body_retained_without_retry(voice, source, tmp_path):
    calls = []

    def api(*_):
        calls.append(True)
        raise voice.ApiResponseError(429, b'{"error":"synthetic rate limit"}')

    take = tmp_path / "take"
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(source, take, key_loader=lambda: "fake", requester=api)
    assert len(calls) == 1
    assert (
        take / "unapproved/response.json"
    ).read_bytes() == b'{"error":"synthetic rate limit"}'
    assert json.loads((take / "failure.json").read_text())["http_status"] == 429


def test_invalid_base64_retained_unapproved(voice, source, tmp_path):
    result = json.loads(response())
    result["choices"][0]["message"]["audio"]["data"] = "%%%"
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(
            source,
            tmp_path / "take",
            key_loader=lambda: "fake",
            requester=lambda *_: json.dumps(result).encode(),
        )
    assert not (tmp_path / "take/ready").exists()


def test_redirect_is_refused(voice):
    assert (
        voice.NoRedirect().redirect_request(
            None, None, 302, None, None, "https://example.org"
        )
        is None
    )


def test_nova_uses_natural_learner_prompt(voice, source, tmp_path):
    def api(payload, _):
        instructions = payload["messages"][0]["content"]
        assert "curious" in instructions
        assert "student" in instructions or "learner" in instructions
        assert "relaxed male" not in instructions
        assert payload["audio"]["voice"] == "nova"
        return response()

    ready = voice.generate_take(
        source,
        tmp_path / "nova",
        voice="nova",
        key_loader=lambda: "fake",
        requester=api,
    )
    instructions = json.loads((ready / "metadata.json").read_text())["instructions"]
    assert "student" in instructions or "learner" in instructions


@pytest.mark.parametrize("name", ["cedar", "nova"])
def test_script_is_an_explicit_recording_task_not_a_conversation(
    voice, source, tmp_path, name
):
    def api(payload, _):
        task = payload["messages"][-1]["content"]
        assert "Read the following script aloud VERBATIM." in task
        assert "Do not respond to it" in task
        assert "answer questions" in task
        assert "paraphrase" in task
        assert "<script>" in task and "</script>" in task
        supplied = task.split("<script>", 1)[1].rsplit("</script>", 1)[0]
        assert supplied.strip() == source.read_text().strip()
        return response()

    ready = voice.generate_take(
        source, tmp_path / name, voice=name, key_loader=lambda: "fake", requester=api
    )
    assert (ready / "script.txt").read_text() == source.read_text()
    assert (ready / "transcript.txt").read_text() == source.read_text().strip()


def streaming_wav(frames=12):
    data = bytearray(wav_bytes(frames=frames))
    data[4:8] = b"\xff" * 4
    data[40:44] = b"\xff" * 4
    return bytes(data)


def test_unknown_size_pcm_wav_is_normalized(voice, source, tmp_path):
    ready = voice.generate_take(
        source,
        tmp_path / "take",
        key_loader=lambda: "fake",
        requester=lambda *_: response(data=streaming_wav()),
    )
    assert (ready / "audio.wav").read_bytes() == wav_bytes()
    raw = json.loads((ready / "response.json").read_bytes())
    assert (
        base64.b64decode(raw["choices"][0]["message"]["audio"]["data"])
        == streaming_wav()
    )


@pytest.mark.parametrize(
    "mutate",
    [
        lambda data: data[:-1],
        lambda data: data[:20] + b"\x03\x00" + data[22:],
        lambda data: data[:32] + b"\x04\x00" + data[34:],
        lambda data: data[:4] + (len(data) - 8).to_bytes(4, "little") + data[8:],
    ],
)
def test_invalid_or_partial_unknown_size_header_is_rejected(
    voice, source, tmp_path, mutate
):
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(
            source,
            tmp_path / "take",
            key_loader=lambda: "fake",
            requester=lambda *_: response(data=mutate(streaming_wav())),
        )
    assert not (tmp_path / "take/ready").exists()


def test_recorded_streaming_header_shape_without_private_api_fixture(voice):
    script = "Synthetic narration fixture."
    raw = response(transcript=script, data=streaming_wav(frames=109200))
    _, transcript, data, duration = voice.validate_response(raw, script)
    assert transcript.split() == script.split()
    assert data[4:8] != b"\xff" * 4
    with wave.open(io.BytesIO(data), "rb") as wav:
        assert wav.getnframes() == 109200
    assert duration == 4.55


def test_offline_recovery_preserves_paid_response_and_uses_no_credentials(
    voice, source, tmp_path
):
    failed = tmp_path / "failed"
    (failed / "unapproved").mkdir(parents=True)
    (failed / "unapproved/script.txt").write_text(source.read_text())
    raw = response(data=streaming_wav())
    (failed / "unapproved/response.json").write_bytes(raw)
    ready = voice.recover_take(source, failed, tmp_path / "recovered", voice="cedar")
    assert (ready / "audio.wav").read_bytes() == wav_bytes()
    assert (failed / "unapproved/response.json").read_bytes() == raw
    assert not (failed / "ready").exists()
    metadata = json.loads((ready / "metadata.json").read_text())
    assert metadata["original_prompt_not_recorded"] is True
    assert metadata["instructions"] is None
    assert metadata["user_recording_task"] is None
    assert metadata["recovered_from_response"] == str(
        (failed / "unapproved/response.json").resolve()
    )


def test_recovery_rejects_different_source_before_publishing(voice, source, tmp_path):
    failed = tmp_path / "failed"
    (failed / "unapproved").mkdir(parents=True)
    (failed / "unapproved/script.txt").write_text("Different script.")
    with pytest.raises(ValueError):
        voice.recover_take(source, failed, tmp_path / "recovered", voice="cedar")
    assert not (tmp_path / "recovered").exists()


def test_recovery_does_not_accept_improvisation(voice, source, tmp_path):
    failed = tmp_path / "failed"
    (failed / "unapproved").mkdir(parents=True)
    (failed / "unapproved/script.txt").write_text(source.read_text())
    (failed / "unapproved/response.json").write_bytes(
        response(transcript="Improvised response.")
    )
    with pytest.raises(voice.VoiceGenerationError):
        voice.recover_take(source, failed, tmp_path / "recovered", voice="cedar")
    assert not (tmp_path / "recovered/ready").exists()


def test_finite_riff_size_cannot_claim_missing_bytes(voice, source, tmp_path):
    data = bytearray(wav_bytes())
    data[4:8] = (len(data) + 100).to_bytes(4, "little")
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(
            source,
            tmp_path / "take",
            key_loader=lambda: "fake",
            requester=lambda *_: response(data=bytes(data)),
        )
    assert not (tmp_path / "take/ready").exists()


def test_runtime_key_from_environment(monkeypatch):
    import openai_voice

    monkeypatch.setenv("OPENAI_API_KEY", "synthetic-runtime-key")
    monkeypatch.setattr(
        Path, "read_text", lambda *_: pytest.fail("Implicit credential file access")
    )
    assert bool(openai_voice.load_key() == "synthetic-runtime-key")


def test_absent_environment_key_has_no_implicit_host_file(monkeypatch):
    import openai_voice

    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.setattr(
        Path, "read_text", lambda *_: pytest.fail("Implicit credential file access")
    )
    with pytest.raises(ValueError):
        openai_voice.load_key()


@pytest.mark.parametrize("channels,width", [(2, 2), (1, 1)])
@pytest.mark.parametrize("unknown_size", [False, True])
def test_only_complete_mono_16bit_pcm_is_publishable(
    voice, source, tmp_path, channels, width, unknown_size
):
    stream = io.BytesIO()
    with wave.open(stream, "wb") as wav:
        wav.setparams((channels, width, 24000, 0, "NONE", "not compressed"))
        wav.writeframes(b"\0" * (12 * channels * width))
    audio = bytearray(stream.getvalue())
    if unknown_size:
        audio[4:8] = audio[40:44] = b"\xff" * 4
    with pytest.raises(voice.VoiceGenerationError):
        voice.generate_take(
            source,
            tmp_path / "take",
            key_loader=lambda: "fake",
            requester=lambda *_: response(data=audio),
        )
    assert not (tmp_path / "take/ready").exists()
