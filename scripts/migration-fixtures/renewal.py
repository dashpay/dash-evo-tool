"""Plan archive renewal and update only verified artifact pointers."""

import argparse
import copy
import hashlib
import json
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path


def timestamp(value: str) -> datetime:
    """Parse an expiry with an explicit timezone."""
    if not isinstance(value, str):
        raise ValueError("missing artifact expiry")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("artifact expiry needs a timezone")
    return parsed


def entries(manifest: dict) -> list[dict]:
    """Validate immutable archive identities before any network access."""
    fixtures = manifest.get("fixtures")
    if not isinstance(fixtures, list) or not fixtures:
        raise ValueError("manifest must contain fixtures")
    seen = {key: set() for key in ("id", "artifact_name", "archive_filename")}
    for fixture in fixtures:
        artifact = fixture["artifact"]
        for key in seen:
            value = fixture["id"] if key == "id" else artifact.get(key, "")
            if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", value):
                raise ValueError(f"invalid {key}")
            if value in seen[key]:
                raise ValueError(f"duplicate {key}: {value}")
            seen[key].add(value)
        if not artifact["archive_filename"].endswith((".tar.zst", ".tar.gz")):
            raise ValueError("unsupported archive extension")
        if not re.fullmatch(r"[0-9a-fA-F]{64}", artifact.get("sha256", "")):
            raise ValueError("invalid archive checksum")
        for key in ("bytes", "workflow_run_id"):
            if type(artifact.get(key)) is not int or artifact[key] <= 0:
                raise ValueError(f"invalid {key}")
        timestamp(artifact.get("expires_at"))
    return fixtures


def plan(manifest: dict, now: datetime, days: int, force: bool = False) -> dict:
    """Select unexpired archives at or within the renewal threshold."""
    if not 1 <= days <= 90:
        raise ValueError("renewal threshold must be between 1 and 90 days")
    selected = []
    for fixture in entries(manifest):
        expiry = timestamp(fixture["artifact"]["expires_at"])
        if expiry <= now:
            raise ValueError(
                f"{fixture['id']}: source artifact has expired; restore a saved archive"
            )
        if force or expiry <= now + timedelta(days=days):
            selected.append({"fixture": fixture})
    return {"include": selected}


def verify_archive(fixture: dict, path: Path) -> None:
    """Verify exact archive size and digest without extracting its contents."""
    artifact = fixture["artifact"]
    if path.name != artifact["archive_filename"] or path.is_symlink():
        raise ValueError("unexpected archive path")
    if path.stat().st_size != artifact["bytes"]:
        raise ValueError("archive size mismatch")
    digest = hashlib.sha256()
    with path.open("rb") as archive:
        for block in iter(lambda: archive.read(1024 * 1024), b""):
            digest.update(block)
    if digest.hexdigest() != artifact["sha256"].lower():
        raise ValueError("archive checksum mismatch")


def renew_manifest(manifest: dict, receipts: list[dict], now: datetime) -> dict:
    """Apply upload receipts only if the source pointers have not changed."""
    result = copy.deepcopy(manifest)
    by_id = {fixture["id"]: fixture for fixture in entries(result)}
    updated = set()
    if not receipts:
        raise ValueError("no renewal receipts")
    for receipt in receipts:
        fixture_id = receipt["fixture_id"]
        if fixture_id not in by_id or fixture_id in updated:
            raise ValueError("unknown or duplicate renewal receipt")
        artifact = by_id[fixture_id]["artifact"]
        if artifact != receipt["source_artifact"]:
            raise ValueError(f"{fixture_id}: source manifest changed; rerun renewal")
        uploaded = receipt["uploaded"]
        run_id = uploaded["workflow_run"]["id"]
        expiry = timestamp(uploaded["expires_at"])
        if (
            uploaded["name"] != artifact["artifact_name"]
            or uploaded["expired"] is not False
            or type(run_id) is not int
            or run_id <= 0
            or run_id == artifact["workflow_run_id"]
            or expiry <= max(now, timestamp(artifact["expires_at"]))
        ):
            raise ValueError(
                f"{fixture_id}: upload does not extend this archive's retention"
            )
        artifact.update(
            workflow_name="migration-fixture-renewal.yml",
            workflow_run_id=run_id,
            retention_days=90,
            expires_at=uploaded["expires_at"],
        )
        updated.add(fixture_id)
    return result


def main() -> None:
    """Expose planning, downloaded archive verification and manifest updates."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("plan", "verify", "apply"))
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--days", type=int, default=30)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--archives", type=Path)
    parser.add_argument("--receipts", type=Path)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    now = datetime.now(timezone.utc)
    if args.command == "plan":
        print(
            json.dumps(
                plan(manifest, now, args.days, args.force), separators=(",", ":")
            )
        )
    elif args.command == "verify":
        if args.archives is None:
            parser.error("verify requires --archives")
        for fixture in entries(manifest):
            verify_archive(
                fixture, args.archives / fixture["artifact"]["archive_filename"]
            )
            print(f"Verified exact archive bytes: {fixture['id']}")
    else:
        if args.receipts is None:
            parser.error("apply requires --receipts")
        receipts = [
            json.loads(path.read_text())
            for path in sorted(args.receipts.glob("*.json"))
        ]
        result = renew_manifest(manifest, receipts, now)
        args.manifest.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
