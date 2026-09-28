#!/usr/bin/env python3
"""Validate a captured three-account Crew fixture report without contacting AWS.

The provisioner may use elevated setup to create disposable accounts, but the
product workflow recorded in the report must be privilege-free. This validator
is deliberately offline so a malformed report cannot trigger a cloud action.
"""

import argparse
import json
from pathlib import Path
import sys


REQUIRED_USERS = {"crew_alice", "crew_bob", "crew_carol"}


def fail(message: str) -> None:
    raise ValueError(message)


def validate(report: dict) -> list[str]:
    errors: list[str] = []
    accounts = report.get("accounts")
    if not isinstance(accounts, list) or {a.get("name") for a in accounts if isinstance(a, dict)} != REQUIRED_USERS:
        errors.append("accounts must contain exactly crew_alice, crew_bob and crew_carol")
        accounts = []
    else:
        by_name = {a["name"]: a for a in accounts}
        uids = [a.get("uid") for a in accounts]
        keys = [a.get("ssh_key_id") for a in accounts]
        homes = [a.get("home") for a in accounts]
        if len(set(uids)) != 3 or any(not isinstance(uid, int) or uid <= 0 for uid in uids):
            errors.append("accounts must have three distinct non-root numeric UIDs")
        if len(set(keys)) != 3 or any(not key for key in keys):
            errors.append("accounts must have three distinct non-empty SSH key IDs")
        if any(home != f"/home/{name}" for name, home in ((name, by_name[name].get("home")) for name in REQUIRED_USERS)):
            errors.append("each account home must be its own /home/<username>")

    broker = report.get("broker")
    if not isinstance(broker, dict):
        errors.append("broker mapping is required")
    else:
        if broker.get("owner") != "crew_alice":
            errors.append("broker owner must be crew_alice")
        if broker.get("uid") != next((a.get("uid") for a in accounts if a.get("name") == "crew_alice"), None):
            errors.append("broker UID must equal crew_alice UID")
        state_path = broker.get("state_path")
        if not isinstance(state_path, str) or not state_path.startswith("/home/crew_alice/"):
            errors.append("broker state must be under crew_alice's home")

    workflow = report.get("product_workflow")
    if not isinstance(workflow, dict):
        errors.append("product_workflow mapping is required")
    else:
        if workflow.get("sudo_used") is not False:
            errors.append("product workflow must record sudo_used=false")
        if workflow.get("global_install") is not False:
            errors.append("product workflow must record global_install=false")
        if workflow.get("new_system_account") is not False:
            errors.append("product workflow must record new_system_account=false")
        if workflow.get("new_unix_group") is not False:
            errors.append("product workflow must record new_unix_group=false")
        if workflow.get("ssh_policy_changed") is not False:
            errors.append("product workflow must record ssh_policy_changed=false")

    checks = report.get("checks")
    if not isinstance(checks, list):
        errors.append("checks must be a list")
    elif any(check.get("result") != "pass" for check in checks if isinstance(check, dict)):
        errors.append("fixture checks may not contain a non-pass result")
    if report.get("cleanup", {}).get("verified") is not True:
        errors.append("cleanup.verified must be true before accepting fixture evidence")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    try:
        report = json.loads(args.report.read_text(encoding="utf-8"))
        errors = validate(report)
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"BLOCKED: {error}", file=sys.stderr)
        return 2
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        return 1
    print("PASS: three-user rootless fixture evidence is structurally valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
