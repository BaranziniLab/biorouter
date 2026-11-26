use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};

use super::*;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_795_707_552 + seconds, 0).unwrap()
}

fn entry(seconds: i64, text: &str, tier: EntryTier) -> HistoryEntry {
    HistoryEntry {
        id: new_entry_id(),
        created_at: at(seconds),
        text: text.to_string(),
        tier,
        institutions: None,
        surface: Surface::Composer,
        duration_ms: 14_200,
        model: "whisper-large-v3-turbo-q5_0".to_string(),
        private_terms: false,
        session_id: Some("20261126_3".to_string()),
        session_incarnation: Some(8_812_734_012),
        truncated: None,
    }
}

fn store() -> (tempfile::TempDir, HistoryStore) {
    let root = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(root.path().join("dictation").join("history"));
    (root, store)
}

fn everything(store: &HistoryStore) -> Vec<HistoryEntry> {
    store
        .search(None, TimeWindow::all(), None, usize::MAX, &|_| true)
        .unwrap()
        .entries
}

fn texts(entries: &[HistoryEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.text.as_str()).collect()
}

fn admit_all(_: &HistoryEntry) -> bool {
    true
}

#[test]
fn an_entry_round_trips_through_the_file() {
    let (_root, store) = store();
    let mut private = entry(
        0,
        "Query SPOKE for drugs that target EPAS1.",
        EntryTier::Private,
    );
    private.institutions = Some(vec!["ucsf".to_string()]);
    private.private_terms = true;
    let mut public = entry(1, "A public note.", EntryTier::Public);
    public.surface = Surface::OtherApp;
    public.session_id = None;
    public.session_incarnation = None;
    store.append(private.clone()).unwrap();
    store.append(public.clone()).unwrap();
    assert_eq!(everything(&store), vec![public.clone(), private.clone()]);
    // Another handle reads the same file.
    let other = HistoryStore::open(store.dir());
    assert_eq!(other.get(&private.id).unwrap(), Some(private));
    assert_eq!(other.get(&new_entry_id()).unwrap(), None);
    let text = fs::read_to_string(store.history_path()).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("{\"format\":\"biorouter.dictation.history\",\"version\":1}")
    );
    assert_eq!(lines.count(), 2);
}

#[test]
fn a_fraction_of_a_second_is_dropped() {
    let (_root, store) = store();
    let mut e = entry(0, "hello", EntryTier::Private);
    e.created_at = DateTime::from_timestamp(1_795_707_552, 123_456_789).unwrap();
    store.append(e.clone()).unwrap();
    assert_eq!(everything(&store)[0].created_at, at(0));
}

#[test]
fn a_long_text_is_cut_on_a_character_boundary_and_marked() {
    let (_root, store) = store();
    let text = "é😀x".repeat(MAX_TEXT_BYTES / 3);
    assert!(text.len() > MAX_TEXT_BYTES);
    store.append(entry(0, &text, EntryTier::Private)).unwrap();
    let kept = everything(&store).remove(0);
    assert!(kept.text.len() <= MAX_TEXT_BYTES);
    assert!(text.starts_with(&kept.text), "cut on a character boundary");
    assert_eq!(kept.truncated, Some(true));
    store.append(entry(1, "short", EntryTier::Private)).unwrap();
    assert_eq!(everything(&store)[0].truncated, None);
}

#[test]
fn an_entry_that_does_not_validate_or_reuses_an_id_is_refused() {
    let (_root, store) = store();
    let first = entry(0, "first", EntryTier::Private);
    store.append(first.clone()).unwrap();
    let mut again = entry(1, "again", EntryTier::Private);
    again.id = first.id.clone();
    assert!(matches!(
        store.append(again),
        Err(HistoryError::Invalid(EntryProblem::Id))
    ));
    let mut public_with_ids = entry(2, "x", EntryTier::Public);
    public_with_ids.institutions = Some(vec!["ucsf".into()]);
    assert!(matches!(
        store.append(public_with_ids),
        Err(HistoryError::Invalid(EntryProblem::Institutions))
    ));
    assert!(matches!(
        store.append(entry(3, "   ", EntryTier::Private)),
        Err(HistoryError::Invalid(EntryProblem::EmptyText))
    ));
    assert_eq!(everything(&store).len(), 1);
}

/// Every way a file can be one this version does not read: each refuses every write and
/// every read, and the file stays byte identical.
#[test]
fn a_file_this_version_cannot_read_fails_closed_and_is_never_rewritten() {
    let good = entry(0, "kept", EntryTier::Public);
    let good_line = serde_json::to_string(&good).unwrap();
    let mut unknown_key: serde_json::Value = serde_json::from_str(&good_line).unwrap();
    unknown_key["language"] = serde_json::json!("en");
    let header = "{\"format\":\"biorouter.dictation.history\",\"version\":1}";
    let cases = [
        (
            "unknown key",
            format!("{header}\n{good_line}\n{unknown_key}\n"),
        ),
        (
            "unknown header key",
            format!(
                "{{\"format\":\"biorouter.dictation.history\",\"version\":1,\"x\":1}}\n{good_line}\n"
            ),
        ),
        (
            "unknown format",
            format!("{{\"format\":\"something.else\",\"version\":1}}\n{good_line}\n"),
        ),
        (
            "newer version",
            format!("{{\"format\":\"biorouter.dictation.history\",\"version\":2}}\n{good_line}\n"),
        ),
        (
            "bad middle line",
            format!("{header}\n{good_line}\nnot json\n{good_line}\n"),
        ),
        ("blank middle line", format!("{header}\n\n{good_line}\n")),
        (
            "duplicate id",
            format!("{header}\n{good_line}\n{good_line}\n"),
        ),
        (
            "entry that does not validate",
            format!("{header}\n{}\n", good_line.replace("\"kept\"", "\"\"")),
        ),
    ];
    for (label, text) in cases {
        let (_root, store) = store();
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(store.history_path(), &text).unwrap();
        let bytes = fs::read(store.history_path()).unwrap();
        let path = store.history_path();
        let unreadable = |r: &Result<_, HistoryError>| matches!(r, Err(HistoryError::Unreadable { path: p }) if *p == path);
        assert!(
            unreadable(
                &store
                    .append(entry(9, "new", EntryTier::Private))
                    .map(|_| ())
            ),
            "{label}: append"
        );
        assert!(
            unreadable(
                &store
                    .search(None, TimeWindow::all(), None, 10, &admit_all)
                    .map(|_| ())
            ),
            "{label}: search"
        );
        assert!(unreadable(&store.get(&good.id).map(|_| ())), "{label}: get");
        assert!(
            unreadable(
                &store
                    .delete(std::slice::from_ref(&good.id), &admit_all)
                    .map(|_| ())
            ),
            "{label}: delete"
        );
        assert!(
            unreadable(&store.clear_through(at(100), &admit_all).map(|_| ())),
            "{label}: clear"
        );
        assert!(
            unreadable(
                &store
                    .set_tier(&good.id, EntryTier::Private, &admit_all)
                    .map(|_| ())
            ),
            "{label}: tier"
        );
        assert_eq!(
            fs::read(store.history_path()).unwrap(),
            bytes,
            "{label}: byte identical"
        );
        assert!(
            !store.dir().join("history.jsonl.tmp").exists(),
            "{label}: no temporary left"
        );
        let refusal = store
            .append(entry(9, "new", EntryTier::Private))
            .unwrap_err();
        assert_eq!(
            refusal.to_string(),
            format!(
                "The dictation history file at {} could not be read, so nothing was changed. Fix or remove it, then try again.",
                path.display()
            ),
            "{label}"
        );
        // The settings stay writable while the history cannot be read.
        let off = store
            .set_settings(SettingsChange {
                keep_history: Some(false),
                public_by_default: None,
            })
            .unwrap();
        assert!(!off.keep_history, "{label}");
        assert_eq!(fs::read(store.history_path()).unwrap(), bytes, "{label}");
    }
}

/// An append that died mid line leaves a torn tail: readers leave it out, and the next
/// append cuts it back to the last newline before it writes.
#[test]
fn a_torn_last_line_is_ignored_then_repaired() {
    let (_root, store) = store();
    store.append(entry(0, "first", EntryTier::Public)).unwrap();
    let path = store.history_path();
    let mut bytes = fs::read(&path).unwrap();
    let committed = bytes.len();
    let torn = serde_json::to_string(&entry(1, "torn", EntryTier::Public)).unwrap();
    bytes.extend_from_slice(&torn.as_bytes()[..torn.len() / 2]);
    fs::write(&path, &bytes).unwrap();
    assert_eq!(texts(&everything(&store)), ["first"]);
    assert_eq!(
        fs::read(&path).unwrap().len(),
        bytes.len(),
        "a reader writes nothing"
    );
    store.append(entry(2, "third", EntryTier::Public)).unwrap();
    let repaired = fs::read(&path).unwrap();
    assert!(repaired.ends_with(b"\n"));
    assert!(!String::from_utf8_lossy(&repaired).contains("torn"));
    assert_eq!(&repaired[..committed], &bytes[..committed]);
    assert_eq!(texts(&everything(&store)), ["third", "first"]);
    // A file holding only a torn header is an empty history, and is repaired the same way.
    fs::write(&path, b"{\"format\":\"biorouter.dict").unwrap();
    assert!(everything(&store).is_empty());
    store.append(entry(3, "after", EntryTier::Public)).unwrap();
    assert_eq!(texts(&everything(&store)), ["after"]);
}

/// Child half of `two_processes_append_under_the_lock`: does nothing unless the parent
/// named a folder.
#[test]
#[ignore = "run as a child process by two_processes_append_under_the_lock"]
fn appending_child() {
    let Some(dir) = std::env::var_os("BR_HISTORY_APPEND_CHILD_DIR") else {
        return;
    };
    let store = HistoryStore::open(Path::new(&dir));
    for i in 0..40 {
        store
            .append(entry(i, &format!("child {i}"), EntryTier::Private))
            .unwrap();
    }
}

/// Two processes appending at once lose nothing: the lock orders them, and every line is
/// whole.
#[test]
fn two_processes_append_under_the_lock() {
    let (_root, store) = store();
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = (0..2)
        .map(|_| {
            std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "history::tests::appending_child",
                    "--ignored",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env("BR_HISTORY_APPEND_CHILD_DIR", store.dir())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let entries = everything(&store);
    assert_eq!(entries.len(), 80);
    let ids: std::collections::HashSet<_> = entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids.len(), 80);
}

/// Passing the entry cap drops the oldest in the same write, to 90 percent, and the next
/// append is a plain append again.
#[test]
fn the_entry_cap_prunes_to_ninety_percent_in_one_write() {
    let (_root, store) = store();
    let full: Vec<HistoryEntry> = (0..MAX_ENTRIES as i64)
        .map(|i| entry(i, &format!("note {i}"), EntryTier::Public))
        .collect();
    fs::create_dir_all(store.dir()).unwrap();
    fs::write(store.history_path(), ops::file_bytes(&full)).unwrap();
    store
        .append(entry(9_000, "newest", EntryTier::Public))
        .unwrap();
    let kept = everything(&store);
    assert_eq!(kept.len(), MAX_ENTRIES * 9 / 10);
    assert_eq!(kept[0].text, "newest");
    assert_eq!(
        kept.last().unwrap().text,
        format!("note {}", MAX_ENTRIES - 4_500 + 1)
    );
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(store.history_path()).unwrap().ino()
    };
    store
        .append(entry(9_001, "after", EntryTier::Public))
        .unwrap();
    assert_eq!(everything(&store).len(), MAX_ENTRIES * 9 / 10 + 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(store.history_path()).unwrap().ino(),
            inode,
            "below the cap an append rewrites nothing"
        );
    }
}

#[test]
fn the_byte_cap_prunes_to_ninety_percent_of_it() {
    let (_root, store) = store();
    let text = "x".repeat(30_000);
    let mut entries = Vec::new();
    let mut size = 0u64;
    let mut i = 0;
    while size < MAX_FILE_BYTES - 40_000 {
        let e = entry(i, &text, EntryTier::Private);
        size += serde_json::to_string(&e).unwrap().len() as u64 + 1;
        entries.push(e);
        i += 1;
    }
    fs::create_dir_all(store.dir()).unwrap();
    fs::write(store.history_path(), ops::file_bytes(&entries)).unwrap();
    store.append(entry(i, &text, EntryTier::Private)).unwrap();
    store
        .append(entry(i + 1, &text, EntryTier::Private))
        .unwrap();
    let len = fs::metadata(store.history_path()).unwrap().len();
    assert!(len <= MAX_FILE_BYTES * 9 / 10 + 40_000, "{len}");
    assert!(len <= MAX_FILE_BYTES);
    let kept = everything(&store);
    assert_eq!(kept[0].created_at, at(i + 1), "the newest stays");
    assert!(kept.len() < entries.len());
}

#[cfg(unix)]
#[test]
fn the_folder_and_its_files_are_private_to_the_account() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, store) = store();
    store.append(entry(0, "x", EntryTier::Private)).unwrap();
    store
        .set_settings(SettingsChange {
            keep_history: Some(false),
            public_by_default: None,
        })
        .unwrap();
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(store.dir()), 0o700);
    assert_eq!(mode(&store.history_path()), 0o600);
    assert_eq!(mode(&store.settings_path()), 0o600);
    assert_eq!(mode(&store.dir().join(LOCK_FILE)), 0o600);
    // A rewrite keeps them so.
    let id = everything(&store)[0].id.clone();
    store.set_tier(&id, EntryTier::Public, &admit_all).unwrap();
    assert_eq!(mode(&store.history_path()), 0o600);
}

/// The cache follows the file's identity: a file renamed into place with the same length
/// and the same modification time is parsed again.
#[test]
fn the_cache_reparses_when_the_file_is_replaced() {
    let (_root, store) = store();
    store.append(entry(0, "AAAA", EntryTier::Public)).unwrap();
    let parses = store.parses();
    everything(&store);
    everything(&store);
    assert_eq!(
        store.parses(),
        parses,
        "an unchanged file is not parsed again"
    );

    let path = store.history_path();
    let before = fs::metadata(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap().replace("AAAA", "BBBB");
    let replacement = store.dir().join("replacement");
    fs::write(&replacement, &text).unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_modified(before.modified().unwrap())
        .unwrap();
    fs::rename(&replacement, &path).unwrap();
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        (after.len(), after.modified().unwrap()),
        (before.len(), before.modified().unwrap())
    );
    assert_eq!(texts(&everything(&store)), ["BBBB"]);
    assert_eq!(store.parses(), parses + 1);
}

/// The person's own write leaves the cache holding what it wrote.
#[test]
fn a_write_through_this_handle_parses_nothing_after_it() {
    let (_root, store) = store();
    store.append(entry(0, "one", EntryTier::Public)).unwrap();
    store.append(entry(1, "two", EntryTier::Public)).unwrap();
    let parses = store.parses();
    assert_eq!(texts(&everything(&store)), ["two", "one"]);
    assert_eq!(store.parses(), parses);
    let other = HistoryStore::open(store.dir());
    other.append(entry(2, "three", EntryTier::Public)).unwrap();
    assert_eq!(texts(&everything(&store)), ["three", "two", "one"]);
    assert_eq!(
        store.parses(),
        parses + 1,
        "another handle's append is seen"
    );
}

/// The quota rule at the store: private entries newer than the public ones never use up
/// a public caller's page, and the total counts only what it may see.
#[test]
fn a_public_search_fills_its_quota_when_private_entries_are_newer() {
    let (_root, store) = store();
    for i in 0..10 {
        store
            .append(entry(
                i,
                &format!("public EPAS1 note {i}"),
                EntryTier::Public,
            ))
            .unwrap();
    }
    for i in 10..70 {
        store
            .append(entry(i, "PRIVATE SENTINEL EPAS1", EntryTier::Private))
            .unwrap();
    }
    let public = |e: &HistoryEntry| e.tier == EntryTier::Public;
    let page = store
        .search(Some("epas 1"), TimeWindow::all(), None, 10, &public)
        .unwrap();
    assert_eq!(page.entries.len(), 10);
    assert_eq!(page.total, 10);
    assert!(page.next_cursor.is_none());
    assert!(page.entries.iter().all(|e| !e.text.contains("SENTINEL")));
    let both = store
        .search(Some("EPAS1"), TimeWindow::all(), None, 10, &admit_all)
        .unwrap();
    assert_eq!(both.total, 70);
    assert!(both.entries.iter().all(|e| e.text.contains("SENTINEL")));
    assert!(both.next_cursor.is_some());
}

#[test]
fn delete_clear_and_tier_touch_only_what_admits_allows() {
    let (_root, store) = store();
    let mut private = entry(0, "private", EntryTier::Private);
    private.institutions = Some(vec!["ucsf".into()]);
    let public = entry(1, "public", EntryTier::Public);
    let later = entry(5, "later", EntryTier::Public);
    for e in [&private, &public, &later] {
        store.append(e.clone()).unwrap();
    }
    let public_only = |e: &HistoryEntry| e.tier == EntryTier::Public;

    // A refused entry answers as a missing one.
    assert_eq!(
        store
            .set_tier(&private.id, EntryTier::Public, &public_only)
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .delete(std::slice::from_ref(&private.id), &public_only)
            .unwrap(),
        0
    );
    // A tier change that changes nothing writes nothing.
    let bytes = fs::read(store.history_path()).unwrap();
    let same = store
        .set_tier(&public.id, EntryTier::Public, &admit_all)
        .unwrap()
        .unwrap();
    assert_eq!(same, public);
    assert_eq!(fs::read(store.history_path()).unwrap(), bytes);
    // Made public, an entry names no institution; made private again, still none.
    let made_public = store
        .set_tier(&private.id, EntryTier::Public, &admit_all)
        .unwrap()
        .unwrap();
    assert_eq!(made_public.tier, EntryTier::Public);
    assert_eq!(made_public.institutions, None);
    let made_private = store
        .set_tier(&private.id, EntryTier::Private, &admit_all)
        .unwrap()
        .unwrap();
    assert_eq!(made_private.institutions, None);

    // Clear stops at `through`, so a newer dictation survives.
    assert_eq!(store.clear_through(at(1), &public_only).unwrap(), 1);
    assert_eq!(texts(&everything(&store)), ["later", "private"]);
    assert_eq!(
        store
            .delete(&[later.id.clone(), new_entry_id()], &admit_all)
            .unwrap(),
        1
    );
    assert_eq!(store.clear_through(at(100), &admit_all).unwrap(), 1);
    assert!(everything(&store).is_empty());
    assert_eq!(
        fs::read_to_string(store.history_path()).unwrap(),
        "{\"format\":\"biorouter.dictation.history\",\"version\":1}\n",
        "a cleared history is its header"
    );
}

#[test]
fn settings_default_on_and_fail_closed_to_off() {
    let (_root, store) = store();
    assert_eq!(store.settings(), HistorySettings::default());
    assert!(store.settings().keep_history);
    assert!(!store.settings().public_by_default);
    // A change that changes nothing writes nothing.
    store.set_settings(SettingsChange::default()).unwrap();
    assert!(!store.settings_path().exists());
    let on = store
        .set_settings(SettingsChange {
            keep_history: None,
            public_by_default: Some(true),
        })
        .unwrap();
    assert!(on.keep_history && on.public_by_default);
    assert_eq!(store.settings(), on);

    let text = fs::read_to_string(store.settings_path()).unwrap();
    for bad in [
        text.replace("\"version\": 1", "\"version\": 2"),
        text.replace(
            "\"keep_history\"",
            "\"retention_days\": 30,\n  \"keep_history\"",
        ),
        "{".to_string(),
    ] {
        fs::write(store.settings_path(), &bad).unwrap();
        assert_eq!(store.settings(), HistorySettings::UNREADABLE, "{bad}");
        assert!(matches!(
            store.set_settings(SettingsChange {
                keep_history: Some(true),
                public_by_default: None,
            }),
            Err(HistoryError::Unreadable { .. })
        ));
        assert_eq!(fs::read_to_string(store.settings_path()).unwrap(), bad);
    }
}

/// The lock is the dictionary's rule: an append whose lock never comes free drops its
/// entry rather than wait.
#[test]
fn an_append_that_cannot_take_the_lock_is_dropped() {
    let (_root, store) = store();
    store.append(entry(0, "first", EntryTier::Private)).unwrap();
    let held = disk::HistoryLock::acquire(store.dir()).unwrap();
    let other = HistoryStore::open(store.dir());
    let started = std::time::Instant::now();
    assert!(matches!(
        other.append(entry(1, "dropped", EntryTier::Private)),
        Err(HistoryError::Busy)
    ));
    assert!(started.elapsed() >= APPEND_BUDGET);
    drop(held);
    assert_eq!(texts(&everything(&store)), ["first"]);
}
