#!/usr/bin/env python3
"""Validate and package the scoped defensive audit's final evidence.

This script does not run tests, build software, contact services, or change the
manuscript. Run it only after the named final test logs are complete. Historical
records remain verbatim; the run manifest explains their original run labels.
"""

import argparse
from collections import Counter
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import zipfile


ROOT = Path(__file__).resolve().parents[2]
ASSESSMENT = ROOT / "docs/security/assessment-2026-10"
DEFAULT_BUNDLE = Path(
    "/Users/wgu/Desktop/biorouter-manuscript/manuscript/v1.1/Security Audit Raw Data"
)
REQUIRED_TESTS = {
    "controlled_approval_and_guardrail_observations",
    "session_directories_are_private_and_owner_access_remains_functional",
    "session_storage_refuses_a_symlinked_directory",
    "judge_receives_arguments_and_distinguishes_same_tool_invocations",
    "malformed_or_unsolicited_judge_decisions_fail_closed",
    "duplicate_request_ids_are_not_sent_for_ambiguous_classification",
    "inspector_does_not_reuse_judge_approval_for_another_invocation",
    "legacy_smart_approve_allow_requires_a_fresh_decision",
    "explicit_user_allow_still_authorizes_unannotated_tools",
    "legacy_smart_approve_restrictions_still_apply",
    "remembered_scope_refuses_unresolved_links_and_preserves_normal_new_files",
    "legacy_permission_file_does_not_restore_automatic_tool_wide_grants",
}
EDITOR_TEST = "public_editor_replay_preserves_workspace_boundaries_and_normal_edits"
EDITOR_CASES = {
    "direct_write:file-link",
    "direct_write:dir-link/new.txt",
    "direct_write:outside-link/missing/new.txt",
    "direct_write:normal_file",
    "direct_write:valid_internal_symlink",
    "diff_creation:file-link",
    "diff_creation:dir-link/new.txt",
    "diff_creation:outside-link/missing/new.txt",
    "diff_creation:workspace/outside-link/missing/new.txt",
    "diff_creation:normal_file",
    "diff_creation:overlap_prefixed_internal_nested_file",
}
SUCCESS = re.compile(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", re.M)
TEST_START = re.compile(r"^test (\S+) \.\.\.\s*(.*)$")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def utc(timestamp=None):
    value = datetime.now(timezone.utc) if timestamp is None else datetime.fromtimestamp(timestamp, timezone.utc)
    return value.isoformat()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def at_root(value):
    path = Path(value)
    return path.resolve() if path.is_absolute() else (ROOT / path).resolve()


def git(*args, allowed=(0,)):
    result = subprocess.run(["git", *args], cwd=ROOT, capture_output=True)
    require(result.returncode in allowed, f"git {' '.join(args)} failed: {result.stderr.decode(errors='replace')}")
    return result.stdout


def file_metadata(path):
    data = path.read_bytes()
    stat = path.stat()
    return {"path": str(path), "bytes": len(data), "sha256": sha256(data),
            "filesystem_mtime_utc": utc(stat.st_mtime)}


def records(path, marker="SECURITY_ASSESSMENT"):
    result = []
    for line in path.read_text().splitlines():
        match = re.search(re.escape(marker) + r" (\{.*\})", line)
        if match:
            result.append(json.loads(match.group(1)))
    return result


def jsonl(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def parse_test_log(path):
    data = re.sub(r"\x1b\[[0-9;]*m", "", path.read_text())
    summaries = SUCCESS.findall(data)
    require(summaries, f"No completed successful Rust test summary in {path}")
    require(not re.search(r"test result: FAILED|error: could not compile", data), f"Failure in final verification log {path}")
    require(all(int(failed) == 0 for _, failed, _ in summaries), f"Failed tests in {path}")
    passed = []
    pending = None
    for line in data.splitlines():
        match = TEST_START.match(line)
        if match:
            pending = match.group(1)
            if match.group(2).strip() == "ok":
                passed.append(pending)
                pending = None
        elif pending and line.strip() == "ok":
            passed.append(pending)
            pending = None
    return {**file_metadata(path), "successful_summaries": len(summaries),
            "passed_tests": sum(int(item[0]) for item in summaries),
            "ignored_tests": sum(int(item[2]) for item in summaries),
            "named_tests_passed": passed}


def short_name(name):
    return name.rsplit("::", 1)[-1]


def validate_session_guard(assessment):
    directory = assessment / "environment/session-guard-regression"
    summary = json.loads((directory / "summary.json").read_text())
    runs = summary.get("runs", [])
    require(len(runs) == 3 and {row["run"] for row in runs} == {1, 2, 3}, "Session guard requires three completed, distinct runs")
    for row in runs:
        require(row.get("permissive_fixture_readable_before_guard") is True
                and row.get("owner_control_allowed") is True
                and row.get("observer_denied") is True
                and row.get("owner_uid") == 10001
                and row.get("observer_uid") == 10002
                and row.get("guard_exit") == 0
                and row.get("session_mode") == "700",
                f"Session guard run not successful: {row}")
    require(summary.get("foreign_owned_directory_rejected") is True, "Foreign-owner guard regression did not pass")
    require(summary.get("final_symlink_rejected_without_chmod_target") is True, "Session symlink guard regression did not pass")
    source = (ROOT / "crates/biorouter/src/session/session_manager.rs").read_text()
    start = source.index("    fn protect_session_directory(path:")
    end = source.index("    #[cfg(not(unix))]", start)
    require(summary.get("guard_sha256") == sha256(source[start:end].strip().encode()), "Session guard evidence does not match current production helper")
    require(summary.get("extracted_source_sha256") == sha256((directory / "extracted-guard.rs").read_bytes()), "Extracted helper hash mismatch")
    require((directory / "build.log").is_file(), "Session guard build log missing")
    return summary


def validate_file_runs(assessment):
    runs = []
    stages = {}
    for prefix, stage in [("file-api", "baseline"), ("post-change-file-api", "post-change")]:
        combined = []
        for run in range(1, 4):
            path = assessment / f"tool-boundary/raw/{prefix}-repeat-{run}.log"
            rows = records(path)
            require(len(rows) == 8 and len({row["case_id"] for row in rows}) == 8, f"Expected eight file-API cases in {path}")
            counts = Counter(row["outcome"] for row in rows)
            expected = {"blocked": 5, "attack_succeeded": 2, "allowed_control": 1} if stage == "baseline" else {"blocked": 7, "allowed_control": 1}
            require(counts == expected, f"Unexpected {stage} file API outcomes: {path}: {dict(counts)}")
            if stage == "post-change":
                require(all(not row["observed"].get("outside_canary_created", False)
                            and not row["observed"].get("outside_file_created", False)
                            for row in rows), f"Post-change outside filesystem effect in {path}")
            runs.append({**file_metadata(path), "relative_path": path.relative_to(assessment).as_posix(),
                         "stage": stage, "run_id": f"workspace-{stage}-r{run}", "run_number_from_filename": run,
                         "embedded_repetition_values": sorted({row.get("repetition") for row in rows}),
                         "case_observations": len(rows), "outcomes": dict(counts),
                         "note": "Original embedded repetition values are preserved. Distinct invocation number is derived from the retained log filename."})
            combined.extend(rows)
        stages[stage] = combined
    return runs, stages


def validate_editor_runs(paths):
    logs = [parse_test_log(path) for path in paths]
    passed = Counter(short_name(name) for log in logs for name in log["named_tests_passed"])
    require(len(paths) == 3 and len(set(paths)) == 3, "Three distinct editor replay log files are required")
    require(passed[EDITOR_TEST] == 3, f"Require three successful public editor replays: {dict(passed)}")
    normalized = []
    run_manifest = []
    for run, path in enumerate(paths, 1):
        rows = records(path, "EDITOR_AUDIT")
        require(len(rows) == 11, f"Expected eleven public editor observations in {path}")
        cases = set()
        for row in rows:
            case = f"{row['operation']}:{row.get('path', row.get('control'))}"
            require(case not in cases, f"Duplicate editor observation: {path}: {case}")
            cases.add(case)
            if "path" in row:
                require(row.get("denied") is True and row.get("outside_entries") == 0,
                        f"Editor denial or unchanged-outside assertion missing: {path}: {row}")
                outcome = "blocked"
            else:
                require(row.get("allowed") is True, f"Editor valid-use control was not allowed: {path}: {row}")
                outcome = "allowed_control"
            normalized.append({"case_id": case, "outcome": outcome})
        require(cases == EDITOR_CASES, f"Public editor case set differs from the final source fixture: {path}: {sorted(cases)}")
        require(sum("path" in row for row in rows) == 7, f"Expected seven editor negative cases in {path}")
        run_manifest.append({**file_metadata(path), "run_number_from_cli_order": run,
                             "case_observations": 11, "expected_denials_observed": 7,
                             "valid_controls_allowed": 4, "outside_entries_after_denials": 0,
                             "note": "EDITOR_AUDIT records are retained verbatim; case keys combine operation and path/control."})
    require(len({row["case_id"] for row in normalized}) == 11, "Editor replay case sets differ between runs")
    return logs, run_manifest, normalized


def make_patch(base_sha):
    patch = git("diff", "--binary", "--no-ext-diff", base_sha, "--", "crates", "Cargo.lock", "Cargo.toml")
    untracked = git("ls-files", "--others", "--exclude-standard", "--", "crates").decode().splitlines()
    for name in sorted(name for name in untracked if name.endswith(".rs")):
        patch += git("diff", "--binary", "--no-ext-diff", "--no-index", "--", "/dev/null", name, allowed=(0, 1))
    require(patch, "No source remediation patch available")
    return patch


def source_hashes():
    tracked = git("ls-files", "--", "*.rs", "Cargo.lock", "Cargo.toml", "crates/*/Cargo.toml").decode().splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard", "--", "*.rs").decode().splitlines()
    return {name: sha256((ROOT / name).read_bytes()) for name in sorted(set(tracked + untracked)) if (ROOT / name).is_file()}


def nonfinal_log_metadata(assessment, final_logs):
    retained = []
    for path in sorted(assessment.glob("tool-boundary/raw/post-change*.log")):
        if path in final_logs or re.fullmatch(r"post-change-file-api-repeat-[123]\.log", path.name):
            continue
        if "paused" in path.name or "aborted" in path.name:
            status = "aborted_or_paused_build"
            interpretation = "Retained incomplete build record; not a completed test and not counted as a test failure."
        elif "compile-attempt" in path.name:
            status = "superseded_compile_error"
            interpretation = "Earlier compiler error retained as process history; separate final logs determine current verification."
        else:
            status = "other_build_or_setup_record"
            interpretation = "Retained process record; no test-success or security-outcome count inferred."
        retained.append({**file_metadata(path), "status": status, "interpretation": interpretation})
    return retained


def final_execution_metadata(assessment, verification_logs, editor_logs):
    path = assessment / "tool-boundary/final-regression-executions.json"
    rows = json.loads(path.read_text())
    require(isinstance(rows, list) and len(rows) == 7, "Seven recorded final regression invocations are required")
    require(Counter(row.get("run_id") for row in rows)
            == {"core": 3, "scope": 1, "editor-1": 1, "editor-2": 1, "editor-3": 1},
            "Final execution provenance has unexpected run IDs")
    expected_logs = set(verification_logs + editor_logs)
    for row in rows:
        binary = Path(row["binary"])
        require(row.get("exit_code") == 0, f"Final invocation did not pass: {row}")
        require(row.get("command", [None])[0] == str(binary), f"Invocation command/binary mismatch: {row}")
        require(row.get("started_unix", 0) > 0 and row.get("seconds", -1) >= 0, f"Invocation timing missing: {row}")
        require(at_root(row["log"]) in expected_logs, f"Execution provenance names an unvalidated log: {row['log']}")
        require(sha256(binary.read_bytes()) == row.get("sha256"), f"Current binary differs from final execution capture: {binary}")
    return {**file_metadata(path), "invocations": rows,
            "note": "These binary hashes and command timings were captured by the final regression runner. Earlier historical file-API runs have no retrospectively invented executable hash."}


def static_check_metadata(assessment):
    directory = assessment / "verification"
    source = directory / "static-checks.json"
    final_source = directory / "final-checks.json"
    require(source.is_file(), "Static-check invocation records are missing")
    require(final_source.is_file(), "Final static and lint outcome records are missing")
    invocations = []
    for row in json.loads(source.read_text()):
        item = dict(row)
        log = directory / (Path(row["command"]).stem + ".log")
        require(log.is_file(), f"Static-check log missing: {log}")
        text = log.read_text()
        if row["exit_code"] == 0:
            item["status"] = "script_invocation_passed"
            item["interpretation"] = "Only this invocation is reported; prerequisites and full repository checks may have a broader scope."
        elif "FileNotFoundError" in text and "No such file or directory: 'cargo'" in text:
            item["status"] = "incomplete_check_setup_missing_cargo_path"
            item["interpretation"] = "The check stopped before its security gate because cargo was absent from PATH. This is an incomplete setup attempt, not a demonstrated vulnerability. Retain any subsequent rerun separately."
        else:
            item["status"] = "check_not_passed"
            item["interpretation"] = "No passing result is inferred. A nonzero check exit alone is not classified as a security vulnerability."
        item["log"] = file_metadata(log)
        invocations.append(item)
    final_summary = json.loads(final_source.read_text())
    final_outcomes = []
    seen = set()
    for row in final_summary["checks"]:
        item = dict(row)
        check_id = row["check_id"]
        require(check_id not in seen, f"Duplicate final check ID: {check_id}")
        seen.add(check_id)
        require(row["status"] in {"passed", "not_passed", "not_run", "not_yet_recorded"},
                f"Unknown final check status: {check_id}: {row['status']}")
        if row["status"] == "passed":
            require(row.get("exit_code") == 0 and row.get("log"),
                    f"Passing final check requires exit zero and a retained log: {check_id}")
        elif row["status"] == "not_passed":
            require(isinstance(row.get("exit_code"), int) and row["exit_code"] != 0 and row.get("log"),
                    f"Nonpassing final check requires its nonzero exit and retained log: {check_id}")
        else:
            require(row.get("exit_code") is None,
                    f"Unrun or unrecorded check cannot claim an exit code: {check_id}")
        if row.get("log"):
            log = (directory / row["log"]).resolve()
            require(log.is_relative_to(directory.resolve()) and log.is_file(),
                    f"Final check log missing or outside verification directory: {check_id}")
            metadata = file_metadata(log)
            require(metadata["sha256"] == row.get("log_sha256"),
                    f"Final check log hash differs from its outcome record: {check_id}")
            for marker in row.get("evidence_strings", []):
                require(marker in log.read_text(), f"Final check evidence marker missing: {check_id}: {marker}")
            item["log_metadata"] = metadata
        final_outcomes.append(item)
    by_id = {row["check_id"]: row for row in final_outcomes}
    for check_id in ["production_and_focused_tests_clippy", "non_inheritable_sockets_final"]:
        require(by_id.get(check_id, {}).get("status") == "passed",
                f"Required final check did not pass: {check_id}")
    require(by_id["non_inheritable_sockets_final"].get("production_targets") == 17,
            "Final socket gate must record its seventeen production targets")
    return {"source_summary": file_metadata(source), "invocations": invocations,
            "final_outcome_summary": file_metadata(final_source), "final_outcomes": final_outcomes,
            "final_recording_note": final_summary["recording_note"],
            "retained_files": [file_metadata(path) for path in sorted(directory.rglob("*")) if path.is_file()],
            "scope": "Static, lint and setup records remain separate from security-case outcomes; no completion of just check-everything is inferred."}


def count_row(stream, stage, rows, repetitions, notes):
    counts = Counter(row["outcome"] for row in rows)
    return {"evidence_stream": stream, "stage": stage,
            "distinct_cases": len({row["case_id"] for row in rows}), "repetitions": repetitions,
            "observations": len(rows), "expected_denials_observed": counts["blocked"],
            "valid_controls_allowed": counts["allowed_control"], "boundary_failures": counts["attack_succeeded"],
            "diagnostic_inconclusive": counts["inconclusive"], "unexercised_observations": counts["not_exercised"],
            "notes": notes}


def case_counts(network, local, file_stages, baseline_component, current_component, editor, session):
    rows = [
        count_row("API and browser-session protocol boundaries", "release baseline", network, 3,
                  "40 distinct expected-denial cases; 14 exercised controls; one control unexercised in each repetition; no browser engine"),
        count_row("Separate-account storage and assessment containment", "release baseline", local, 3,
                  "Includes six product local-account cases and two assessment-container cases; one disclosure reproduced three times"),
        count_row("Workspace file APIs", "source baseline", file_stages["baseline"], 3,
                  "Two dangling-link paths; host-side file APIs, not Docker process isolation"),
        count_row("Workspace file APIs", "post-change component", file_stages["post-change"], 3,
                  "Seven negative cases and one valid-use control per run; historical raw embedded repetition remains 1"),
        count_row("Permission, output framing and redaction components", "source baseline", baseline_component, 1,
                  "Scripted providers; approval-boundary decisions only; no dispatched side effects; five diagnostic records are not empirical attack outcomes"),
        count_row("Permission, output framing and redaction components", "post-change component", current_component, 1,
                  "Scripted providers; no live-model robustness or general security pass rate inferred"),
        count_row("Public developer editor", "post-change component", editor, 3,
                  "Seven negative cases and four valid-use controls per run; direct writes and diff creation including overlap-prefixed paths; outside directories unchanged"),
    ]
    rows.append({"evidence_stream": "Unix session-directory guard", "stage": "post-change extracted-helper fixture",
                 "distinct_cases": "paired-account fixture plus two refusal conditions", "repetitions": "3 paired fixtures; one per refusal condition",
                 "observations": "not pooled with case-level corpus", "expected_denials_observed": "3/3 observer reads; foreign-owner and final-symlink refusal",
                 "valid_controls_allowed": "3/3 owner reads; all permissive pre-guard controls readable", "boundary_failures": 0,
                 "diagnostic_inconclusive": "", "unexercised_observations": "",
                 "notes": session["limitation"]})
    output = io.StringIO(newline="")
    writer = csv.DictWriter(output, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(rows)
    return output.getvalue().encode()


def immutable_baseline(relative):
    return (relative.startswith("network/repetition-") or relative.startswith("network/combined-results")
            or relative.startswith("environment/raw/local/") or relative == "environment/local-results.csv"
            or re.fullmatch(r"tool-boundary/raw/file-api-repeat-[123]\.log", relative) is not None
            or relative == "tool-boundary/raw/core-and-regressions.log")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assessment-dir", type=at_root, default=ASSESSMENT)
    parser.add_argument("--bundle-dir", type=at_root, default=DEFAULT_BUNDLE)
    parser.add_argument("--verification-log", type=at_root, action="append", required=True,
                        help="Completed current-source Rust logs, together containing all required core, scope and config regressions; repeat as needed")
    parser.add_argument("--editor-replay-log", type=at_root, action="append", required=True,
                        help="Completed production editor replay log(s), with each editor regression passing three times total")
    parser.add_argument("--binary", type=at_root, action="append", required=True,
                        help="Exact current test executable to hash now; repeat for each relevant executable")
    parser.add_argument("--primary-checkout", type=at_root, default=Path("/Users/wgu/Desktop/BioRouter"),
                        help="Original shared checkout whose HEAD is recorded separately from the frozen audit base")
    parser.add_argument("--expected-test", action="append", default=[], help="Additional required successful Rust test name")
    args = parser.parse_args()
    assessment, bundle = args.assessment_dir, args.bundle_dir
    require(bundle.is_dir() and any(bundle.iterdir()), f"Existing curated baseline bundle required: {bundle}")
    verification = [parse_test_log(path) for path in args.verification_log]
    passed = {short_name(name) for log in verification for name in log["named_tests_passed"]}
    missing = (REQUIRED_TESTS | set(args.expected_test)) - passed
    require(not missing, f"Required final test successes absent: {sorted(missing)}")
    editor, editor_runs, editor_observations = validate_editor_runs(args.editor_replay_log)
    execution = final_execution_metadata(assessment, args.verification_log, args.editor_replay_log)
    static_checks = static_check_metadata(assessment)
    session = validate_session_guard(assessment)
    file_runs, file_stages = validate_file_runs(assessment)
    network = jsonl(assessment / "network/combined-results.jsonl")
    local = list(csv.DictReader((assessment / "environment/local-results.csv").open()))
    baseline_component = records(assessment / "tool-boundary/raw/core-and-regressions.log")
    current_component = [row for path in args.verification_log for row in records(path)]
    require(len(network) == 165 and Counter(row["outcome"] for row in network) == {"blocked": 120, "allowed_control": 42, "not_exercised": 3}, "Frozen network baseline mismatch")
    require(len(local) == 24 and Counter(row["outcome"] for row in local) == {"blocked": 12, "allowed_control": 9, "attack_succeeded": 3}, "Frozen local-account baseline mismatch")
    require(len(baseline_component) == 14, "Fourteen baseline component observations required")
    require(Counter(row["outcome"] for row in baseline_component)
            == {"blocked": 4, "allowed_control": 3, "attack_succeeded": 2, "inconclusive": 5},
            "Frozen component baseline mismatch")
    require(len(current_component) == 14 and len({row["case_id"] for row in current_component}) == 14, "Fourteen distinct current component observations required in final logs")
    require({row["case_id"] for row in current_component} == {row["case_id"] for row in baseline_component},
            "Current component observation IDs differ from the frozen case set")
    require(Counter(row["outcome"] for row in current_component)
            == {"blocked": 6, "allowed_control": 3, "inconclusive": 5},
            "Current component evidence requires six denials, three controls and five explicitly diagnostic records; no unresolved failure")
    binaries = [file_metadata(path) for path in args.binary]
    collected = utc()
    sources = source_hashes()
    frozen = json.loads((assessment / "environment/manifest.json").read_text())
    patch_base = frozen["assessment_main_commit"]
    git("cat-file", "-e", patch_base + "^{commit}")
    patch = make_patch(patch_base)
    primary_head = git("-C", str(args.primary_checkout), "rev-parse", "HEAD").decode().strip()
    ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", primary_head, frozen["assessment_main_commit"]], cwd=ROOT, capture_output=True)
    require(ancestor.returncode in (0, 1), "Could not compare primary checkout with frozen audit source base")
    snapshot = {"collected_at_utc": collected, "git_head_sha": git("rev-parse", "HEAD").decode().strip(),
                "git_branch": git("branch", "--show-current").decode().strip(),
                "frozen_audit_source_base_sha": frozen["assessment_main_commit"],
                "release_baseline_tag": frozen["release_tag"], "release_baseline_commit_sha": frozen["release_commit"],
                "release_baseline_package_sha256": frozen["package_sha256"],
                "release_to_audit_source_diff": file_metadata(assessment / "environment/release-to-assessment.diff"),
                "primary_checkout": str(args.primary_checkout), "primary_checkout_head_sha": primary_head,
                "primary_checkout_is_ancestor_of_frozen_audit_base": ancestor.returncode == 0,
                "checkout_note": "The original checkout HEAD is recorded separately. An older original checkout does not change the frozen released-package and source baseline used for this audit.",
                "source_files_sha256": sources, "source_fixes_patch_sha256": sha256(patch),
                "source_fixes_patch_base_sha": patch_base,
                "packaging_script_sha256": sha256(Path(__file__).read_bytes()), "current_executables": binaries,
                "provenance_limit": "Executable hashes in current_executables were collected at packaging time, not captured during historical runs. Filesystem mtimes are not attested execution times. Source hashes identify the current working tree. The source patch compares the frozen assessment source base with the current tree, including new Rust tests, so creating a local audit commit does not erase the remediation diff."}
    final_verification = {"collected_at_utc": collected, "required_test_names": sorted(REQUIRED_TESTS | set(args.expected_test)),
                          "current_source_test_logs": verification, "editor_replay_logs": editor,
                          "final_execution_capture": execution, "static_and_lint_records": static_checks,
                          "nonfinal_build_and_setup_records": nonfinal_log_metadata(
                              assessment, set(args.verification_log + args.editor_replay_log)),
                          "editor_replay": {"test_name": EDITOR_TEST, "successful_runs": 3,
                                            "case_observations": 33, "expected_denials_observed": 21,
                                            "valid_controls_allowed": 12, "outside_entries_after_denials": 0},
                          "session_guard_evidence": session,
                          "limit": "Only recorded completed successes are reported. Earlier paused, failed-build, baseline or diagnostic records remain separate; an extracted production helper is not a rebuilt BioRouter/SQLite end-to-end retest."}

    # Refresh already curated material. Historical raw baseline bytes may never
    # be silently replaced by newer or corrected records.
    for destination in sorted(bundle.rglob("*")):
        if not destination.is_file():
            continue
        relative = destination.relative_to(bundle).as_posix()
        source = assessment / relative
        if source.is_file():
            value = source.read_bytes()
            require(not immutable_baseline(relative) or destination.read_bytes() == value,
                    f"Historical baseline differs between curated bundle and source: {relative}")
            if destination.read_bytes() != value:
                destination.write_bytes(value)
    additions = set(assessment.glob("tool-boundary/raw/post-change*.log"))
    additions.update(assessment.glob("tool-boundary/raw/*scope*.log"))
    additions.update((assessment / "environment/session-guard-regression").rglob("*"))
    additions.update((assessment / "verification").rglob("*"))
    additions.update(path for path in [assessment / "tool-boundary/final-regression-executions.json",
                                      assessment / "tool-boundary/finding-smart-approve.md",
                                      assessment / "environment/release-to-assessment.diff",
                                      assessment / "environment/session-guard-run.log"] if path.is_file())
    additions.update(args.verification_log)
    additions.update(args.editor_replay_log)
    for source in sorted(additions):
        if source.is_file():
            require(source.is_relative_to(assessment), f"Final evidence log must be inside the assessment directory: {source}")
            destination = bundle / source.relative_to(assessment)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read_bytes())
    (bundle / "source-fixes.patch").write_bytes(patch)
    (bundle / "case-counts.csv").write_bytes(case_counts(network, local, file_stages, baseline_component, current_component, editor_observations, session))
    for name, value in [("source-provenance.json", snapshot), ("post-change-verification.json", final_verification),
                        ("run-manifest.json", {"collected_at_utc": collected, "workspace_file_api_runs": file_runs,
                                               "public_editor_runs": editor_runs})]:
        (bundle / name).write_text(json.dumps(value, indent=2) + "\n")
    require(source_hashes() == sources, "Source files changed while packaging; rerun after work is stable")
    require(make_patch(patch_base) == patch, "Source patch changed while packaging; rerun after work is stable")
    entries = {path.relative_to(bundle).as_posix(): {"sha256": sha256(path.read_bytes()), "bytes": path.stat().st_size}
               for path in sorted(bundle.rglob("*")) if path.is_file() and path.name != "bundle-manifest.json"}
    (bundle / "bundle-manifest.json").write_text(json.dumps({"collected_at_utc": collected,
        "note": "This manifest excludes itself to avoid a self-referential hash.", "files": entries}, indent=2) + "\n")
    archive = bundle.with_suffix(".zip")
    temporary = archive.with_suffix(".zip.tmp")
    with zipfile.ZipFile(temporary, "w", compression=zipfile.ZIP_DEFLATED) as output:
        for path in sorted(bundle.rglob("*")):
            if path.is_file():
                output.write(path, Path(bundle.name) / path.relative_to(bundle))
    with zipfile.ZipFile(temporary) as output:
        require(output.testzip() is None, "ZIP integrity check failed")
        zip_entries = len(output.infolist())
    temporary.replace(archive)
    print(json.dumps({"bundle": str(bundle), "archive": str(archive), "archive_sha256": sha256(archive.read_bytes()),
                      "zip_entries": zip_entries, "zip_integrity": "passed", "patch_sha256": sha256(patch),
                      "source_files_hashed": len(sources), "current_executables_hashed": len(binaries),
                      "all_required_recorded_successes_present": True}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"Evidence finalization refused: {error}", file=sys.stderr)
        sys.exit(1)
