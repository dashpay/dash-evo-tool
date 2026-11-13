"""Batch contracts using only temporary files, fake keys and fabricated responses."""

import base64
import hashlib
import io
import json
from pathlib import Path
import threading
import time
import wave

import pytest

import batch_narration as batch


@pytest.fixture
def script(tmp_path):
    source = tmp_path / "dialogue.json"
    source.write_text(
        json.dumps(
            {
                "parts": [
                    {
                        "id": 1,
                        "intro": [{"speaker": "teacher", "text": "Welcome."}],
                        "outro": [{"speaker": "learner", "text": "Thank you."}],
                    }
                ],
                "scenes": [
                    {
                        "id": "00",
                        "part": 1,
                        "turns": [
                            {
                                "speaker": "teacher" if i % 2 == 0 else "learner",
                                "text": f"Sentence {i}.",
                            }
                            for i in range(8)
                        ],
                    }
                ],
            }
        )
    )
    return source


@pytest.fixture
def api(monkeypatch):
    calls = []

    def request(payload, key):
        assert key == "fabricated-secret-never-persist"
        calls.append(payload)
        content = payload["messages"][-1]["content"]
        text = (
            content.split("<script>\n", 1)[1].rsplit("\n</script>", 1)[0]
            if "<script>\n" in content
            else content.split("\n\n", 1)[1]
        )
        buf = io.BytesIO()
        with wave.open(buf, "wb") as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(24000)
            audio.writeframes(b"\0\0" * 24)
        return json.dumps(
            {
                "id": "response-" + hashlib.sha256(text.encode()).hexdigest(),
                "model": "gpt-audio-1.5",
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 30,
                    "total_tokens": 130,
                    "completion_tokens_details": {"audio_tokens": 20},
                },
                "choices": [
                    {
                        "finish_reason": "stop",
                        "message": {
                            "audio": {
                                "data": base64.b64encode(buf.getvalue()).decode(),
                                "transcript": text,
                            }
                        },
                    }
                ],
            }
        ).encode()

    monkeypatch.setattr(
        batch.voice, "load_key", lambda: "fabricated-secret-never-persist"
    )
    monkeypatch.setattr(batch.voice, "request_audio", request)
    return calls, request


def execute(script, root, **kwargs):
    return batch.run_batch(
        script,
        root,
        execute=True,
        approved_script_sha256=hashlib.sha256(script.read_bytes()).hexdigest(),
        **kwargs,
    )


def test_free_plan_expands_sources_and_never_calls_key_or_api(
    script, tmp_path, monkeypatch
):
    monkeypatch.setattr(
        batch.voice, "load_key", lambda: pytest.fail("credential access")
    )
    monkeypatch.setattr(
        batch.voice, "request_audio", lambda *_: pytest.fail("HTTP access")
    )
    manifest = batch.run_batch(script, tmp_path / "audio")
    assert len(manifest["jobs"]) == 10
    assert manifest["jobs"][0]["id"] == "scene-00-turn-00"
    assert manifest["jobs"][-2]["id"] == "part-01-intro-00"
    assert {job["voice"] for job in manifest["jobs"]} == {"cedar", "nova"}
    assert all(
        Path(j["source_path"]).read_text() == j["text"] + "\n" for j in manifest["jobs"]
    )


@pytest.mark.parametrize("approved", [None, "0" * 64])
def test_execute_needs_exact_audited_hash(script, tmp_path, api, approved):
    with pytest.raises(ValueError):
        batch.run_batch(
            script, tmp_path / "audio", execute=True, approved_script_sha256=approved
        )
    assert api[0] == []


def test_selected_pilot_and_resume_do_not_repeat_green_jobs(script, tmp_path, api):
    root = tmp_path / "audio"
    first = execute(script, root, ids=["scene-00-turn-00"])
    assert len(api[0]) == 1
    assert first["summary"]["ready"] == 1
    second = execute(script, root, ids=["scene-00-turn-00"])
    assert len(api[0]) == 1
    assert second["jobs"][0]["audio_path"] == first["jobs"][0]["audio_path"]
    execute(script, root)
    assert len(api[0]) == 10
    assert "fabricated-secret-never-persist" not in (root / "manifest.json").read_text()
    assert all(
        "Authorization" not in p.read_text() for p in (root / "requests").glob("*.json")
    )


def test_failure_never_retries_without_explicit_failed_id(
    script, tmp_path, api, monkeypatch
):
    calls, good = api

    def fail(payload, key):
        calls.append(payload)
        raise RuntimeError("fabricated-secret-never-persist")

    monkeypatch.setattr(batch.voice, "request_audio", fail)
    root = tmp_path / "audio"
    first = execute(script, root, ids=["scene-00-turn-00"])
    old_take = first["jobs"][0]["attempts"][0]["take_directory"]
    execute(script, root, ids=["scene-00-turn-00"])
    assert len(calls) == 1
    assert first["summary"]["failed_ids"] == ["scene-00-turn-00"]
    monkeypatch.setattr(batch.voice, "request_audio", good)
    final = execute(script, root, retry_failed=["scene-00-turn-00"])
    assert len(calls) == 2
    assert final["jobs"][0]["attempts"][-1]["take_directory"] != old_take
    assert final["jobs"][0]["status"] == "ready"
    assert "fabricated-secret-never-persist" not in (root / "manifest.json").read_text()


def test_changed_source_refuses_resume_before_api(script, tmp_path, api):
    root = tmp_path / "audio"
    batch.run_batch(script, root)
    script.write_text(script.read_text() + "\n")
    with pytest.raises(ValueError):
        execute(script, root)
    assert api[0] == []


def test_corrupt_green_is_failed_not_automatically_regenerated(script, tmp_path, api):
    root = tmp_path / "audio"
    manifest = execute(script, root, ids=["scene-00-turn-00"])
    Path(manifest["jobs"][0]["audio_path"]).write_bytes(b"bad")
    result = execute(script, root, ids=["scene-00-turn-00"])
    assert len(api[0]) == 1
    assert result["jobs"][0]["status"] == "failed"


def test_interrupted_attempt_is_reconciled_without_paid_retry(script, tmp_path, api):
    root = tmp_path / "audio"
    manifest = execute(script, root, ids=["scene-00-turn-00"])
    manifest["jobs"][0]["status"] = "running"
    manifest["jobs"][0]["attempts"][-1]["status"] = "running"
    (root / "manifest.json").write_text(json.dumps(manifest))
    resumed = execute(script, root, ids=["scene-00-turn-00"])
    assert resumed["jobs"][0]["status"] == "ready"
    assert len(api[0]) == 1


def test_concurrency_is_bounded_and_second_coordinator_locked(
    script, tmp_path, api, monkeypatch
):
    lock = threading.Lock()
    state = {"active": 0, "peak": 0}
    root = tmp_path / "audio"
    calls, good = api

    def slow(payload, key):
        with pytest.raises(RuntimeError, match="already"):
            batch.run_batch(script, root)
        with lock:
            state["active"] += 1
            state["peak"] = max(state["peak"], state["active"])
        time.sleep(0.03)
        try:
            return good(payload, key)
        finally:
            with lock:
                state["active"] -= 1

    monkeypatch.setattr(batch.voice, "request_audio", slow)
    result = execute(script, root, workers=5)
    assert result["summary"]["ready"] == 10
    assert 1 < state["peak"] <= 5


def test_usage_deduplicates_response_ids_and_separates_baseline():
    ledger = batch.usage_summary(
        [
            {
                "id": "a",
                "response_id": "same",
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 30,
                    "completion_tokens_details": {"audio_tokens": 20},
                },
            },
            {
                "id": "b",
                "response_id": "same",
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 30,
                    "completion_tokens_details": {"audio_tokens": 20},
                },
            },
        ]
    )
    assert ledger["unique_responses"] == 1
    assert ledger["output_audio_tokens"] == 20
    assert ledger["estimated_batch_usd"] == pytest.approx(0.00163)
    assert ledger["baseline_usd"] == 0.0


def test_unknown_ids_and_worker_overflow_refuse_before_api(script, tmp_path, api):
    with pytest.raises(ValueError):
        execute(script, tmp_path / "a", ids=["missing"])
    with pytest.raises(ValueError):
        execute(script, tmp_path / "b", workers=6)
    assert api[0] == []


def test_part_selection_includes_its_extras_only(script, tmp_path, api):
    doc = json.loads(script.read_text())
    doc["parts"].append(
        {
            "id": 2,
            "intro": [{"speaker": "teacher", "text": "Next."}],
            "outro": [{"speaker": "learner", "text": "Later."}],
        }
    )
    doc["scenes"].append(
        {
            "id": "01",
            "part": 2,
            "turns": [{"speaker": "teacher", "text": "Another scene."}],
        }
    )
    script.write_text(json.dumps(doc))
    result = execute(script, tmp_path / "audio", part=1)
    assert len(api[0]) == 10
    assert all(
        j["status"] == ("ready" if j["part"] == 1 else "planned")
        for j in result["jobs"]
    )


def test_empty_selection_never_expands_to_all_jobs(script, tmp_path, api):
    execute(script, tmp_path / "audio", ids=[])
    assert api[0] == []


@pytest.mark.parametrize(
    "index,speaker,chosen_voice", [(0, "teacher", "cedar"), (1, "learner", "nova")]
)
def test_plain_prompt_preserves_script_voice_system_and_actual_journal(
    script, tmp_path, api, index, speaker, chosen_voice
):
    result = execute(
        script,
        tmp_path / "audio",
        ids=[f"scene-00-turn-{index:02d}"],
        plain_prompt=True,
    )
    job = result["jobs"][index]
    assert job["status"] == "ready"
    assert job["speaker"] == speaker
    payload = api[0][0]
    expected_system = (
        batch.voice.INSTRUCTIONS
        if speaker == "teacher"
        else batch.voice.LEARNER_INSTRUCTIONS
    )
    assert payload["messages"][0] == {"role": "system", "content": expected_system}
    assert payload["audio"]["voice"] == chosen_voice
    content = payload["messages"][-1]["content"]
    assert "<script>" not in content and "</script>" not in content
    assert content.split("\n\n", 1)[1] == Path(job["source_path"]).read_text()
    attempt = job["attempts"][-1]
    assert attempt["prompt_mode"] == "plain"
    assert json.loads(Path(attempt["request_path"]).read_text())["payload"] == payload
    assert (
        json.loads(Path(job["metadata_path"]).read_text())["user_recording_task"]
        == content
    )


def test_plain_flag_does_not_retry_failed_until_explicit_selection(
    script, tmp_path, api, monkeypatch
):
    calls, good = api

    def read_tags(payload, key):
        raw = json.loads(good(payload, key))
        audio = raw["choices"][0]["message"]["audio"]
        audio["transcript"] = "<script>\n" + audio["transcript"] + "\n</script>"
        return json.dumps(raw).encode()

    monkeypatch.setattr(batch.voice, "request_audio", read_tags)
    root = tmp_path / "audio"
    first = execute(script, root, ids=["scene-00-turn-00"])
    old_attempt = first["jobs"][0]["attempts"][0].copy()
    old_request = Path(old_attempt["request_path"]).read_bytes()
    old_response = Path(old_attempt["response_path"]).read_bytes()
    assert first["jobs"][0]["status"] == "failed"
    execute(script, root, ids=["scene-00-turn-00"], plain_prompt=True)
    assert len(calls) == 1
    monkeypatch.setattr(batch.voice, "request_audio", good)
    result = execute(script, root, retry_failed=["scene-00-turn-00"], plain_prompt=True)
    assert len(calls) == 2
    assert result["jobs"][0]["status"] == "ready"
    assert result["jobs"][0]["attempts"][0] == old_attempt
    assert Path(old_attempt["request_path"]).read_bytes() == old_request
    assert Path(old_attempt["response_path"]).read_bytes() == old_response
    assert result["jobs"][0]["attempts"][-1]["id"] != old_attempt["id"]
