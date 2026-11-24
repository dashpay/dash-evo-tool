"""Resolve release provenance and immutable GUI-review context before Claude runs."""

import json
import os
import re
import subprocess
from pathlib import Path


def run(*args):
    return subprocess.check_output(args, text=True, timeout=180).strip()


def tag(value):
    if not re.fullmatch(
        r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", value
    ):
        raise ValueError("Invalid release tag")
    return value


def select_baseline(candidates, explicit, new_sha):
    for release in candidates:
        name = release["tag_name"]
        if explicit and name != explicit:
            continue
        try:
            name = tag(name)
        except ValueError:
            if explicit:
                raise
            continue
        run("git", "fetch", "origin", f"+refs/tags/{name}:refs/tags/{name}")
        sha = run("git", "rev-parse", f"{name}^{{commit}}")
        try:
            run("git", "merge-base", "--is-ancestor", sha, new_sha)
        except subprocess.CalledProcessError as error:
            if explicit or error.returncode != 1:
                raise
            continue
        return name, sha
    raise ValueError("No older published pre-release ancestor of NEW")


def main():
    root = Path(os.environ["QA_ROOT"])
    root.mkdir(parents=True, exist_ok=True)
    repo, new = os.environ["GITHUB_REPOSITORY"], tag(os.environ["NEW_TAG"])
    releases = json.loads(
        run("gh", "api", f"repos/{repo}/releases", "--paginate", "--slurp")
    )
    releases = [r for page in releases for r in page]
    new_release = next(r for r in releases if r["tag_name"] == new)
    if not new_release["prerelease"]:
        raise ValueError("Release QA requires a pre-release")
    baseline = os.environ.get("BASELINE", "")
    candidates = [
        r
        for r in releases
        if not r["draft"]
        and r["prerelease"]
        and r["tag_name"] != new
        and r["created_at"] < new_release["created_at"]
    ]
    candidates.sort(key=lambda r: r["published_at"], reverse=True)
    run("git", "fetch", "origin", f"+refs/tags/{new}:refs/tags/{new}")
    new_sha = run("git", "rev-parse", f"{new}^{{commit}}")
    baseline, old_sha = select_baseline(candidates, baseline, new_sha)
    binary = root / "dash-evo-tool-linux-x86_64.zip"
    if not binary.exists():
        run(
            "gh",
            "release",
            "download",
            new,
            "-R",
            repo,
            "-p",
            binary.name,
            "-D",
            str(root),
        )
    run(
        "gh",
        "attestation",
        "verify",
        str(binary),
        "-R",
        repo,
        "--signer-workflow",
        repo + "/.github/workflows/release.yml",
        "--source-digest",
        new_sha,
    )
    context = root / "context"
    context.mkdir(exist_ok=True)
    commits = run(
        "git", "log", "--no-merges", "--format=%H %s", f"{old_sha}..{new_sha}"
    )
    (context / "commits.txt").write_text(commits)
    (context / "changes.diff").write_text(
        run("git", "diff", "--no-ext-diff", old_sha, new_sha)
    )
    prs = []
    for sha in (line.split()[0] for line in commits.splitlines()):
        prs.extend(json.loads(run("gh", "api", f"repos/{repo}/commits/{sha}/pulls")))
    (context / "pull-requests.json").write_text(
        json.dumps(
            {
                p["number"]: {
                    k: p.get(k)
                    for k in ("number", "title", "body", "html_url", "merged_at")
                }
                for p in prs
            }
        )
    )
    (context / "inputs.json").write_text(
        json.dumps(
            {
                "new_tag": new,
                "new_sha": new_sha,
                "baseline": baseline,
                "baseline_sha": old_sha,
                "repository": repo,
            }
        )
    )


if __name__ == "__main__":
    main()
