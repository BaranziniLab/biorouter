#!/usr/bin/env python3
"""Owner-run M-21 collection using the exact Rust candidate function.

The source is opened read only, backed up into the dedicated PR24 sandbox, and
closed. Eligibility and message filtering read only that consistent snapshot.
No content is printed.

The owner's run root comes from PR24_PRECISION_ROOT, outside the repository.
Without it the run is skipped. Cargo runs from this repository's root, so put
the project's toolchain on PATH first (source bin/activate-hermit).
"""
import argparse
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile

ROOT_VAR = "PR24_PRECISION_ROOT"
REPO_ROOT = Path(__file__).resolve().parents[2]


def run_roots():
    """The sandbox and output folders under the owner's run root, or None when it is unset."""
    raw = os.environ.get(ROOT_VAR, "")
    if not raw:
        return None
    runs = Path(raw)
    return runs / "sandboxes" / "pr24", runs / "pr24-precision"


def run_precision_test(name, env):
    subprocess.run(
        ["cargo", "test", "-p", "biorouter", "--test", "dictation_extractor_precision",
         "--", "--ignored", "--exact", name, "--nocapture"],
        check=True, env=env, cwd=REPO_ROOT,
    )


def dedicated_root(path):
    if path.is_symlink():
        raise ValueError("dedicated PR24 root cannot be a symlink")
    if not path.exists():
        path.mkdir(mode=0o700)
    if path.resolve() != path or not path.is_dir():
        raise ValueError("dedicated PR24 root must be canonical")
    if path.stat().st_mode & 0o077:
        raise ValueError("dedicated PR24 root must be owner only")


def person_written_user_message(session_type, metadata_raw):
    if session_type != "user" or not metadata_raw:
        return False
    try:
        metadata = json.loads(metadata_raw)
    except (ValueError, TypeError):
        return False
    if not isinstance(metadata, dict) or metadata.get("userVisible") is not True:
        return False
    provenance = metadata.get("provenance")
    return provenance is None or (
        isinstance(provenance, dict) and provenance.get("kind") == "user_direct"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-db", required=True, type=Path)
    parser.add_argument("--eligible-manifest", required=True, type=Path)
    parser.add_argument("--run-name", required=True)
    args = parser.parse_args()
    roots = run_roots()
    if roots is None:
        print(json.dumps({"gate": "skipped", "reason": f"{ROOT_VAR} is not set"}))
        return
    sandbox_root, output_root = roots
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{1,40}", args.run_name):
        parser.error("run name must use lowercase letters, digits, underscore or hyphen")
    manifest = json.loads(args.eligible_manifest.read_text())
    if manifest.get("owner_attested_patient_data_excluded") is not True or manifest.get("owner_attested_private_models_excluded") is not True:
        parser.error("owner attestations required")
    ids = manifest.get("eligible_session_ids", [])
    providers = set(manifest.get("public_provider_names", []))
    if not isinstance(ids, list) or not all(isinstance(s, str) for s in ids) or len(ids) != 200 or len(set(ids)) != 200 or not providers:
        parser.error("manifest needs 200 distinct eligible session ids and explicit public provider names")
    dedicated_root(sandbox_root)
    dedicated_root(output_root)
    run = sandbox_root / args.run_name
    run.mkdir(mode=0o700)
    snapshot_path = run / "sessions-snapshot.db"
    filtered = run / "eligible.db"
    run_manifest = run / "manifest.json"
    output = output_root / f"{args.run_name}-candidates.tsv"
    if output.exists():
        parser.error("candidate output already exists")
    env = os.environ.copy()
    env["PR24_SOURCE_DB"] = str(args.source_db)
    run_precision_test("m21_source_path_matches_resolver", env)
    source = sqlite3.connect(args.source_db.resolve().as_uri() + "?mode=ro", uri=True)
    source.execute("PRAGMA query_only=ON")
    fd = os.open(snapshot_path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
    os.close(fd)
    snapshot_writer = sqlite3.connect(snapshot_path)
    source.backup(snapshot_writer)
    snapshot_writer.close()
    source.close()
    fd = os.open(run_manifest, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, "w") as manifest_out:
        json.dump({
            "run_name": args.run_name,
            "source_verified_by_rust_paths": True,
            "snapshot": snapshot_path.name,
            "filtered_copy": filtered.name,
            "status": "snapshot_created",
            "retained_for_owner_review": True,
        }, manifest_out)
    snapshot = sqlite3.connect(snapshot_path.resolve().as_uri() + "?mode=ro", uri=True)
    snapshot.execute("PRAGMA query_only=ON")
    snapshot.execute("BEGIN")
    eligible = []
    excluded = {}
    for session_id in ids:
        row = snapshot.execute("SELECT session_type, privacy_tier, provider_name, incarnation, name, external_key FROM sessions WHERE id=?", (session_id,)).fetchone()
        reason = None
        if row is None:
            reason = "missing"
        elif row[0] != "user" or row[1] != "public":
            reason = "nonpublic_or_nonuser"
        elif row[4].startswith("app:") or (row[5] or "").startswith("app:"):
            reason = "app_session"
        elif row[2] not in providers:
            reason = "unapproved_provider"
        elif snapshot.execute("SELECT 1 FROM classification_audit WHERE session_id=? AND (session_incarnation=? OR session_incarnation IS NULL) AND (from_classification='private' OR to_classification='private') LIMIT 1", (session_id, row[3])).fetchone():
            reason = "private_history"
        if reason:
            excluded[reason] = excluded.get(reason, 0) + 1
        else:
            eligible.append(session_id)
    if len(eligible) != 200:
        with run_manifest.open("w") as manifest_out:
            json.dump({"run_name": args.run_name, "snapshot": snapshot_path.name,
                       "status": "insufficient_eligible_sessions", "sessions_included": len(eligible),
                       "exclusions": excluded, "retained_for_owner_review": True}, manifest_out)
        snapshot.execute("COMMIT")
        snapshot.close()
        print(json.dumps({"sessions_requested": 200, "sessions_included": len(eligible), "exclusions": excluded, "gate": "insufficient_eligible_sessions"}))
        raise SystemExit(2)
    fd = os.open(filtered, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
    os.close(fd)
    copy = sqlite3.connect(filtered)
    copy.executescript("CREATE TABLE sessions (id TEXT PRIMARY KEY); CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, content_json TEXT NOT NULL);")
    copy.executemany("INSERT INTO sessions(id) VALUES (?)", ((sid,) for sid in eligible))
    sessions_without_person_messages = 0
    for session_id in eligible:
        copied = 0
        for row_id, content, metadata_raw in snapshot.execute("SELECT id, content_json, metadata_json FROM messages WHERE session_id=? AND role='user' ORDER BY id", (session_id,)):
            if not person_written_user_message("user", metadata_raw):
                continue
            copy.execute("INSERT INTO messages(id, session_id, content_json) VALUES (?, ?, ?)", (row_id, session_id, content))
            copied += 1
        if copied == 0:
            sessions_without_person_messages += 1
    copy.commit()
    snapshot.execute("COMMIT")
    copy.close()
    snapshot.close()
    if sessions_without_person_messages:
        with run_manifest.open("w") as manifest_out:
            json.dump({"run_name": args.run_name, "snapshot": snapshot_path.name,
                       "filtered_copy": filtered.name, "status": "insufficient_person_messages",
                       "sessions_included": 200,
                       "sessions_without_person_messages": sessions_without_person_messages,
                       "retained_for_owner_review": True}, manifest_out)
        print(json.dumps({"sessions_requested": 200, "sessions_included": 200,
                          "sessions_without_person_messages": sessions_without_person_messages,
                          "gate": "insufficient_person_messages"}))
        raise SystemExit(2)
    with run_manifest.open("w") as manifest_out:
        json.dump({
            "run_name": args.run_name,
            "source_verified_by_rust_paths": True,
            "snapshot": snapshot_path.name,
            "filtered_copy": filtered.name,
            "sessions_included": 200,
            "owner_attested_patient_data_excluded": True,
            "owner_attested_private_models_excluded": True,
            "status": "filtered_copy_ready",
            "retained_for_owner_review": True,
        }, manifest_out)
    env = os.environ.copy()
    env["PR24_FILTERED_DB"] = str(filtered)
    env["PR24_CANDIDATES_FILE"] = str(output)
    run_precision_test("m21_candidate_export", env)
    print(json.dumps({"sessions_requested": 200, "sessions_included": 200, "exclusions": excluded, "gate": "await_owner_labels"}))



def snapshot_self_test():
    with tempfile.TemporaryDirectory() as root:
        db_path = Path(root) / "synthetic.db"
        writer = sqlite3.connect(db_path)
        writer.execute("PRAGMA journal_mode=WAL")
        writer.executescript("CREATE TABLE sessions (id TEXT, privacy_tier TEXT); CREATE TABLE messages (session_id TEXT, content_json TEXT); INSERT INTO sessions VALUES ('s', 'public'); INSERT INTO messages VALUES ('s', 'before');")
        writer.commit()
        reader = sqlite3.connect(db_path)
        reader.execute("PRAGMA query_only=ON")
        reader.execute("BEGIN")
        assert reader.execute("SELECT privacy_tier FROM sessions WHERE id='s'").fetchone() == ("public",)
        writer.execute("UPDATE sessions SET privacy_tier='private' WHERE id='s'")
        writer.execute("INSERT INTO messages VALUES ('s', 'after')")
        writer.commit()
        assert reader.execute("SELECT content_json FROM messages WHERE session_id='s'").fetchall() == [("before",)]
        reader.execute("COMMIT")
        reader.close()
        writer.close()
    print("snapshot_consistency=pass")


def provenance_self_test():
    db = sqlite3.connect(":memory:")
    db.executescript("""
        CREATE TABLE sessions (id TEXT PRIMARY KEY, session_type TEXT NOT NULL, name TEXT NOT NULL, external_key TEXT);
        CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, metadata_json TEXT);
        INSERT INTO sessions VALUES ('ordinary', 'user', 'chat', NULL);
        INSERT INTO sessions VALUES ('child', 'sub_agent', 'child', NULL);
        INSERT INTO sessions VALUES ('scheduled', 'scheduled', 'scheduled', NULL);
        INSERT INTO sessions VALUES ('app', 'user', 'app:demo', 'app:demo:browser');
    """)
    rows = [
        ("ordinary", "user", '{"userVisible":true,"agentVisible":true}'),
        ("ordinary", "user", '{"userVisible":true,"provenance":{"kind":"user_direct"}}'),
        ("ordinary", "user", '{"userVisible":true,"provenance":{"kind":"agent_injection"}}'),
        ("ordinary", "user", '{"userVisible":true,"provenance":{"kind":"spawn_context"}}'),
        ("ordinary", "user", '{"userVisible":false}'),
        ("child", "user", '{"userVisible":true,"provenance":{"kind":"user_direct"}}'),
        ("scheduled", "user", '{"userVisible":true}'),
        ("app", "user", '{"userVisible":true}'),
        ("ordinary", "tool", '{"userVisible":true}'),
    ]
    db.executemany("INSERT INTO messages(session_id, role, metadata_json) VALUES (?, ?, ?)", rows)
    selected = []
    for session_type, name, external_key, role, metadata in db.execute("""
        SELECT s.session_type, s.name, s.external_key, m.role, m.metadata_json
        FROM sessions s JOIN messages m ON m.session_id=s.id ORDER BY m.id
    """):
        selected.append(role == "user" and not name.startswith("app:")
                        and not (external_key or "").startswith("app:")
                        and person_written_user_message(session_type, metadata))
    assert selected == [True, True, False, False, False, False, False, False, False]
    db.close()
    print("provenance_selection=pass")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test-snapshot"]:
        snapshot_self_test()
    elif sys.argv[1:] == ["--self-test-provenance"]:
        provenance_self_test()
    else:
        main()
