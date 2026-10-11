//! The move, the resolution and the rewrite, against temporary base
//! directories. Nothing here reads the environment or the real folders, so the
//! whole suite runs on every OS, `windows-latest` included.

use super::rewrite::{Format, RootRewrite};
use super::*;
use std::sync::{Arc, Barrier};

const NO_DELAYS: &[Duration] = &[];

fn refuse(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "a file in the folder is held open",
    ))
}

fn ops(rename_fails: bool) -> MoveOps<'static> {
    MoveOps {
        rename: if rename_fails { &refuse } else { &std_rename },
        retry_delays: NO_DELAYS,
    }
}

/// `<base>/<token>/<rest>`, for the tokens `resolution-cases.json` defines.
fn token_path(base: &Path, spec: &str) -> PathBuf {
    let spec = spec.trim_end_matches('/');
    let (head, rest) = spec.split_once('/').unwrap_or((spec, ""));
    let root = match head {
        "current" => base.join(APP_AUTHOR).join(APP_NAME),
        "legacy" => base.join(LEGACY_APP_AUTHOR).join(APP_NAME),
        "currentParent" => base.join(APP_AUTHOR),
        "legacyParent" => base.join(LEGACY_APP_AUTHOR),
        "lookalike" => base.join(format!("{LEGACY_APP_AUTHOR}2")).join(APP_NAME),
        other => panic!("unknown path token {other:?}"),
    };
    rest.split('/')
        .filter(|segment| !segment.is_empty())
        .fold(root, |path, segment| {
            path.join(match segment {
                "marker" => MOVE_MARKER,
                "lock" => MOVE_LOCK,
                other => other,
            })
        })
}

fn materialize(base: &Path, spec: &str) {
    let path = token_path(base, spec);
    if spec.ends_with('/') {
        fs::create_dir_all(&path).unwrap();
    } else {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, spec).unwrap();
    }
}

/// Directories whose permissions a case removes, restored on drop so a failed
/// assertion cannot leave an undeletable temporary directory behind.
struct Unreadable(Vec<PathBuf>);

impl Unreadable {
    /// `None` when the case cannot be set up here: not unix, or permissions
    /// that are not enforced (running as root).
    fn make(base: &Path, specs: &[String]) -> Option<Self> {
        if specs.is_empty() {
            return Some(Self(Vec::new()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dirs: Vec<PathBuf> = specs.iter().map(|spec| token_path(base, spec)).collect();
            for dir in &dirs {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
            }
            let guard = Self(dirs);
            if guard.0.iter().any(|dir| fs::read_dir(dir).is_ok()) {
                return None;
            }
            Some(guard)
        }
        #[cfg(not(unix))]
        {
            let _ = base;
            None
        }
    }
}

impl Drop for Unreadable {
    fn drop(&mut self) {
        #[cfg(unix)]
        for dir in &self.0 {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolutionCase {
    name: String,
    before: Vec<String>,
    #[serde(default)]
    unreadable: Vec<String>,
    resolves: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveCase {
    name: String,
    before: Vec<String>,
    #[serde(default)]
    unreadable: Vec<String>,
    #[serde(default)]
    rename_fails: bool,
    runs: Vec<String>,
    resolves: String,
    exists: Vec<String>,
    absent: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Table {
    resolution: Vec<ResolutionCase>,
    moves: Vec<MoveCase>,
}

fn table() -> Table {
    serde_json::from_str(include_str!("resolution-cases.json")).unwrap()
}

/// The shared resolution states, also run by
/// `ui/desktop/src/utils/appFolder.test.ts`, so the desktop main process and
/// every Rust process resolve each state to the same folder.
#[test]
fn every_resolution_case_in_the_shared_table_holds() {
    let cases = table().resolution;
    assert!(cases.len() >= 8, "the table lost resolution cases");
    for case in cases {
        let base = tempfile::tempdir().unwrap();
        for spec in &case.before {
            materialize(base.path(), spec);
        }
        let Some(_unreadable) = Unreadable::make(base.path(), &case.unreadable) else {
            eprintln!("skipped here: {}", case.name);
            continue;
        };
        let root = AppRoot::under(base.path());
        assert_eq!(
            root.resolve(),
            token_path(base.path(), &case.resolves),
            "{}: resolved the wrong folder",
            case.name
        );
    }
}

/// The daemon's moves, state by state.
#[test]
fn every_move_case_in_the_shared_table_holds() {
    let cases = table().moves;
    assert!(cases.len() >= 10, "the table lost move cases");
    for case in cases {
        let base = tempfile::tempdir().unwrap();
        for spec in &case.before {
            materialize(base.path(), spec);
        }
        let Some(unreadable) = Unreadable::make(base.path(), &case.unreadable) else {
            eprintln!("skipped here: {}", case.name);
            continue;
        };
        let root = AppRoot::under(base.path());
        for (run, expected) in case.runs.iter().enumerate() {
            let outcome = move_root(&root, true, &ops(case.rename_fails));
            assert_eq!(
                outcome.token(),
                expected,
                "{}: run {} ended {outcome:?}",
                case.name,
                run + 1
            );
        }
        assert_eq!(
            root.resolve(),
            token_path(base.path(), &case.resolves),
            "{}: resolved the wrong folder",
            case.name
        );
        drop(unreadable);
        for spec in &case.exists {
            assert!(
                token_path(base.path(), spec).exists(),
                "{}: {spec} is missing",
                case.name
            );
        }
        for spec in &case.absent {
            assert!(
                !token_path(base.path(), spec).exists(),
                "{}: {spec} should not exist",
                case.name
            );
        }
    }
}

/// The move never reads or writes a byte of content: a file's bytes are the
/// same after the rename, and nothing was copied.
#[test]
fn a_moved_file_keeps_its_bytes() {
    let base = tempfile::tempdir().unwrap();
    let root = AppRoot::under(base.path());
    fs::create_dir_all(root.legacy.join("data")).unwrap();
    let bytes: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    fs::write(root.legacy.join("data").join("blob.bin"), &bytes).unwrap();

    assert_eq!(move_root(&root, true, &ops(false)).token(), "moved");
    assert_eq!(
        fs::read(root.current.join("data").join("blob.bin")).unwrap(),
        bytes
    );
}

/// Two daemons racing: one takes the lock and renames, the other waits for the
/// lock and then sees the current folder (or gives up waiting), and the tree
/// arrives whole either way. Repeated, because the interleaving differs from
/// run to run.
///
/// With retries, as in production: on Windows the other mover's marker handle,
/// open for an instant inside the folder, is enough to refuse a rename once.
#[test]
fn two_movers_at_once_end_with_one_tree() {
    const BRIEF: &[Duration] = &[Duration::from_millis(5); 5];
    for _ in 0..25 {
        let base = tempfile::tempdir().unwrap();
        let root = AppRoot::under(base.path());
        fs::create_dir_all(root.legacy.join("config")).unwrap();
        fs::write(root.legacy.join("config").join("config.yaml"), "a: 1\n").unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let racers: Vec<_> = (0..2)
            .map(|_| {
                let root = root.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let ops = MoveOps {
                        rename: &std_rename,
                        retry_delays: BRIEF,
                    };
                    move_root(&root, true, &ops).token()
                })
            })
            .collect();
        let mut tokens: Vec<&str> = racers.into_iter().map(|r| r.join().unwrap()).collect();
        tokens.sort();
        // The loser either waited for the lock and found the move done, or
        // gave up waiting while the winner held it; it never moves twice.
        assert!(
            tokens == ["already-moved", "moved"] || tokens == ["in-use", "moved"],
            "{tokens:?}"
        );
        assert!(!root.legacy.exists());
        assert_eq!(
            fs::read_to_string(root.current.join("config").join("config.yaml")).unwrap(),
            "a: 1\n"
        );
        assert!(root.current.join(MOVE_MARKER).exists());
        assert_eq!(root.resolve(), root.current);
    }
}

/// A rename that fails a few times and then succeeds (an antivirus scan
/// letting go) is retried, not abandoned.
#[test]
fn a_rename_that_fails_briefly_is_retried() {
    let base = tempfile::tempdir().unwrap();
    let root = AppRoot::under(base.path());
    fs::create_dir_all(&root.legacy).unwrap();
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let flaky = |from: &Path, to: &Path| {
        if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
            refuse(from, to)
        } else {
            fs::rename(from, to)
        }
    };
    let delays = [Duration::ZERO; 3];
    let outcome = move_root(
        &root,
        true,
        &MoveOps {
            rename: &flaky,
            retry_delays: &delays,
        },
    );
    assert_eq!(outcome.token(), "moved");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

/// Under a sandbox root nothing real is looked at: the bases are not even
/// asked for, and a legacy folder beside the sandbox stays exactly where it
/// is.
#[test]
fn a_sandbox_root_skips_the_move_entirely() {
    let base = tempfile::tempdir().unwrap();
    materialize(base.path(), "legacy/config/config.yaml");
    let asked = std::cell::Cell::new(false);
    let report = move_for_process(
        Some("/tmp/sandbox".to_string()),
        || {
            asked.set(true);
            Some((base.path().to_path_buf(), base.path().to_path_buf()))
        },
        &ops(false),
    );
    assert!(report.sandboxed);
    assert!(!asked.get(), "the bases were read under a sandbox root");
    assert!(report.roaming.is_none() && report.local.is_none() && report.rewrite.is_none());
    assert!(token_path(base.path(), "legacy/config/config.yaml").exists());
    assert!(!token_path(base.path(), "current").exists());
}

/// Both bases move: config, data and logs in the roaming one, the cache in the
/// local one. Only the roaming one gets the marker, because only its stores
/// hold paths.
#[test]
fn a_process_moves_both_bases_and_marks_only_the_roaming_one() {
    let roaming = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    materialize(roaming.path(), "legacy/config/config.yaml");
    materialize(local.path(), "legacy/cache/autovisualiser/figure.html");

    let report = move_for_process(
        None,
        || Some((roaming.path().to_path_buf(), local.path().to_path_buf())),
        &ops(false),
    );
    assert_eq!(report.roaming.as_ref().unwrap().outcome.token(), "moved");
    assert_eq!(report.local.as_ref().unwrap().outcome.token(), "moved");
    assert!(token_path(local.path(), "current/cache/autovisualiser/figure.html").exists());
    assert!(!token_path(local.path(), "current/marker").exists());
    let rewrite = report
        .rewrite
        .expect("the roaming move left a marker to finish");
    assert!(rewrite.marker_cleared && rewrite.failed.is_empty());
    assert!(!token_path(roaming.path(), "current/marker").exists());

    let windows = crate::paths::windows_dirs(roaming.path(), local.path());
    assert_eq!(windows.config, token_path(roaming.path(), "current/config"));
    assert_eq!(windows.data, token_path(roaming.path(), "current/data"));
    assert_eq!(windows.cache, token_path(local.path(), "current/cache"));
    assert_eq!(windows.state, None);
}

/// The layout under a base whose move failed still points at the legacy
/// folder, so nothing appears lost for that run.
#[test]
fn the_layout_follows_a_failed_move_to_the_legacy_folder() {
    let roaming = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    materialize(roaming.path(), "legacy/config/config.yaml");
    let report = move_for_process(
        None,
        || Some((roaming.path().to_path_buf(), local.path().to_path_buf())),
        &ops(true),
    );
    assert_eq!(
        report.roaming.as_ref().unwrap().outcome.token(),
        "move-failed"
    );
    assert!(report.rewrite.is_none(), "nothing to rewrite before a move");
    let windows = crate::paths::windows_dirs(roaming.path(), local.path());
    assert_eq!(windows.config, token_path(roaming.path(), "legacy/config"));
    assert_eq!(windows.cache, token_path(local.path(), "current/cache"));
}

/// The real thing on Windows: a file held open with no sharing. Windows is
/// expected to refuse the rename, and whichever way it goes, every byte must
/// be where the resolver says the folder is.
#[cfg(windows)]
#[test]
fn a_file_held_open_never_loses_data() {
    use std::os::windows::fs::OpenOptionsExt;
    let base = tempfile::tempdir().unwrap();
    let root = AppRoot::under(base.path());
    fs::create_dir_all(root.legacy.join("data").join("sessions")).unwrap();
    let db = root
        .legacy
        .join("data")
        .join("sessions")
        .join("sessions.db");
    fs::write(&db, b"held").unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&db)
        .unwrap();

    let outcome = move_root(&root, true, &ops(false));
    let resolved = root.resolve().to_path_buf();
    drop(held);
    eprintln!("a held file made the move end {}", outcome.token());
    assert!(matches!(outcome.token(), "move-failed" | "moved"));
    assert_eq!(
        resolved == root.legacy,
        outcome.token() == "move-failed",
        "the resolver must follow the outcome"
    );
    assert_eq!(
        fs::read(resolved.join("data").join("sessions").join("sessions.db")).unwrap(),
        b"held"
    );
}

// ---------------------------------------------------------------------------
// Rewriting stored paths.

/// A Windows-shaped roaming base. `Path::join` adds `/` off Windows, which is
/// fine: the rewrite treats `\` and `/` alike, as Windows does.
const BASE: &str = r"C:\Users\Ana\AppData\Roaming";

fn windows_rewrite() -> RootRewrite {
    RootRewrite::new(&AppRoot::under(Path::new(BASE)))
}

fn legacy(rest: &str) -> String {
    format!(r"{BASE}\{LEGACY_APP_AUTHOR}\{APP_NAME}{rest}")
}

fn current(rest: &str) -> String {
    format!(r"{BASE}\{APP_AUTHOR}\{APP_NAME}{rest}")
}

#[test]
fn a_path_under_the_legacy_root_moves_with_it() {
    let rw = windows_rewrite();
    for rest in [
        "",
        r"\config",
        r"\config\extensions\spokeagent",
        r"\data\sessions",
    ] {
        assert_eq!(
            rw.rewrite_str(&legacy(rest)).as_deref(),
            Some(current(rest).as_str())
        );
    }
}

#[test]
fn case_and_separators_are_matched_loosely_and_kept_as_stored() {
    let rw = windows_rewrite();
    let lower = legacy(r"\config").to_lowercase();
    let expected = lower.replacen(&LEGACY_APP_AUTHOR.to_lowercase(), APP_AUTHOR, 1);
    assert_eq!(rw.rewrite_str(&lower).as_deref(), Some(expected.as_str()));

    let forward = legacy(r"\config\extensions\x").replace('\\', "/");
    assert_eq!(
        rw.rewrite_str(&forward),
        Some(current(r"\config\extensions\x").replace('\\', "/"))
    );

    let mixed = format!("C:/Users\\Ana/AppData\\Roaming/{LEGACY_APP_AUTHOR}\\{APP_NAME}/config");
    assert_eq!(
        rw.rewrite_str(&mixed),
        Some(format!(
            "C:/Users\\Ana/AppData\\Roaming/{APP_AUTHOR}\\{APP_NAME}/config"
        ))
    );
}

#[test]
fn a_verbatim_canonical_path_stays_verbatim() {
    let rw = windows_rewrite();
    assert_eq!(
        rw.rewrite_str(&format!(r"\\?\{}", legacy(r"\config"))),
        Some(format!(r"\\?\{}", current(r"\config")))
    );
}

#[test]
fn every_occurrence_inside_a_longer_value_is_rewritten() {
    let rw = windows_rewrite();
    let value = format!("--directory={};{}", legacy(r"\a"), legacy(r"\b"));
    assert_eq!(
        rw.rewrite_str(&value),
        Some(format!("--directory={};{}", current(r"\a"), current(r"\b")))
    );
}

#[test]
fn lookalikes_and_other_folders_are_never_rewritten() {
    let rw = windows_rewrite();
    for value in [
        format!(r"{BASE}\{LEGACY_APP_AUTHOR}2\{APP_NAME}\config"),
        format!(r"{BASE}\X{LEGACY_APP_AUTHOR}\{APP_NAME}\config"),
        format!(r"{BASE}\{LEGACY_APP_AUTHOR}\{APP_NAME}2\config"),
        format!(r"{BASE}\{LEGACY_APP_AUTHOR}\{APP_NAME}-old"),
        format!(r"{BASE}\{LEGACY_APP_AUTHOR}\{APP_NAME}.bak"),
        format!(r"D:\Other\{LEGACY_APP_AUTHOR}\{APP_NAME}\config"),
        format!(r"X{}", legacy(r"\config")),
        format!(r"{BASE}\{APP_AUTHOR}\{APP_NAME}\config"),
        format!(r"{BASE}\{LEGACY_APP_AUTHOR}\other-app\config"),
    ] {
        assert_eq!(rw.rewrite_str(&value), None, "{value} must be left alone");
    }
}

/// JSON escapes every backslash, and the escaped text is what sits in the
/// file and in the session table.
#[test]
fn a_json_document_is_rewritten_on_its_escaped_text() {
    let rw = windows_rewrite();
    let doc = serde_json::json!({
        "install_dir": legacy(r"\config\extensions\x"),
        "source_url": "https://example.org/x.brxt",
        "nested": [{"config_dir": format!(r"\\?\{}", legacy(r"\config"))}],
    });
    let text = serde_json::to_string_pretty(&doc).unwrap();
    let rewritten = rw
        .rewrite_document(&text, Format::Json)
        .unwrap()
        .expect("changed");
    let parsed: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
    assert_eq!(parsed["install_dir"], current(r"\config\extensions\x"));
    assert_eq!(
        parsed["nested"][0]["config_dir"],
        format!(r"\\?\{}", current(r"\config"))
    );
    assert_eq!(rewritten.replace(APP_AUTHOR, LEGACY_APP_AUTHOR), text);
}

/// Comments, quoting and indentation survive, because only the author
/// segments in the text change.
#[test]
fn a_yaml_document_keeps_every_other_byte() {
    let rw = windows_rewrite();
    let plain = legacy(r"\config\extensions\spokeagent");
    let quoted = legacy(r"\config\extensions\omop").replace('\\', r"\\");
    let text = format!(
        "# my extensions\nextensions:\n  spokeagent:\n    cmd: uv   # pinned\n    args:\n    - run\n    - --directory\n    - {plain}\n  omop:\n    args: [\"run\", \"--directory\", \"{quoted}\"]\n    envs:\n      DATA: '{plain}'\n"
    );
    let rewritten = rw
        .rewrite_document(&text, Format::Yaml)
        .unwrap()
        .expect("changed");
    assert_eq!(rewritten.replace(APP_AUTHOR, LEGACY_APP_AUTHOR), text);
    let parsed: serde_yaml::Value = serde_yaml::from_str(&rewritten).unwrap();
    assert_eq!(
        parsed["extensions"]["omop"]["args"][2].as_str(),
        Some(current(r"\config\extensions\omop").as_str())
    );
    assert_eq!(
        parsed["extensions"]["omop"]["envs"]["DATA"].as_str(),
        Some(current(r"\config\extensions\spokeagent").as_str())
    );
}

/// A text match the parse does not confirm is refused rather than written: a
/// plain YAML scalar with doubled backslashes is two separators to the parser
/// but one escaped separator to the text matcher.
#[test]
fn a_rewrite_the_parse_does_not_confirm_is_refused() {
    let rw = windows_rewrite();
    let doubled = legacy(r"\config").replace('\\', r"\\");
    let text = format!("path: {doubled}\n");
    assert!(rw.rewrite_document(&text, Format::Yaml).is_err());
    assert_eq!(
        rw.rewrite_document("path: elsewhere\n", Format::Yaml),
        Ok(None)
    );
}

/// A roaming profile redirected to a share: the environment names the base
/// `\\fs1\...`, and `fs::canonicalize`, which `daemon_runtime::profile_record`
/// stores, spells it `\\?\UNC\fs1\...`. The environment's spelling alone does
/// not match the stored one, so the rewrite also matches the canonical
/// spelling; without it the Crew vault refuses to unlock after the move.
#[test]
fn a_canonical_unc_path_on_a_redirected_profile_is_rewritten() {
    let plain = Path::new(r"\\fs1\home$\ana\AppData\Roaming");
    let canonical = Path::new(r"\\?\UNC\fs1\home$\ana\AppData\Roaming");
    let root = AppRoot::under(plain);
    let stored = format!(
        r"{}\{LEGACY_APP_AUTHOR}\{APP_NAME}\config",
        canonical.display()
    );
    let expected = format!(r"{}\{APP_AUTHOR}\{APP_NAME}\config", canonical.display());

    assert_eq!(
        RootRewrite::new(&root).rewrite_str(&stored),
        None,
        "the environment's spelling alone does not reach the canonical one"
    );
    let spellings = rewrite::root_spellings(&root, Some(canonical));
    assert_eq!(spellings.len(), 2);
    let rw = RootRewrite::for_roots(&spellings);
    assert_eq!(rw.rewrite_str(&stored).as_deref(), Some(expected.as_str()));
    assert_eq!(
        rw.rewrite_str(&format!(
            r"{plain}\{LEGACY_APP_AUTHOR}\{APP_NAME}\config",
            plain = plain.display()
        )),
        Some(format!(
            r"{plain}\{APP_AUTHOR}\{APP_NAME}\config",
            plain = plain.display()
        )),
        "the environment's spelling still matches too"
    );

    let profile = serde_json::to_string(
        &serde_json::json!({"version": 1, "profile_id": "p", "config_dir": stored}),
    )
    .unwrap();
    let rewritten = rw
        .rewrite_document(&profile, Format::Json)
        .unwrap()
        .expect("changed");
    let parsed: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
    assert_eq!(parsed["config_dir"], expected);
}

/// The same spelling found on disk: a base reached through a link (on
/// Windows a junction, a mapped drive or a `subst` drive), with the stored
/// config dir in the canonical spelling `daemon_runtime::profile_record`
/// writes. After the move it must equal the canonical current config dir,
/// which is exactly what that function compares it to.
#[cfg(unix)]
#[test]
fn a_canonical_path_through_a_link_in_the_base_is_rewritten() {
    let real = tempfile::tempdir().unwrap();
    let links = tempfile::tempdir().unwrap();
    let base = links.path().join("Roaming");
    std::os::unix::fs::symlink(real.path(), &base).unwrap();
    let root = AppRoot::under(&base);
    fs::create_dir_all(root.legacy.join("config")).unwrap();
    let stored = fs::canonicalize(root.legacy.join("config")).unwrap();
    assert!(
        !stored.starts_with(&base),
        "the stored spelling must differ"
    );
    fs::write(
        root.legacy.join("config").join("daemon-profile.json"),
        serde_json::to_string(
            &serde_json::json!({"version": 1, "profile_id": "p", "config_dir": stored}),
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(move_root(&root, true, &ops(false)).token(), "moved");
    let report = rewrite::finish_pending_rewrite(&root);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.files_rewritten.len(), 1);
    let profile: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.current.join("config").join("daemon-profile.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        profile["config_dir"].as_str().map(PathBuf::from),
        Some(fs::canonicalize(root.current.join("config")).unwrap()),
    );
}

// ---------------------------------------------------------------------------
// The stores, end to end.

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

async fn session_db(path: &Path, rows: &[(&str, &str, &str, Option<&str>)], message: &str) {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::Connection;
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, working_dir TEXT NOT NULL, \
         updated_at TEXT, extension_data TEXT DEFAULT '{}', workflow_json TEXT)",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query("CREATE TABLE messages (id INTEGER PRIMARY KEY, content_json TEXT)")
        .execute(&mut connection)
        .await
        .unwrap();
    for (id, working_dir, extension_data, workflow) in rows {
        sqlx::query(
            "INSERT INTO sessions (id, working_dir, updated_at, extension_data, workflow_json) \
             VALUES (?, ?, '2026-01-01', ?, ?)",
        )
        .bind(id)
        .bind(working_dir)
        .bind(extension_data)
        .bind(workflow)
        .execute(&mut connection)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO messages (content_json) VALUES (?)")
        .bind(message)
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
}

async fn read_sessions(path: &Path) -> Vec<(String, String, String, Option<String>, String)> {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::{Connection, Row};
    let options = SqliteConnectOptions::new().filename(path);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let rows = sqlx::query(
        "SELECT id, working_dir, extension_data, workflow_json, updated_at FROM sessions \
         ORDER BY id",
    )
    .fetch_all(&mut connection)
    .await
    .unwrap();
    rows.iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)))
        .collect()
}

/// Every store the rewrite covers, written the way its owner writes it, under
/// a real moved folder; plus the things it must leave alone.
#[test]
fn a_finished_move_points_every_store_at_the_current_folder() {
    let base = tempfile::tempdir().unwrap();
    let root = AppRoot::under(base.path());
    let old = root.legacy.to_string_lossy().into_owned();
    let new = root.current.to_string_lossy().into_owned();
    let ext = |root: &str| format!("{root}/config/extensions/spokeagent");

    let config = root.legacy.join("config");
    let data = root.legacy.join("data");
    let write = |path: PathBuf, body: String| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    };
    let config_yaml = format!(
        "# hand edited\nextensions:\n  spokeagent:\n    cmd: uv\n    args:\n    - run\n    - --directory\n    - {}\n    - spokeagent\n",
        ext(&old)
    );
    write(config.join("config.yaml"), config_yaml.clone());
    write(config.join("config.yaml.bak"), config_yaml.clone());
    write(
        config.join("knowledge").join("registry.yaml"),
        format!("bases:\n- id: soul\n  path: {old}/config/knowledge/soul\n"),
    );
    let json = |value: serde_json::Value| serde_json::to_string(&value).unwrap();
    write(
        config.join("daemon-profile.json"),
        json(
            serde_json::json!({"version": 1, "profile_id": "p", "config_dir": format!("{old}/config")}),
        ),
    );
    write(
        config.join("extension-provenance.json.d").join("393f.json"),
        json(serde_json::json!({"op": "upsert", "record": {"install_dir": ext(&old)}})),
    );
    write(
        config.join("extension-installs").join("3339"),
        json(serde_json::json!({"install_dir": ext(&old)})),
    );
    write(
        config.join("crew").join("connections.json"),
        json(
            serde_json::json!({"scopes": {"a": {"session_store": format!("{old}/data/sessions")}}}),
        ),
    );
    write(
        data.join("schedule.json"),
        json(
            serde_json::json!({"jobs": [{"source": format!("{old}/data/scheduled_workflows/j.yaml")}]}),
        ),
    );
    write(
        data.join("scheduled_workflows").join("j.yaml"),
        format!("extensions:\n- args: [run, --directory, '{}']\n", ext(&old)),
    );
    write(data.join("logs").join("server.log"), format!("{old}\n"));

    let snapshot = json(serde_json::json!({
        "enabled_extensions.v0": {"extensions": [{"cmd": "uv", "args": ["run", "--directory", ext(&old), "spokeagent"]}]}
    }));
    let message = json(serde_json::json!({"text": format!("I wrote {old}/cache/a.html")}));
    let db = data.join("sessions").join("sessions.db");
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    run(session_db(
        &db,
        &[
            ("a", &format!("{old}/data/crew/tasks"), &snapshot, None),
            (
                "b",
                "/home/ana/project",
                "{}",
                Some("{\"title\":\"plain\"}"),
            ),
        ],
        &message,
    ));

    assert_eq!(move_root(&root, true, &ops(false)).token(), "moved");
    let report = rewrite::finish_pending_rewrite(&root);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.unverified.is_empty(), "{:?}", report.unverified);
    assert!(report.marker_cleared);
    assert!(!root.current.join(MOVE_MARKER).exists());
    assert_eq!(
        report.files_rewritten.len(),
        8,
        "{:?}",
        report.files_rewritten
    );
    assert_eq!(report.session_rows_rewritten, 2);

    let config = root.current.join("config");
    let data = root.current.join("data");
    let read = |path: PathBuf| fs::read_to_string(path).unwrap();
    assert_eq!(
        read(config.join("config.yaml")),
        config_yaml.replace(&old, &new),
        "only the author changes, comments included"
    );
    for (path, expected) in [
        (config.join("knowledge").join("registry.yaml"), 1),
        (config.join("daemon-profile.json"), 1),
        (
            config.join("extension-provenance.json.d").join("393f.json"),
            1,
        ),
        (config.join("extension-installs").join("3339"), 1),
        (config.join("crew").join("connections.json"), 1),
        (data.join("schedule.json"), 1),
        (data.join("scheduled_workflows").join("j.yaml"), 1),
    ] {
        let body = read(path.clone());
        assert_eq!(body.matches(&new).count(), expected, "{}", path.display());
        assert!(
            !body.contains(&old),
            "{} still names the old root",
            path.display()
        );
    }
    assert_eq!(
        read(config.join("config.yaml.bak")),
        config_yaml,
        "a backup is history"
    );
    assert_eq!(
        read(data.join("logs").join("server.log")),
        format!("{old}\n")
    );

    let sessions = run(read_sessions(
        &root.current.join("data/sessions/sessions.db"),
    ));
    assert_eq!(sessions[0].1, format!("{new}/data/crew/tasks"));
    assert!(sessions[0].2.contains(&ext(&new)) && !sessions[0].2.contains(&old));
    assert_eq!(
        sessions[0].4, "2026-01-01",
        "the chat list order must not move"
    );
    assert_eq!(sessions[1].1, "/home/ana/project");
    assert_eq!(sessions[1].3.as_deref(), Some("{\"title\":\"plain\"}"));

    // The transcript is history and keeps the path it was written with.
    let transcript = run(async {
        use sqlx::{Connection, Row};
        let mut connection = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(root.current.join("data/sessions/sessions.db")),
        )
        .await
        .unwrap();
        sqlx::query("SELECT content_json FROM messages")
            .fetch_one(&mut connection)
            .await
            .unwrap()
            .get::<String, _>(0)
    });
    assert_eq!(transcript, message);

    // A second pass finds nothing left to do.
    fs::File::create(root.current.join(MOVE_MARKER)).unwrap();
    let again = rewrite::finish_pending_rewrite(&root);
    assert!(again.files_rewritten.is_empty() && again.session_rows_rewritten == 0);
    assert!(again.marker_cleared);
}

/// A crash after the rename and before the rewrite: the marker came along with
/// the tree, so the next start finishes the job.
#[test]
fn a_crash_between_the_rename_and_the_rewrite_is_finished_next_start() {
    let roaming = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let root = AppRoot::under(roaming.path());
    let old = root.legacy.to_string_lossy().into_owned();
    fs::create_dir_all(root.legacy.join("config")).unwrap();
    fs::write(
        root.legacy.join("config").join("config.yaml"),
        format!("x: {old}/config/extensions/a\n"),
    )
    .unwrap();
    assert_eq!(move_root(&root, true, &ops(false)).token(), "moved");
    // The process dies here, before any rewrite.

    let next_start = move_for_process(
        None,
        || Some((roaming.path().to_path_buf(), local.path().to_path_buf())),
        &ops(false),
    );
    assert_eq!(
        next_start.roaming.as_ref().unwrap().outcome.token(),
        "already-moved"
    );
    let rewrite = next_start.rewrite.expect("the marker asked for a rewrite");
    assert_eq!(rewrite.files_rewritten.len(), 1);
    assert!(rewrite.marker_cleared);

    let third = move_for_process(
        None,
        || Some((roaming.path().to_path_buf(), local.path().to_path_buf())),
        &ops(false),
    );
    assert!(third.rewrite.is_none(), "nothing is left to finish");
}

/// A store that cannot be written keeps the marker, so the next start tries
/// again; the stores that could be written are done.
#[cfg(unix)]
#[test]
fn a_store_that_cannot_be_written_keeps_the_marker() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let root = AppRoot::under(base.path());
    let old = root.legacy.to_string_lossy().into_owned();
    fs::create_dir_all(root.legacy.join("config").join("crew")).unwrap();
    fs::write(
        root.legacy.join("config").join("config.yaml"),
        format!("x: {old}/config/a\n"),
    )
    .unwrap();
    let crew = root.legacy.join("config").join("crew");
    fs::write(
        crew.join("connections.json"),
        format!("{{\"s\":\"{old}/data/sessions\"}}"),
    )
    .unwrap();
    assert_eq!(move_root(&root, true, &ops(false)).token(), "moved");

    let crew = root.current.join("config").join("crew");
    fs::set_permissions(&crew, fs::Permissions::from_mode(0o500)).unwrap();
    let report = rewrite::finish_pending_rewrite(&root);
    fs::set_permissions(&crew, fs::Permissions::from_mode(0o700)).unwrap();

    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(!report.marker_cleared);
    assert!(root.current.join(MOVE_MARKER).exists());
    assert_eq!(report.files_rewritten.len(), 1);
}

/// The desktop main process carries the same three values in `appFolder.ts`.
/// Read from its source, and compared to these constants rather than to
/// literals, so no third copy of the legacy name exists.
#[test]
fn the_desktop_constants_match_these() {
    let source_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/desktop/src/utils/appFolder.ts");
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|error| panic!("{}: {error}", source_path.display()));
    for (name, value) in [
        ("APP_AUTHOR", APP_AUTHOR),
        ("APP_NAME", APP_NAME),
        ("LEGACY_APP_AUTHOR", LEGACY_APP_AUTHOR),
    ] {
        let declaration = format!("export const {name} = '{value}';");
        assert!(
            source.contains(&declaration),
            "appFolder.ts must declare `{declaration}` to agree with the daemon"
        );
    }
    assert!(
        source.contains("resolution-cases.json") || {
            let test = fs::read_to_string(source_path.with_file_name("appFolder.test.ts"))
                .unwrap_or_default();
            test.contains("resolution-cases.json")
        },
        "the desktop suite must run the shared table"
    );
}

// ---------------------------------------------------------------------------
// Who may move, and when.

/// The race the move must never lose: one process resolved the legacy folder
/// and froze the path (as `Config::global`, the session store and the log
/// directory do), and a second process starts and would move the folder out
/// from under it. The first holds the lock, as `platform_dirs` makes every
/// resolver do, so the second leaves the folder where it is and resolves the
/// same one; the frozen writer's data stays in the one folder in use, and the
/// next start with nobody left on the old folder moves it, data included.
#[test]
fn a_process_using_the_legacy_folder_keeps_the_move_away() {
    let roaming = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    materialize(roaming.path(), "legacy/config/config.yaml");
    let bases = || Some((roaming.path().to_path_buf(), local.path().to_path_buf()));

    // The first process resolves, holds, and freezes its config dir.
    let first = Pins::new();
    let root = AppRoot::under(roaming.path());
    first.hold_while_legacy(&[&root]);
    assert!(
        !first.is_empty(),
        "a process using the legacy folder must hold its lock"
    );
    let frozen = crate::paths::windows_dirs(roaming.path(), local.path()).config;
    assert_eq!(frozen, token_path(roaming.path(), "legacy/config"));

    // The daemon starts.
    let second = move_for_process(None, bases, &ops(false));
    assert_eq!(second.roaming.as_ref().unwrap().outcome.token(), "in-use");
    assert!(
        !token_path(roaming.path(), "current").exists(),
        "the folder moved under a process that had frozen its path"
    );
    assert_eq!(
        crate::paths::windows_dirs(roaming.path(), local.path()).config,
        frozen,
        "both processes must resolve the one folder in use"
    );
    assert!(
        !second.pins.is_empty(),
        "the daemon now uses the legacy folder too, so it holds the lock"
    );

    // The first process's cached writer: `Config::save_values` runs
    // `create_dir_all` on the parent it froze, then writes.
    fs::create_dir_all(&frozen).unwrap();
    fs::write(frozen.join("config.yaml"), "written: later\n").unwrap();

    // Everyone exits; the next start moves the folder with the write in it.
    drop(first);
    drop(second);
    let next = move_for_process(None, bases, &ops(false));
    assert_eq!(next.roaming.as_ref().unwrap().outcome.token(), "moved");
    assert!(next.pins.is_empty());
    assert_eq!(
        fs::read_to_string(token_path(roaming.path(), "current/config/config.yaml")).unwrap(),
        "written: later\n"
    );
    assert!(!token_path(roaming.path(), "legacy").exists());
    assert!(
        !token_path(roaming.path(), "legacyParent").exists(),
        "the lock and the emptied author folder go with the move"
    );
}

/// A process that only ever sees the current folder (or a fresh install)
/// holds nothing, and a resolver holds each legacy lock once.
#[test]
fn a_resolver_holds_a_lock_only_while_it_uses_a_legacy_folder() {
    let fresh = tempfile::tempdir().unwrap();
    let moved = tempfile::tempdir().unwrap();
    materialize(moved.path(), "current/config/config.yaml");
    let pins = Pins::new();
    pins.hold_while_legacy(&[&AppRoot::under(fresh.path()), &AppRoot::under(moved.path())]);
    assert!(pins.is_empty());
    assert!(
        !token_path(fresh.path(), "legacyParent").exists(),
        "holding nothing creates nothing"
    );

    let legacy = tempfile::tempdir().unwrap();
    materialize(legacy.path(), "legacy/config/config.yaml");
    let root = AppRoot::under(legacy.path());
    pins.hold_while_legacy(&[&root]);
    pins.hold_while_legacy(&[&root]);
    assert_eq!(pins.into_inner().len(), 1);
}

/// A process that starts while the daemon is moving waits for the move, and
/// then resolves the folder it moved to.
#[test]
fn a_resolver_that_starts_during_a_move_waits_for_it() {
    let base = tempfile::tempdir().unwrap();
    materialize(base.path(), "legacy/config/config.yaml");
    let root = AppRoot::under(base.path());
    let mover = match take_exclusive(&root.move_lock(), NO_DELAYS) {
        Exclusive::Held(lock) => lock,
        _ => panic!("nobody else holds the lock"),
    };

    let (done, finished) = std::sync::mpsc::channel();
    let waiting = {
        let root = root.clone();
        std::thread::spawn(move || {
            Pins::new().hold_while_legacy(&[&root]);
            done.send(root.resolve().to_path_buf()).unwrap();
        })
    };
    assert!(
        finished.recv_timeout(Duration::from_millis(200)).is_err(),
        "a resolver went ahead in the middle of a move"
    );
    assert_eq!(attempt_move(&root, true, &ops(false)).token(), "moved");
    mover.unlock().unwrap();
    assert_eq!(
        finished.recv_timeout(Duration::from_secs(10)).unwrap(),
        root.current
    );
    waiting.join().unwrap();
}

/// Only the daemon (`biorouterd agent`) moves: the command line tool, the
/// `mcp` children and every resolver only resolve, so no process that has
/// frozen the legacy path can be the one that moves it, and no test binary
/// that reaches a resolver can move a developer's real folder.
#[test]
fn only_the_daemon_moves_the_folder() {
    // Built from pieces, so this test's own source is not a caller.
    let needle = ["move_for_this", "_process("].concat();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut callers = Vec::new();
    let mut stack = vec![crates];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !path.ends_with("target") {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("app_folder/mod.rs")
            {
                continue;
            }
            let code = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = code.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if line.trim_start().starts_with("//") || !line.contains(&needle) {
                    continue;
                }
                let arm = lines[index.saturating_sub(12)..index]
                    .iter()
                    .any(|above| above.contains("Commands::Agent"));
                callers.push((path.display().to_string(), arm));
            }
        }
    }
    assert_eq!(
        callers.len(),
        1,
        "the move has callers besides the daemon: {callers:?}"
    );
    let (file, in_agent_arm) = &callers[0];
    assert!(
        file.ends_with("biorouter-server/src/main.rs") && *in_agent_arm,
        "the one caller must be `biorouterd agent`, found {callers:?}"
    );
}
