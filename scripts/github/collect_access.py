#!/usr/bin/env python3
"""Collect read-only GitHub access metadata; never request or print secret values."""

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import quote


def fields(value, names):
    return {name: value[name] for name in names if name in value}


def project(kind, data):
    if kind == "user":
        return fields(data, ("login", "id", "type"))
    if kind == "repo":
        return fields(
            data,
            (
                "full_name",
                "default_branch",
                "visibility",
                "permissions",
                "allow_auto_merge",
                "security_and_analysis",
            ),
        )
    if kind == "secrets":
        return {
            "secrets": [
                fields(s, ("name", "created_at", "updated_at", "visibility"))
                for s in data.get("secrets", [])
            ]
        }
    if kind == "keys":
        return [
            fields(k, ("id", "title", "read_only", "verified", "created_at"))
            for k in data
        ]
    if kind == "permission":
        result = fields(data, ("permission", "role_name", "state", "role"))
        result["user"] = fields(data.get("user", {}), ("login", "id"))
        return result
    if kind == "runners":
        return {
            "runners": [
                fields(r, ("id", "name", "os", "status", "busy", "labels"))
                for r in data.get("runners", [])
            ]
        }
    if kind == "collaborators":
        return [
            fields(c, ("login", "id", "type", "permissions", "role_name")) for c in data
        ]
    if kind == "variable":
        return fields(data, ("name", "value"))
    if kind == "pulls":
        return [fields(pr, ("number", "draft")) for pr in data]
    # The remaining endpoints return access rules and policy metadata, not values.
    return data


def pages(data):
    if not isinstance(data, list) or not data:
        return data
    if len(data) == 1:
        return data[0]
    if all(isinstance(page, list) for page in data):
        return [item for page in data for item in page]
    if all(isinstance(page, dict) for page in data):
        result = dict(data[0])
        for key, value in data[0].items():
            if isinstance(value, list):
                result[key] = [item for page in data for item in page.get(key, [])]
        return result
    raise ValueError("Unexpected API pagination response")


def api_get(kind, endpoint, query=None, paginate=True):
    if query is not None and (
        endpoint != "graphql" or not query.lstrip().startswith("query ")
    ):
        raise ValueError("Only the collector's read-only GraphQL queries are supported")
    command = [
        "gh",
        "api",
        "--method",
        "POST" if query is not None else "GET",
        "-H",
        "X-GitHub-Api-Version: 2026-03-10",
        endpoint,
    ]
    if query is not None:
        command += ["-f", f"query={query}"]
    elif paginate:
        command += ["--paginate", "--slurp"]
    try:
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            check=False,
            timeout=90,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"ok": False, "error": {"message": type(error).__name__}}
    try:
        data = json.loads(result.stdout)
        if query is None and paginate:
            data = pages(data)
    except (json.JSONDecodeError, ValueError):
        return {
            "ok": False,
            "error": {"message": "No usable JSON response", "exit": result.returncode},
        }
    if result.returncode:
        return {
            "ok": False,
            "error": fields(data, ("message", "status", "documentation_url"))
            if isinstance(data, dict)
            else {"message": "API request failed"},
        }
    if isinstance(data, dict) and data.get("errors"):
        return {
            "ok": False,
            "error": {"message": "GraphQL returned incomplete evidence"},
        }
    return {"ok": True, "data": project(kind, data)}


def required_checks(repo, number):
    owner, name = repo.split("/")
    # isRequired is the per-PR source of truth; REST rules alone omitted PR Hygiene.
    query = f'''query {{ repository(owner:"{owner}", name:"{name}") {{
      pullRequest(number:{number}) {{ number baseRefName headRefOid mergeStateStatus
        commits(last:1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(first:100) {{
          pageInfo {{ hasNextPage }} nodes {{ __typename
            ... on StatusContext {{ context state isRequired(pullRequestNumber:{number}) creator {{ login }} }}
            ... on CheckRun {{ name status conclusion isRequired(pullRequestNumber:{number}) }}
          }} }} }} }} }} }}
      }} }} }}'''
    result = api_get("checks", "graphql", query)
    if result["ok"]:
        pr = result["data"]["data"]["repository"]["pullRequest"]
        result["data"] = pr
        if pr is None:
            return {
                "ok": False,
                "error": {"message": "The pull request is no longer available"},
            }
        commits = pr.get("commits", {}).get("nodes", []) if pr else []
        for commit in commits:
            contexts = (commit["commit"].get("statusCheckRollup") or {}).get(
                "contexts", {}
            )
            if contexts.get("pageInfo", {}).get("hasNextPage"):
                result["complete"] = False
                result["limitation"] = (
                    "More than 100 contexts; this PR's check list is incomplete."
                )
    return result


def write_report(output, report):
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="dashpay/dash-evo-tool")
    parser.add_argument("--agent", default="Claudius-Maginificent")
    parser.add_argument("--expected-user", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo):
        parser.error("Use an owner/repository name.")
    if not re.fullmatch(r"[A-Za-z0-9-]+", args.agent):
        parser.error("Use a GitHub account login.")
    operator = api_get("user", "user")
    if (
        not operator["ok"]
        or operator["data"].get("login", "").lower() != args.expected_user.lower()
    ):
        print(
            "Authenticate as the expected administrator in your own terminal; no repository reads were made.",
            file=sys.stderr,
        )
        return 1
    prefix = f"repos/{args.repo}"
    org = args.repo.split("/")[0]
    queries = {
        "repository": ("repo", prefix),
        "agent_permission": (
            "permission",
            f"{prefix}/collaborators/{args.agent}/permission",
        ),
        "agent_org_membership": ("permission", f"orgs/{org}/memberships/{args.agent}"),
        "rulesets": ("rules", f"{prefix}/rulesets?includes_parents=true&per_page=100"),
        "default_rules": ("rules", f"{prefix}/rules/branches/v1.0-dev"),
        "release_rules": ("rules", f"{prefix}/rules/branches/master"),
        "default_legacy_protection": (
            "rules",
            f"{prefix}/branches/v1.0-dev/protection",
        ),
        "release_legacy_protection": ("rules", f"{prefix}/branches/master/protection"),
        "actions_permissions": ("actions", f"{prefix}/actions/permissions"),
        "workflow_permissions": ("actions", f"{prefix}/actions/permissions/workflow"),
        "allowed_actions": (
            "actions",
            f"{prefix}/actions/permissions/selected-actions",
        ),
        "actions_policies": (
            "policies",
            f"{prefix}/actions/policies?has_parents=true&per_page=100",
        ),
        "environments": ("environments", f"{prefix}/environments?per_page=100"),
        "repository_secrets": ("secrets", f"{prefix}/actions/secrets?per_page=100"),
        "organization_secrets": (
            "secrets",
            f"{prefix}/actions/organization-secrets?per_page=100",
        ),
        "deploy_keys": ("keys", f"{prefix}/keys?per_page=100"),
        "runners": ("runners", f"{prefix}/actions/runners?per_page=100"),
        "collaborators": ("collaborators", f"{prefix}/collaborators?per_page=100"),
        "light_runner": ("variable", f"{prefix}/actions/variables/DASH_LIGHT_RUNNER"),
        "hygiene_enabled": (
            "variable",
            f"{prefix}/actions/variables/PR_REVIEW_AUTOMATION_ENABLED",
        ),
    }
    report = {
        "read_only": True,
        "repository": args.repo,
        "agent": args.agent,
        "collected_at": datetime.now(timezone.utc).isoformat(),
        "operator": operator,
        "checks": {},
    }
    for name, (kind, endpoint) in queries.items():
        report["checks"][name] = api_get(kind, endpoint)
    for ruleset in report["checks"]["rulesets"].get("data", []):
        key = f"ruleset_{ruleset['id']}"
        report["checks"][key] = api_get(
            "rules", f"{prefix}/rulesets/{ruleset['id']}?includes_parents=true"
        )
    for environment in (
        report["checks"]["environments"].get("data", {}).get("environments", [])
    ):
        name = environment["name"]
        endpoint = f"{prefix}/environments/{quote(name, safe='')}"
        report["checks"][f"environment_{name}"] = api_get("environments", endpoint)
        report["checks"][f"environment_secrets_{name}"] = api_get(
            "secrets", f"{endpoint}/secrets?per_page=100"
        )
        report["checks"][f"environment_refs_{name}"] = api_get(
            "environments", f"{endpoint}/deployment-branch-policies?per_page=100"
        )
    pull_requests = api_get(
        "pulls", f"{prefix}/pulls?state=open&base=v1.0-dev&per_page=5", paginate=False
    )
    report["checks"]["sampled_prs"] = pull_requests
    if pull_requests["ok"]:
        for pr in pull_requests["data"]:
            if not pr.get("draft"):
                report["checks"][f"required_checks_pr_{pr['number']}"] = (
                    required_checks(args.repo, pr["number"])
                )
    report["limitations"] = (
        "403/404 are unavailable evidence, not proof that protection is disabled. No secret values or token strings are requested."
    )
    write_report(args.output, report)
    missing = [
        name
        for name, result in report["checks"].items()
        if not result["ok"] or result.get("complete") is False
    ]
    print(f"Report: {args.output}")
    print("Unavailable reads: " + (", ".join(missing) if missing else "none"))
    return 2 if missing else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"Collection stopped: {type(error).__name__}", file=sys.stderr)
        raise SystemExit(1)
