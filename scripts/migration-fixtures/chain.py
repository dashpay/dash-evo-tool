"""Advance a verified testnet fixture through an explicit sequence of releases.

The first --release must be the source fixture's version. Each later release
writes a new archive and receipt; no input archive or published fixture changes.
Requires Python 3.12+, jq, tar, and zstd for .tar.zst inputs.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import tarfile
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent


def run(args: list[str], *, env: dict[str, str] | None = None) -> str:
    """Run one bounded command, retaining diagnostics outside the repository."""
    result = subprocess.run(args, env=env, text=True, capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f"{Path(args[0]).name} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def digest(path: Path) -> str:
    """Hash bytes without loading a large binary into memory."""
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_version(binary: Path, tag: str) -> None:
    """Require the binary's embedded version to match the requested release."""
    version = run([str(binary), "--version"])
    if version.split("+", 1)[0] != f"det-cli {tag.removeprefix('v').split('+', 1)[0]}":
        raise ValueError(f"Binary version {version!r} does not match {tag}")


def extract(archive: Path, destination: Path) -> None:
    """Extract only regular files and directories, without escaping staging."""
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as tmp:
        tar_path = archive
        if archive.name.endswith(".zst"):
            tar_path = Path(tmp) / "input.tar"
            with tar_path.open("wb") as output:
                subprocess.run(
                    ["zstd", "-dc", str(archive)], stdout=output, check=True, timeout=60
                )
        with tarfile.open(tar_path) as stream:
            members = stream.getmembers()
            for member in members:
                path = Path(member.name)
                if (
                    path.is_absolute()
                    or ".." in path.parts
                    or not (member.isfile() or member.isdir())
                ):
                    raise ValueError(f"Unsafe archive member: {member.name}")
            stream.extractall(destination, members=members, filter="data")
    destination.chmod(0o700)


def cli(binary: Path, profile: Path, command: str) -> dict:
    """Use a fresh standalone process bound to the staged profile."""
    env = dict(
        os.environ, DASH_EVO_DATA_DIR=str(profile), MCP_API_KEY="", RUST_LOG="off"
    )
    for name in ("GH_TOKEN", "GITHUB_TOKEN"):
        env.pop(name, None)
    for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
        directory = profile.parent / name.lower()
        directory.mkdir(mode=0o700, exist_ok=True)
        env[name] = str(directory)
    return json.loads(run([str(binary), "--standalone", command], env=env))


def clear_credentials(profile: Path) -> None:
    """Remove RPC/MCP credentials from the disposable capture copy only."""
    config = profile / ".env"
    lines = config.read_text().splitlines()
    for index, line in enumerate(lines):
        match = re.match(r"\s*(?:export\s+)?([A-Za-z_][A-Za-z_0-9]*)\s*=", line)
        if match and (
            match[1].lower() == "mcp_api_key"
            or re.search(
                r"_(core_rpc_(user|password)|wallet_private_key)$",
                match[1],
                re.IGNORECASE,
            )
        ):
            lines[index] = f'{match[1]}=""'
    config.write_text("\n".join(lines) + "\n")


def check_wallets(before: list[dict], after: list[dict]) -> None:
    """Fail on lost/replaced wallets, duplicate identities, or empty fixtures."""

    def keys(wallets: list[dict]) -> list[tuple[str, str]]:
        return sorted((wallet["seed_hash"], wallet["alias"]) for wallet in wallets)

    if not before or keys(before) != keys(after):
        raise ValueError(
            "The upgrade changed the wallet identities or aliases, or the baseline was empty"
        )
    if len({wallet["seed_hash"] for wallet in after}) != len(after):
        raise ValueError("Duplicate wallet identity")


def probe(binary: Path, profile: Path) -> list[dict]:
    """Require a persisted testnet selection and successful wallet hydration."""
    if cli(binary, profile, "network-info").get("active") != "testnet":
        raise ValueError(
            "Fixture must boot on testnet; select it with the source release first"
        )
    wallets = cli(binary, profile, "core-wallets-list")["wallets"]
    check_wallets(wallets, cli(binary, profile, "core-wallets-list")["wallets"])
    return wallets


def releases(values: list[str]) -> list[tuple[str, Path]]:
    """Validate version order and every binary before opening any wallet data."""
    result = []
    previous = None
    for value in values:
        tag, binary_name = value.split("=", 1)
        key = json.loads(
            run(
                [
                    "jq",
                    "-L",
                    str(SCRIPT_DIR),
                    "-en",
                    "--arg",
                    "tag",
                    tag,
                    'include "semver"; $tag | semver_key',
                ]
            )
        )
        # Compare using jq: numeric prerelease identifiers sort before strings.
        if (
            previous is not None
            and run(
                [
                    "jq",
                    "-n",
                    "--argjson",
                    "a",
                    json.dumps(previous),
                    "--argjson",
                    "b",
                    json.dumps(key),
                    "$a < $b",
                ]
            )
            != "true"
        ):
            raise ValueError("Release versions must be strictly increasing")
        binary = Path(binary_name).resolve(strict=True)
        check_version(binary, tag)
        result.append((tag, binary))
        previous = key
    if len(result) < 2:
        raise ValueError("Provide the source release and at least one newer release")
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument(
        "--sha256", required=True, help="Source digest from the committed manifest"
    )
    parser.add_argument(
        "--source-tag", required=True, help="Source git_tag from the committed manifest"
    )
    parser.add_argument(
        "--release",
        action="append",
        required=True,
        help="TAG=/absolute/path/to/det-cli",
    )
    parser.add_argument(
        "--work-dir",
        type=Path,
        required=True,
        help="Private staging parent with non-writable ancestors",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        required=True,
        help="New directory for archives and receipts",
    )
    args = parser.parse_args()
    archive = args.archive.resolve(strict=True)
    if digest(archive) != args.sha256:
        raise ValueError("Source archive checksum does not match the manifest")
    hops = releases(args.release)
    if hops[0][0] != args.source_tag:
        raise ValueError("The first release must match the source fixture tag")
    output = args.output_dir.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    args.work_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    source = {
        "git_tag": args.source_tag,
        "archive": archive.name,
        "sha256": args.sha256,
    }
    with tempfile.TemporaryDirectory(prefix="fixture-chain-", dir=args.work_dir) as tmp:
        profile = Path(tmp) / "profile"
        extract(archive, profile)
        clear_credentials(profile)
        baseline = probe(hops[0][1], profile)
        for tag, binary in hops[1:]:
            wallets = probe(binary, profile)
            check_wallets(baseline, wallets)
            prefix = output / f"{tag}-wallet-only"
            packed = Path(
                run(
                    ["bash", str(SCRIPT_DIR / "pack.sh"), str(profile), str(prefix)]
                ).splitlines()[-1]
            )
            receipt = {
                "capture_method": "release-chain",
                "git_tag": tag,
                "network": "testnet",
                "source": source,
                "binary_sha256": digest(binary),
                "wallets": wallets,
                "archive": packed.name,
                "sha256": digest(packed),
                "bytes": packed.stat().st_size,
            }
            Path(f"{prefix}.receipt.json").write_text(
                json.dumps(receipt, indent=2) + "\n"
            )
            source = {
                "git_tag": tag,
                "archive": packed.name,
                "sha256": receipt["sha256"],
            }
            print(f"Verified {tag}: {packed.name}", flush=True)


if __name__ == "__main__":
    main()
