"""Describe, check and source the migration fixture a release carries as assets.

build   turn a chain.py receipt into the manifest entry published beside the archive
check   accept a downloaded entry only if it describes exactly this release and archive
source  choose the fixture a release's chain starts from
Requires jq for SemVer ordering, like chain.py.
"""

import argparse
import hashlib
import json
import re
import subprocess
from datetime import date, datetime, timezone
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
WORKFLOW = "migration-fixture-capture.yml"
# Everything an entry may carry; `check` rejects any other key.
ENTRY_KEYS = {
    "id",
    "git_tag",
    "det_version",
    "capture_method",
    "profile",
    "network",
    "host_platform",
    "binary_asset",
    "platform_rev",
    "captured_at",
    "provenance",
    "artifact",
    "expect",
    "contents",
    "notes",
}
ARTIFACT_KEYS = {
    "workflow_name",
    "workflow_run_id",
    "artifact_name",
    "archive_filename",
    "sha256",
    "bytes",
}


def asset_name(tag: str, extension: str) -> str:
    """Name of a release's fixture asset; `tar.zst` is the archive, `json` its entry."""
    return f"migration-fixture-{tag}-wallet-only.{extension}"


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build(
    receipt: dict,
    archive: Path,
    tag: str,
    det_version: str,
    platform_rev: str,
    run_id: int,
    today: date,
) -> dict:
    """Write the manifest entry for the archive a chain.py hop produced."""
    size = archive.stat().st_size
    actual = digest(archive)
    if (receipt["git_tag"], receipt["sha256"], receipt["bytes"]) != (tag, actual, size):
        raise ValueError("The receipt does not describe this release and archive")
    wallets = receipt["wallets"]
    if not wallets:
        raise ValueError("The receipt lists no wallet")
    seeds = ", ".join(wallet["seed_hash"] for wallet in wallets)
    return {
        "id": f"{tag}-wallet-only",
        "git_tag": tag,
        "det_version": det_version,
        "capture_method": "release-chain",
        "profile": "wallet-only",
        "network": "testnet",
        "host_platform": "x86_64-linux",
        "binary_asset": "dash-evo-tool-linux-x86_64.zip",
        "platform_rev": platform_rev,
        "captured_at": today.isoformat(),
        "provenance": {
            "source": receipt["source"],
            "binary_sha256": receipt["binary_sha256"],
        },
        "artifact": {
            "workflow_name": WORKFLOW,
            "workflow_run_id": run_id,
            "artifact_name": asset_name(tag, "tar.zst").removesuffix(".tar.zst"),
            "archive_filename": asset_name(tag, "tar.zst"),
            "sha256": actual,
            "bytes": size,
        },
        "expect": {
            "wallet_aliases": sorted(wallet["alias"] for wallet in wallets),
            "derive_address": False,
        },
        "contents": {"wallets": [], "identity": None},
        "notes": f"Captured by {WORKFLOW} and attached to the release: the "
        f"{receipt['source']['git_tag']} fixture advanced by this release's own det-cli. "
        "Inherits that profile's limits; the capture ran offline wallet listing only. "
        f"Wallet seed hash: {seeds}.",
    }


def check(entry: dict, archive: Path, tag: str) -> None:
    """Reject an entry that is not this release's password-free, offline profile."""
    if not isinstance(entry, dict):
        raise ValueError("The entry is not an object")
    artifact, expect = entry.get("artifact"), entry.get("expect")
    if not isinstance(artifact, dict) or not isinstance(expect, dict):
        raise ValueError("The entry lacks its artifact or expect block")
    if set(entry) - ENTRY_KEYS or set(artifact) - ARTIFACT_KEYS:
        raise ValueError("The entry carries a field an attached fixture may not set")
    aliases = expect.get("wallet_aliases")
    required = {
        "release": entry.get("git_tag") == tag
        and entry.get("id") == f"{tag}-wallet-only",
        "profile": (
            entry.get("capture_method"),
            entry.get("profile"),
            entry.get("network"),
        )
        == ("release-chain", "wallet-only", "testnet"),
        "archive name": artifact.get("archive_filename")
        == archive.name
        == asset_name(tag, "tar.zst"),
        "archive size": artifact.get("bytes") == archive.stat().st_size,
        "archive digest": artifact.get("sha256") == digest(archive),
        # No wallet entry may name a password variable, and nothing may need the network.
        "contents": entry.get("contents") == {"wallets": [], "identity": None},
        "expectations": set(expect) == {"wallet_aliases", "derive_address"}
        and expect["derive_address"] is False
        and isinstance(aliases, list)
        and bool(aliases)
        and all(isinstance(alias, str) for alias in aliases),
    }
    failed = [name for name, holds in required.items() if not holds]
    if failed:
        raise ValueError(f"The entry does not match {tag}: {', '.join(failed)}")


def ordered(tags: list[str]) -> list[str]:
    """Sort by SemVer precedence, dropping tags that are not SemVer."""
    query = (
        'include "semver"; $ARGS.positional'
        " | map(select(try (semver_key | true) catch false)) | sort_by(semver_key)"
    )
    result = subprocess.run(
        ["jq", "-L", str(SCRIPT_DIR), "-cn", query, "--args", *tags],
        text=True,
        capture_output=True,
        check=True,
        timeout=60,
    )
    return json.loads(result.stdout)


def source(manifest: dict, tag: str, published: list[str]) -> dict:
    """Pick the newest fixture older than `tag`, never skipping an uncovered release."""
    fixtures = {fixture["git_tag"]: fixture for fixture in manifest["fixtures"]}
    if tag in fixtures:
        raise ValueError(f"{tag} already has a fixture")
    order = ordered(sorted({*fixtures, *published, tag}))
    if tag not in order:
        raise ValueError(f"{tag} is not a SemVer release tag")
    older = order[: order.index(tag)]
    covered = [candidate for candidate in older if candidate in fixtures]
    if not covered:
        raise ValueError(f"No fixture older than {tag} to start from")
    skipped = [
        candidate
        for candidate in older[older.index(covered[-1]) + 1 :]
        if candidate in published and candidate not in fixtures
    ]
    if skipped:
        raise ValueError(
            f"Capture these published releases first, they have no fixture: {', '.join(skipped)}"
        )
    return fixtures[covered[-1]]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "check", "source"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--receipt", type=Path, help="chain.py receipt (build)")
    parser.add_argument("--det-version")
    parser.add_argument("--platform-rev")
    parser.add_argument("--run-id", type=int)
    parser.add_argument("--out", type=Path, help="entry to write (build)")
    parser.add_argument("--metadata", type=Path, help="entry to verify (check)")
    parser.add_argument("--manifest", type=Path, help="merged manifest (source)")
    parser.add_argument(
        "--published", type=Path, help="published tags, one per line (source)"
    )
    args = parser.parse_args()
    try:
        if args.command == "build":
            if not re.fullmatch(r"[0-9a-f]{40}", args.platform_rev or ""):
                raise ValueError("--platform-rev must be a full commit id")
            entry = build(
                json.loads(args.receipt.read_text()),
                args.archive,
                args.tag,
                args.det_version,
                args.platform_rev,
                args.run_id,
                datetime.now(timezone.utc).date(),
            )
            check(entry, args.archive, args.tag)
            args.out.write_text(json.dumps(entry, indent=2) + "\n")
        elif args.command == "check":
            check(json.loads(args.metadata.read_text()), args.archive, args.tag)
        else:
            chosen = source(
                json.loads(args.manifest.read_text()),
                args.tag,
                args.published.read_text().split(),
            )
            print(json.dumps(chosen))
    except (ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"release_fixture {args.command}: {error}") from error


if __name__ == "__main__":
    main()
