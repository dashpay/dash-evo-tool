#!/usr/bin/env python3
"""Authorize release commits against reviewed branch history, never tag names alone."""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

TRUSTED_BRANCHES = ("v1.0-dev", "master")
SHA = re.compile(r"[0-9a-f]{40}")
TAG = re.compile(r"v[0-9][A-Za-z0-9.+-]{0,127}")


class ReleaseSourceError(ValueError):
    """The requested release is outside reviewed history or changed its identity."""


def git(repo, *args):
    return subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=False
    )


def resolve_source(repo, event_ref, event_sha, requested_sha="", tag=""):
    if not SHA.fullmatch(event_sha) or (requested_sha and requested_sha != event_sha):
        raise ReleaseSourceError(
            "The source must be the workflow event's full commit SHA."
        )
    if git(repo, "cat-file", "-e", f"{event_sha}^{{commit}}").returncode:
        raise ReleaseSourceError(
            "The source commit is unavailable; fetch its history and retry."
        )
    branch = next((b for b in TRUSTED_BRANCHES if event_ref == f"refs/heads/{b}"), None)
    event_tag = (
        event_ref.removeprefix("refs/tags/")
        if event_ref.startswith("refs/tags/")
        else ""
    )
    if branch is None and not event_tag:
        raise ReleaseSourceError(
            "Signed releases must run from a release branch or a reviewed release tag."
        )
    if event_tag and tag and tag != event_tag:
        raise ReleaseSourceError(
            "The publication tag must match the workflow's release tag."
        )
    tag = tag or event_tag
    if tag and not TAG.fullmatch(tag):
        raise ReleaseSourceError(
            "Use a release tag beginning with v and a version number."
        )
    histories = (branch,) if branch else TRUSTED_BRANCHES
    if not any(
        git(
            repo, "merge-base", "--is-ancestor", event_sha, f"refs/remotes/origin/{b}"
        ).returncode
        == 0
        for b in histories
    ):
        raise ReleaseSourceError(
            "The source commit is not in reviewed release-branch history."
        )
    if tag:
        exists = git(repo, "show-ref", "--verify", "--quiet", f"refs/tags/{tag}")
        if exists.returncode not in (0, 1):
            raise ReleaseSourceError("The release tag could not be checked.")
        if exists.returncode == 0:
            resolved = git(repo, "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
            if resolved.returncode or resolved.stdout.strip() != event_sha:
                raise ReleaseSourceError(
                    "The release tag points to a different source commit."
                )
        elif event_tag:
            raise ReleaseSourceError("The triggering release tag no longer exists.")
    return {"source_sha": event_sha, "tag": tag}


def github_object(endpoint, allow_missing=False):
    reply = subprocess.run(
        ["gh", "api", "--method", "GET", endpoint],
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
    )
    try:
        data = json.loads(reply.stdout)
    except json.JSONDecodeError as error:
        raise ReleaseSourceError(
            "GitHub did not return usable release metadata."
        ) from error
    if not isinstance(data, dict):
        raise ReleaseSourceError("GitHub returned invalid release metadata.")
    if reply.returncode:
        if allow_missing and str(data.get("status")) == "404":
            return None
        raise ReleaseSourceError("The release metadata could not be verified.")
    return data


def verify_remote_tag(repository, tag, expected_sha, allow_missing=False):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ReleaseSourceError("Invalid release repository.")
    if not TAG.fullmatch(tag) or not SHA.fullmatch(expected_sha):
        raise ReleaseSourceError("Invalid publication tag or source SHA.")
    endpoint = f"repos/{repository}/git/ref/tags/{tag}"
    seen = set()
    for depth in range(16):
        data = github_object(endpoint, allow_missing=depth == 0 and allow_missing)
        if data is None:
            return
        obj = data.get("object", {})
        if not isinstance(obj, dict):
            raise ReleaseSourceError(
                "The publication tag returned invalid object metadata."
            )
        sha = obj.get("sha", "")
        if not isinstance(sha, str) or not SHA.fullmatch(sha):
            raise ReleaseSourceError(
                "The publication tag returned invalid commit metadata."
            )
        if obj.get("type") == "commit":
            if sha != expected_sha:
                raise ReleaseSourceError(
                    "The publication tag moved to a different source commit."
                )
            return
        if obj.get("type") != "tag" or sha in seen:
            raise ReleaseSourceError(
                "The publication tag cannot be resolved to a commit."
            )
        seen.add(sha)
        endpoint = f"repos/{repository}/git/tags/{sha}"
    raise ReleaseSourceError("Too many nested annotated release tags.")


def cleanup_plan(repository, tag, expected_sha):
    verify_remote_tag(repository, tag, expected_sha)
    release = github_object(
        f"repos/{repository}/releases/tags/{tag}", allow_missing=True
    )
    if release is None:
        return {"delete_release": "false"}
    if release.get("draft") is not True:
        raise ReleaseSourceError("Refusing to delete an already-published release.")
    body = release.get("body") or ""
    if not isinstance(body, str) or "<!-- weekly-version:" not in body:
        raise ReleaseSourceError("Refusing to delete an unrecognized release draft.")
    return {"delete_release": "true"}


def emit_outputs(result):
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as stream:
            for key, value in result.items():
                stream.write(f"{key}={value}\n")
    print(json.dumps(result))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--verify-remote-tag", action="store_true")
    parser.add_argument("--allow-new-tag", action="store_true")
    parser.add_argument("--cleanup-plan", action="store_true")
    args = parser.parse_args()
    try:
        if args.cleanup_plan:
            emit_outputs(
                cleanup_plan(
                    os.environ.get("GITHUB_REPOSITORY", ""),
                    os.environ.get("RELEASE_TAG", ""),
                    os.environ.get("RELEASE_SOURCE_SHA", ""),
                )
            )
            return 0
        if args.verify_remote_tag:
            verify_remote_tag(
                os.environ.get("GITHUB_REPOSITORY", ""),
                os.environ.get("RELEASE_TAG", ""),
                os.environ.get("RELEASE_SOURCE_SHA", ""),
                args.allow_new_tag
                and os.environ.get("GITHUB_REF")
                in {f"refs/heads/{branch}" for branch in TRUSTED_BRANCHES},
            )
            print("The publication tag matches the authorized source.")
            return 0
        result = resolve_source(
            args.repo,
            os.environ.get("GITHUB_REF", ""),
            os.environ.get("GITHUB_SHA", ""),
            os.environ.get("RELEASE_SOURCE_SHA", ""),
            os.environ.get("RELEASE_TAG", ""),
        )
        policy = git(args.repo, "rev-parse", "HEAD")
        if policy.returncode or not SHA.fullmatch(policy.stdout.strip()):
            raise ReleaseSourceError(
                "The trusted release-policy revision is unavailable."
            )
        result["policy_sha"] = policy.stdout.strip()
        emit_outputs(result)
        return 0
    except (ReleaseSourceError, OSError, subprocess.TimeoutExpired) as error:
        print(f"Release authorization failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
