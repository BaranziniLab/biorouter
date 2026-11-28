//! J-P03: the clamp and the folder check as plain values.

use super::*;
use rmcp::model::ToolAnnotations;
use serde_json::json;

fn tool(name: &str, read_only: Option<bool>) -> Tool {
    let tool = Tool::new(
        name.to_string(),
        String::new(),
        json!({"type": "object"}).as_object().unwrap().clone(),
    );
    match read_only {
        Some(read_only) => tool.annotate(ToolAnnotations {
            title: None,
            read_only_hint: Some(read_only),
            destructive_hint: Some(!read_only),
            idempotent_hint: None,
            open_world_hint: None,
        }),
        None => tool,
    }
}

fn bundled(name: &str) -> ExtensionConfig {
    crate::agents::extension_manager::resolve_bundled_extension(name)
        .unwrap()
        .into_config(String::new())
}

fn third_party(name: &str) -> ExtensionConfig {
    ExtensionConfig::Stdio {
        name: name.to_string(),
        description: String::new(),
        cmd: "true".to_string(),
        args: Vec::new(),
        envs: Default::default(),
        env_keys: Vec::new(),
        timeout: None,
        bundled: None,
        available_tools: Vec::new(),
    }
}

fn offered() -> Vec<Tool> {
    vec![
        tool("developer__shell", None),
        tool("developer__text_editor", None),
        tool("lab__search", Some(true)),
        tool("lab__write_row", Some(false)),
        tool("lab__untagged", None),
        tool("code_execution__execute_code", Some(true)),
        tool("platform__manage_workflow", Some(false)),
        tool("platform__read_session_blob", Some(true)),
        tool(FINAL_OUTPUT_TOOL_NAME, None),
    ]
}

fn loaded() -> Vec<ExtensionConfig> {
    vec![
        bundled("developer"),
        third_party("lab"),
        bundled("code_execution"),
    ]
}

fn granted(clamp: &ToolClamp) -> Vec<&str> {
    clamp.tools.iter().map(String::as_str).collect()
}

#[test]
fn no_tools_keeps_only_the_structured_output_tool() {
    let clamp = ToolClamp::for_preset(SpawnPreset::NoTools, &offered(), &loaded());
    assert_eq!(granted(&clamp), [FINAL_OUTPUT_TOOL_NAME]);
    clamp.validate().unwrap();
}

#[test]
fn only_extensions_keeps_every_tool_of_the_loaded_extensions_and_no_platform_actor() {
    let clamp = ToolClamp::for_preset(
        SpawnPreset::OnlyExtensions,
        &offered(),
        &[third_party("lab")],
    );
    assert_eq!(
        granted(&clamp),
        [
            "lab__search",
            "lab__untagged",
            "lab__write_row",
            "platform__read_session_blob",
            FINAL_OUTPUT_TOOL_NAME,
        ]
    );
}

/// A third-party extension stored under `platform` (or under the
/// structured-output tool's `workflow` prefix) does not own Biorouter's own
/// tools of the same prefix: naming it under `only_extensions` grants none of
/// them, it is not kept, and an extension call in those namespaces is refused
/// even where the clamp grants the agent's own tool.
#[test]
fn an_extension_named_like_biorouters_own_tools_owns_none_of_them() {
    let offered = vec![
        tool("platform__manage_workflow", Some(false)),
        tool("platform__manage_schedule", Some(false)),
        tool("platform__read_session_blob", Some(true)),
        tool(FINAL_OUTPUT_TOOL_NAME, None),
    ];
    for preset in [SpawnPreset::OnlyExtensions, SpawnPreset::ReadOnly] {
        let clamp = ToolClamp::for_preset(
            preset,
            &offered,
            &[third_party("platform"), third_party("workflow")],
        );
        assert_eq!(
            granted(&clamp),
            ["platform__read_session_blob", FINAL_OUTPUT_TOOL_NAME],
            "{preset:?}"
        );
        assert!(!clamp.keeps_extension("platform"), "{preset:?}");
        assert!(!clamp.keeps_extension("workflow"), "{preset:?}");
        clamp.validate().unwrap();
        for name in ["platform__read_session_blob", FINAL_OUTPUT_TOOL_NAME] {
            assert!(clamp.refusal(name, None).is_none(), "{name}");
            let refusal = clamp
                .refusal_for_extension_call(name, None)
                .unwrap_or_else(|| panic!("{name} is never an extension's to answer"));
            assert!(refusal.contains("Biorouter's own tool names"), "{refusal}");
        }
    }
}

#[test]
fn read_only_keeps_readers_view_and_drops_writers_unknowns_and_acting_extensions() {
    let clamp = ToolClamp::for_preset(SpawnPreset::ReadOnly, &offered(), &loaded());
    assert_eq!(
        granted(&clamp),
        [
            "developer__text_editor",
            "lab__search",
            "platform__read_session_blob",
            FINAL_OUTPUT_TOOL_NAME,
        ],
        "the shell, a writer, an unannotated tool, Code Execution (even marked \
         read-only) and the workflow manager all go"
    );
    assert!(clamp.keeps_extension("developer"));
    assert!(!clamp.keeps_extension("code_execution"));
}

#[test]
fn a_view_only_tool_may_view_and_nothing_else() {
    let clamp = ToolClamp::for_preset(SpawnPreset::ReadOnly, &offered(), &loaded());
    let args = |command: &str| {
        json!({"command": command, "path": "a.md"})
            .as_object()
            .cloned()
    };
    assert_eq!(
        clamp.refusal("developer__text_editor", args("view").as_ref()),
        None
    );
    for command in ["write", "str_replace", "insert", "undo_edit"] {
        let refusal = clamp
            .refusal("developer__text_editor", args(command).as_ref())
            .unwrap_or_else(|| panic!("{command} is refused"));
        assert!(refusal.contains("may only view"), "{refusal}");
        assert!(
            clamp
                .refusal_for_extension_call("developer__text_editor", args(command).as_ref())
                .is_some(),
            "{command} is refused at the extension's dispatch too"
        );
    }
    assert!(clamp.refusal("developer__text_editor", None).is_some());
    let refusal = clamp.refusal("developer__shell", None).unwrap();
    assert!(
        refusal.contains("started with the read_only preset"),
        "{refusal}"
    );
}

/// A third-party tool named like Developer's is not Developer's: it is
/// judged by its own annotations.
#[test]
fn only_biorouters_developer_gets_the_view_exception() {
    let tools = vec![tool("developer__text_editor", None)];
    let clamp = ToolClamp::for_preset(SpawnPreset::ReadOnly, &tools, &[third_party("developer")]);
    assert!(granted(&clamp).is_empty(), "{:?}", granted(&clamp));
}

#[test]
fn a_stored_clamp_round_trips_and_a_forged_one_is_refused() {
    let clamp = ToolClamp::for_preset(SpawnPreset::ReadOnly, &offered(), &loaded());
    let value = serde_json::to_value(&clamp).unwrap();
    let back: ToolClamp = serde_json::from_value(value).unwrap();
    assert_eq!(back, clamp);

    for forged in [
        json!({ "preset": "no_tools", "tools": ["developer__shell"] }),
        json!({
            "preset": "read_only",
            "tools": ["a__b"],
            "view_only": ["developer__text_editor"]
        }),
        json!({ "preset": "only_extensions", "tools": ["platform__manage_schedule"] }),
    ] {
        let forged: ToolClamp = serde_json::from_value(forged).unwrap();
        assert!(forged.validate().is_err(), "{forged:?}");
    }
    assert!(serde_json::from_value::<ToolClamp>(json!({
        "preset": "read_only",
        "tools": [],
        "extra": true
    }))
    .is_err());
}

// ─── The folder check ───────────────────────────────────────────────────────

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// A walk with no home, no global ignore file and the given limits.
fn take(root: &Path, limits: FolderLimits) -> FolderSnapshot {
    FolderSnapshot::take_with(root, &[], None, None, limits, &AtomicBool::new(false))
}

fn snap(root: &Path) -> FolderSnapshot {
    take(root, FolderLimits::default())
}

fn small(max_files: usize) -> FolderLimits {
    FolderLimits {
        max_files,
        ..FolderLimits::default()
    }
}

#[test]
fn an_untouched_folder_compares_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "a");
    write(&dir.path().join("data/b.csv"), "b");
    let check = snap(dir.path()).compare(&snap(dir.path()));
    assert!(check.unchanged, "{check:?}");
    assert_eq!(check.files_compared, 2);
    assert!(check.complete);
    let sentence = check.sentence();
    assert!(sentence.contains("no file in"), "{sentence}");
    assert!(
        sentence.contains("ignored files were left out"),
        "{sentence}"
    );
}

#[test]
fn changed_added_and_removed_files_are_named() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "a");
    write(&dir.path().join("gone.txt"), "x");
    let before = snap(dir.path());
    write(&dir.path().join("a.md"), "edited");
    write(&dir.path().join("sub/new.txt"), "n");
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    let check = before.compare(&snap(dir.path()));
    assert!(!check.unchanged);
    assert_eq!(check.changed, ["a.md"]);
    assert_eq!(check.added, ["sub/new.txt"]);
    assert_eq!(check.removed, ["gone.txt"]);
    let sentence = check.sentence();
    assert!(
        sentence.contains("3 files") && sentence.contains("changed while the subagent ran"),
        "{sentence}"
    );
    assert!(
        sentence.contains("this chat, another subagent or another program"),
        "a background verifier's parent keeps working, so it may be the writer: {sentence}"
    );
}

#[test]
fn same_size_content_changes_are_caught_by_the_hash() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), "aaaa");
    let before = snap(dir.path());
    write(&dir.path().join("a.txt"), "bbbb");
    let check = before.compare(&snap(dir.path()));
    assert_eq!(check.changed, ["a.txt"]);
}

#[test]
fn biorouters_own_writes_and_git_internals_are_not_reported_but_git_head_and_hooks_are() {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("store");
    write(&own.join("sessions.db"), "1");
    write(&dir.path().join(".git/HEAD"), "ref: refs/heads/main\n");
    write(&dir.path().join(".git/index"), "1");
    let take_skipping = |skip: &[PathBuf]| {
        FolderSnapshot::take_with(
            dir.path(),
            skip,
            None,
            None,
            FolderLimits::default(),
            &AtomicBool::new(false),
        )
    };
    let before = take_skipping(std::slice::from_ref(&own));
    write(&dir.path().join(".git/index"), "changed");
    write(&dir.path().join(".git/objects/ab/cd"), "object");
    write(
        &dir.path().join(".biorouter/tool-output/shell-x-1.txt"),
        "big result",
    );
    write(&dir.path().join(".biorouter/.gitignore"), "*\n");
    write(&own.join("sessions.db"), "2");
    let check = before.compare(&take_skipping(std::slice::from_ref(&own)));
    assert!(check.unchanged, "{check:?}");

    write(&dir.path().join(".git/hooks/pre-commit"), "#!/bin/sh\n");
    write(&dir.path().join(".git/HEAD"), "ref: refs/heads/other\n");
    let check = before.compare(&take_skipping(std::slice::from_ref(&own)));
    assert_eq!(check.changed, [".git/HEAD"], "{check:?}");
    assert_eq!(check.added, [".git/hooks/pre-commit"], "{check:?}");
}

#[cfg(unix)]
#[test]
fn a_link_is_recorded_by_its_target_and_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("big.txt"), "outside");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
    let before = snap(dir.path());
    write(&outside.path().join("big.txt"), "changed outside");
    let check = before.compare(&snap(dir.path()));
    assert!(check.unchanged, "the link was not followed: {check:?}");
    assert_eq!(check.files_compared, 1);
}

#[test]
fn a_missing_folder_says_it_was_not_checked() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("nope");
    let check = snap(&gone).compare(&snap(&gone));
    assert!(!check.unchanged);
    assert!(check.not_checked.is_some());
    assert!(
        check.sentence().contains("was not checked"),
        "{}",
        check.sentence()
    );
}

#[test]
fn long_lists_are_cut_and_counted() {
    let changed = (0..25).map(|i| format!("f{i}")).collect();
    let check = FolderCheck::from_lists(
        "/w".into(),
        25,
        changed,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(check.changed.len(), FOLDER_CHECK_LISTED);
    assert_eq!(check.unlisted, 5);
    let sentence = check.sentence();
    assert!(sentence.contains("(and 5 more)"), "{sentence}");
    assert!(sentence.contains("25 files"), "{sentence}");
}

/// The reviewer's probe: a dependency folder that sorts before the source
/// used to fill the file limit, so the changed source file was never looked
/// at and the check still said "no file changed". Dependency folders are now
/// left out, by name and by `CACHEDIR.TAG`, and said to be.
#[test]
fn a_dependency_folder_cannot_starve_the_source_of_the_check() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..30 {
        write(&dir.path().join(format!("node_modules/p{i:02}.js")), "x");
        write(&dir.path().join(format!("build/out{i:02}.o")), "x");
    }
    write(
        &dir.path().join("build/CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55",
    );
    write(&dir.path().join("src/main.rs"), "fn main() {}");
    let limits = small(10);
    let before = take(dir.path(), limits);
    write(&dir.path().join("src/main.rs"), "fn main() { changed() }");
    write(&dir.path().join("node_modules/p00.js"), "changed by npm");
    let check = before.compare(&take(dir.path(), limits));
    assert_eq!(check.changed, ["src/main.rs"], "{check:?}");
    assert!(check.complete, "{check:?}");
    assert_eq!(check.left_out, ["build", "node_modules"], "{check:?}");
    let sentence = check.sentence();
    assert!(
        sentence.contains("ignored files and build, node_modules were left out"),
        "{sentence}"
    );
}

/// When the file limit is reached anyway, the check never says "no file
/// changed": it says what it did not compare, and `unchanged` is false.
#[test]
fn a_check_that_stopped_at_the_file_limit_never_claims_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..30 {
        write(&dir.path().join(format!("aaa/f{i:02}.csv")), "x");
    }
    write(&dir.path().join("src/main.rs"), "fn main() {}");
    let limits = small(10);
    let before = take(dir.path(), limits);
    write(&dir.path().join("src/main.rs"), "fn main() { changed() }");
    let check = before.compare(&take(dir.path(), limits));
    assert!(!check.unchanged, "{check:?}");
    assert!(!check.complete, "{check:?}");
    assert!(
        check.changed.is_empty(),
        "src/ was past the limit: {check:?}"
    );
    let sentence = check.sentence();
    assert!(!sentence.contains("no file in"), "{sentence}");
    assert!(
        sentence.contains("no change was found in the part")
            && sentence.contains("files past the first 10 were not compared")
            && sentence.contains("This is not proof that nothing changed"),
        "{sentence}"
    );
}

/// A walk out of time stops, and says so.
#[test]
fn a_walk_out_of_time_says_what_it_did_not_reach() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "a");
    let limits = FolderLimits {
        budget: Duration::ZERO,
        ..FolderLimits::default()
    };
    let check = take(dir.path(), limits).compare(&take(dir.path(), limits));
    assert!(!check.unchanged && !check.complete, "{check:?}");
    assert!(
        check.sentence().contains("took longer than 0 seconds"),
        "{}",
        check.sentence()
    );
}

/// Files `.gitignore` and `.biorouterignore` leave out are not compared (and
/// their names never reach the model).
#[test]
fn ignored_files_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".gitignore"), "results/\n");
    write(&dir.path().join(".biorouterignore"), "secret.txt\n");
    write(&dir.path().join("results/r.csv"), "1");
    write(&dir.path().join("secret.txt"), "1");
    write(&dir.path().join("notes.md"), "1");
    let before = snap(dir.path());
    write(&dir.path().join("results/r.csv"), "2");
    write(&dir.path().join("secret.txt"), "2");
    let check = before.compare(&snap(dir.path()));
    assert!(check.unchanged, "{check:?}");
    assert_eq!(
        check.files_compared, 3,
        "notes.md and the two ignore files: {check:?}"
    );
}

/// A folder that cannot be read is named and left out of the comparison, so
/// it reads neither as unchanged nor, once it becomes readable, as added
/// files.
#[cfg(unix)]
#[test]
fn an_unreadable_folder_is_named_and_never_reads_as_a_change() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("locked/a.txt"), "a");
    write(&dir.path().join("open.txt"), "o");
    let locked = dir.path().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Running as a user the mode does not stop (root): nothing to test.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let before = snap(dir.path());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let after = snap(dir.path());
    let check = before.compare(&after);
    assert!(check.added.is_empty(), "{check:?}");
    assert!(!check.unchanged && !check.complete, "{check:?}");
    let sentence = check.sentence();
    assert!(sentence.contains("locked could not be read"), "{sentence}");

    // Unreadable on both sides: still named, never "no file changed".
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let check = snap(dir.path()).compare(&snap(dir.path()));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!check.unchanged, "{check:?}");
    assert!(
        !check.sentence().contains("no file in"),
        "{}",
        check.sentence()
    );
}

/// The home folder, a disk's root and opaque system or cloud-sync trees are
/// never walked: the measured hang under `~/Library/Group Containers` is why.
#[test]
fn the_home_folder_a_root_and_opaque_trees_are_not_walked() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("Library/Caches")).unwrap();
    std::fs::create_dir_all(home.path().join("work/Group Containers/x")).unwrap();
    let canonical_home = std::fs::canonicalize(home.path()).unwrap();
    let take_from = |root: &Path| {
        FolderSnapshot::take_with(
            root,
            &[],
            Some(&canonical_home),
            None,
            FolderLimits::default(),
            &AtomicBool::new(false),
        )
    };
    for (root, expected) in [
        (canonical_home.clone(), "home folder"),
        (
            canonical_home.join("Library/Caches"),
            "system or cloud-sync",
        ),
        (
            canonical_home.join("work/Group Containers/x"),
            "system or cloud-sync",
        ),
        (PathBuf::from("/"), "root of a disk"),
    ] {
        let check = take_from(&root).compare(&take_from(&root));
        assert!(!check.unchanged, "{root:?}");
        let why = check.not_checked.clone().unwrap_or_default();
        assert!(why.contains(expected), "{root:?}: {why}");
    }

    // And an opaque tree inside an ordinary folder is pruned, not walked.
    let project = tempfile::tempdir().unwrap();
    write(&project.path().join("Mobile Documents/doc.txt"), "x");
    write(&project.path().join("notes.md"), "x");
    let check = snap(project.path()).compare(&snap(project.path()));
    assert_eq!(check.files_compared, 1, "{check:?}");
}

/// The wait for a walk is bounded: a walk that never returns is given up on,
/// the parent is told why, and a second check of the same folder is refused
/// at once rather than blocking another thread, until the first returns.
#[tokio::test]
async fn a_walk_that_does_not_return_is_given_up_on_and_not_repeated() {
    let root = tempfile::tempdir().unwrap().keep();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let blocked_root = root.clone();
    let snapshot = snapshot_bounded(root.clone(), None, Duration::ZERO, move |_stop| {
        let _ = released.recv();
        FolderSnapshot::empty(&blocked_root)
    });
    // The budget is zero, so the wait lasts only the grace period.
    let snapshot = tokio::time::timeout(FOLDER_CHECK_GRACE * 3, snapshot)
        .await
        .expect("the wait is bounded");
    let why = snapshot.not_checked.clone().unwrap_or_default();
    assert!(why.contains("did not finish within"), "{why}");

    let again = snapshot_bounded(root.clone(), None, Duration::ZERO, |_stop| {
        panic!("a second walk of a folder whose first walk is blocked must not start")
    })
    .await;
    let why = again.not_checked.clone().unwrap_or_default();
    assert!(
        why.contains("an earlier check of it has not finished"),
        "{why}"
    );

    release.send(()).unwrap();
    for _ in 0..200 {
        if !walk_presumed_blocked(&root, Duration::ZERO) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!walk_presumed_blocked(&root, Duration::ZERO));
}

/// A Stop ends the wait at once and tells the walk to stop.
#[tokio::test]
async fn a_stop_ends_the_wait_for_a_walk() {
    let root = tempfile::tempdir().unwrap().keep();
    let token = CancellationToken::new();
    token.cancel();
    let saw_stop = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&saw_stop);
    let blocked_root = root.clone();
    let snapshot = snapshot_bounded(root, Some(token), FOLDER_CHECK_BUDGET, move |stop| {
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        seen.store(true, Ordering::Relaxed);
        FolderSnapshot::empty(&blocked_root)
    })
    .await;
    let why = snapshot.not_checked.clone().unwrap_or_default();
    assert!(why.contains("the check was stopped"), "{why}");
    for _ in 0..200 {
        if saw_stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        saw_stop.load(Ordering::Relaxed),
        "the walk was told to stop"
    );
}

/// The async entry refuses the home folder without starting a walk.
#[tokio::test]
async fn the_async_check_refuses_the_home_folder_without_a_walk() {
    let Some(home) = crate::config::paths::Paths::home_dir() else {
        return;
    };
    let snapshot = snapshot_folder(home, None).await;
    let why = snapshot.not_checked.clone().unwrap_or_default();
    assert!(why.contains("home folder"), "{why}");
}
