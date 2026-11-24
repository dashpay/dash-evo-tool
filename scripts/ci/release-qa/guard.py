#!/usr/bin/python3
"""Claude PreToolUse boundary: files in public inputs/output; one GUI helper."""

import json
import os
import shlex
import sys
from pathlib import Path

from insight import query_path


def allowed(payload, root, checkout):
    tool, args = payload.get("tool_name"), payload.get("tool_input", {})
    if tool == "Bash":
        command = args.get("command", "")
        words = shlex.split(command)
        if len(words) > 1 and words[1] == "insight":
            if len(words) != 4:
                return False
            try:
                query_path(words[2], words[3])
            except (ValueError, TypeError):
                return False
        return (
            not args.get("run_in_background")
            and bool(words)
            and words[0] == "/usr/local/bin/qa-ui"
            and not any(c in command for c in "\n\r;|&><`$()")
        )
    if tool not in ("Read", "Write", "Edit"):
        return False
    path = Path(args.get("file_path", "")).resolve()
    if tool == "Read":
        return any(
            path.is_relative_to(p)
            for p in (root / "report", root / "context", checkout)
        ) and not any(
            p.startswith(".env") or p in (".git", ".claude.json") for p in path.parts
        )
    return (
        path.is_relative_to(root / "report")
        and path.name not in ("result.json", "wallet-status.json", "00-smoke.png")
        and not path.name.startswith("app-")
        and path.suffix in (".md", ".json")
    )


if __name__ == "__main__":
    try:
        ok = allowed(
            json.load(sys.stdin),
            Path(os.environ["QA_ROOT"]).resolve(),
            Path(os.environ["QA_SOURCE"]).resolve(),
        )
    except Exception:
        ok = False
    print(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow" if ok else "deny",
                    "permissionDecisionReason": "Release QA allows public files and the fixed GUI driver only.",
                }
            }
        )
    )
