#!/usr/bin/env python3
"""Classify the complete PR diff; uncertain history fails instead of skipping CI."""

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


def is_relevant(paths, suite):
    for path in paths:
        if (
            path.endswith(".rs")
            or path == "Cargo.toml"
            or path.endswith("/Cargo.toml")
            or path
            in {
                "Cargo.lock",
                ".cargo/config.toml",
                "rust-toolchain.toml",
                "scripts/ci/pr_scope.py",
                f".github/workflows/{suite}.yml",
            }
        ):
            return True
        if suite == "tests" and (
            path.startswith(
                (
                    "tests/backend-e2e/",
                    "tests/migration-fixtures/",
                    "src/wallet_backend/platform_compatibility/fixtures/",
                    "scripts/migration-fixtures/",
                    "scripts/ci/",
                )
            )
            or path
            in {
                ".github/workflows/weekly-build.yml",
                ".github/workflows/migration-fixture-capture.yml",
            }
        ):
            return True
    return False


def changed_paths(repo, event_path):
    event = json.loads(event_path.read_text())
    pr = event["pull_request"]
    base, head = pr["base"]["sha"], pr["head"]["sha"]
    if not all(
        isinstance(sha, str) and re.fullmatch(r"[0-9a-f]{40}", sha)
        for sha in (base, head)
    ):
        raise ValueError("The PR event must contain full base and head commit SHAs.")
    result = subprocess.run(
        [
            "git",
            "-C",
            str(repo),
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            f"{base}...{head}",
            "--",
        ],
        capture_output=True,
        check=True,
        timeout=60,
    )
    return [os.fsdecode(path) for path in result.stdout.split(b"\0") if path]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=("tests", "clippy"), required=True)
    args = parser.parse_args()
    paths = changed_paths(Path("."), Path(os.environ["GITHUB_EVENT_PATH"]))
    needed = "true" if is_relevant(paths, args.suite) else "false"
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"needed={needed}\n")
    print(f"{args.suite}: needed={needed} ({len(paths)} changed paths)")


if __name__ == "__main__":
    main()
