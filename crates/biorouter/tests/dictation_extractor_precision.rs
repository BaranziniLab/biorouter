//! Internal M-21 export helper. Run only on the filtered 200-session SQLite copy.
//!
//! The owner's run root is read from `PR24_PRECISION_ROOT`, never written here, because this file
//! ships in a public repository. Each owner-only test returns early, and says so, when that
//! variable or its own inputs are not set.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::dictation::extractor::candidates;
use biorouter::security::dictionary_store::store::fold_key;
use sqlx::{Connection, Row};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Names the folder that holds the owner's M-21 run. Candidates are written only under
/// `<root>/pr24-precision`.
const ROOT_VAR: &str = "PR24_PRECISION_ROOT";

fn same_canonical_file(expected: &std::path::Path, supplied: &std::path::Path) -> bool {
    match (expected.canonicalize(), supplied.canonicalize()) {
        (Ok(expected), Ok(supplied)) => expected == supplied,
        _ => false,
    }
}

/// The value of each named variable, or `None` after printing which one is unset.
fn owner_inputs<const N: usize>(names: [&str; N]) -> Option<[OsString; N]> {
    let mut values = Vec::with_capacity(N);
    for name in names {
        match std::env::var_os(name) {
            Some(value) if !value.is_empty() => values.push(value),
            _ => {
                println!("skipped: {name} is not set");
                return None;
            }
        }
    }
    values.try_into().ok()
}

#[test]
fn source_guard_rejects_another_database_in_a_sandbox() {
    let root = tempfile::tempdir().unwrap();
    let expected = root.path().join("sessions.db");
    let other = root.path().join("other.db");
    std::fs::write(&expected, []).unwrap();
    std::fs::write(&other, []).unwrap();
    assert!(same_canonical_file(&expected, &expected));
    assert!(!same_canonical_file(&expected, &other));
}

#[test]
#[ignore = "owner-only source path guard before opening the SQLite store"]
fn m21_source_path_matches_resolver() {
    let Some([_root, supplied]) = owner_inputs([ROOT_VAR, "PR24_SOURCE_DB"]) else {
        return;
    };
    let supplied = PathBuf::from(supplied);
    let expected = biorouter::config::paths::Paths::data_dir()
        .join(biorouter::session::session_manager::SESSIONS_FOLDER)
        .join(biorouter::session::session_manager::DB_NAME);
    assert!(
        same_canonical_file(&expected, &supplied),
        "source path mismatch"
    );
}

#[tokio::test]
#[ignore = "owner-only M-21 run over a filtered private SQLite copy"]
async fn m21_candidate_export() {
    let Some([root, db_path, output]) =
        owner_inputs([ROOT_VAR, "PR24_FILTERED_DB", "PR24_CANDIDATES_FILE"])
    else {
        return;
    };
    let output_root = PathBuf::from(root).join("pr24-precision");
    let db_path = PathBuf::from(db_path);
    let output = PathBuf::from(output);
    assert_eq!(
        output.parent().unwrap().canonicalize().unwrap(),
        output_root.canonicalize().unwrap()
    );
    let url = format!("sqlite://{}?mode=ro", db_path.display());
    let mut db = sqlx::SqliteConnection::connect(&url).await.unwrap();
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&mut db)
        .await
        .unwrap();
    assert_eq!(
        sessions, 200,
        "filtered copy must have 200 eligible sessions"
    );
    let rows = sqlx::query("SELECT session_id, content_json FROM messages ORDER BY session_id, id")
        .fetch_all(&mut db)
        .await
        .unwrap();
    let mut found: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
    for row in rows {
        let session_id: String = row.get("session_id");
        let raw: String = row.get("content_json");
        let content: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let Some(parts) = content.as_array() else {
            continue;
        };
        if parts
            .iter()
            .any(|part| part.get("type").and_then(|v| v.as_str()) != Some("text"))
        {
            continue;
        }
        let text = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        for term in candidates(&text) {
            let entry = found
                .entry(fold_key(&term))
                .or_insert_with(|| (term, BTreeSet::new()));
            entry.1.insert(session_id.clone());
        }
    }
    let selected = found
        .values()
        .filter(|(_, ids)| ids.len() >= 2)
        .collect::<Vec<_>>();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options.open(&output).unwrap();
    writeln!(out, "candidate\tsessions_seen\towner_label").unwrap();
    for (term, ids) in &selected {
        writeln!(out, "{term}\t{}\t", ids.len()).unwrap();
    }
    out.sync_all().unwrap();
    println!(
        "sessions_included={sessions} candidates_seen_twice={}",
        selected.len()
    );
}
