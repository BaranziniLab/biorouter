//! Opening a chat is not activity in it (R-30).
//!
//! `sessions.updated_at` is the chat's **activity** time: the sidebar's "Last
//! activity" order, History's date buckets, `GET /sessions`, the CLI's
//! `session list`, and `--resume` with no id all sort by it. A chat moves when
//! something happens IN it: a message is stored, a turn runs (its token
//! accounting stamps as it goes), the conversation is rewritten or truncated, or
//! the person renames it.
//!
//! Opening a chat is none of those, and it used to move the chat to the top
//! anyway. Opening has automatic side effects that write the row: a cold agent
//! rebinds the model the row records (`Agent::restore_provider_from_session`),
//! the CLI saves the extension roster it loaded, ACP applies the client's
//! directory to an empty chat. Each of those writes stamped `updated_at`, so the
//! first open of a chat after every daemon start reordered every list that sorts
//! by activity.
//!
//! An opening path runs its work inside [`opening`], and machine-wide
//! housekeeping that sweeps many chats at once runs inside [`housekeeping`]
//! (uninstalling an extension prunes it from every saved roster that names it:
//! a cleanup the person did to the machine, not to any one chat). The two
//! scopes are the same scope under two names, so a door says which it is.
//! Inside either one the **setup** writes leave `updated_at` alone:
//!
//! * the provider bind (`SessionStorage::bind_provider_if_allowed`);
//! * the extension-data write (`SessionStorage::update_extension_data`, which
//!   backs `update_extension_state`);
//! * a `SessionUpdateBuilder::apply` that sets `extension_data` and nothing
//!   else (an app's connect re-installs its goal guardrail this way);
//! * the empty-chat working-directory write
//!   (`SessionStorage::try_update_working_dir_if_empty`).
//!
//! ⚠ **Nothing else reads the scope, on purpose.** A message insert, a
//! conversation rewrite, a truncation, the token ledger and every other
//! `SessionUpdateBuilder::apply` (a rename above all, even one that also sets
//! `extension_data`) stamp whether or not they run inside it. A rename is
//! activity even while a chat is being opened, and a scope that grew by mistake
//! to cover a turn must not stop that turn from moving its chat.
//!
//! ⚠ **The default is "this write is activity".** Outside a scope every write
//! stamps, exactly as before R-30. A forgotten scope costs a chat that jumps on
//! open (the old bug, visible and easy to report), never a chat that stays put
//! while a turn runs in it.
//!
//! What stays activity is a deliberate change made TO a chat (coordinator ruling
//! 16): a message or a turn, a rename, an explicit switch to a model the chat
//! does not already record, toggling its extensions or skills from its composer,
//! `workspace_set_tools` aimed at it, and a Crew grant on it. None of those
//! doors enters a scope, and a test in this module counts every door that does,
//! so wrapping one of them fails a test rather than quietly freezing a chat.
//!
//! One rule is not a scope at all, because it holds at every door: the provider
//! bind stamps only when it changes what the chat records (a different provider,
//! or a different `model_name`). Re-binding the model a chat already records is
//! not a change to it, whether a cold open restores it, a person picks it again,
//! or a `--shared-daemon` resume names it.
//!
//! It is a `task_local`, shaped after [`crate::user_surface`], because the
//! decision belongs to the DOOR (the route, the CLI builder, the ACP handler)
//! and the write sits several calls below it, behind `Agent::update_provider`.
//! Passing a parameter down instead would add a second public name for the
//! provider bind, and the privacy guards that count `.update_provider(` call
//! sites (`routes/apps.rs`, the wiring census) would not see it. The scope
//! follows the future across `.await`s but not into a `tokio::spawn`, so every
//! write it covers must be awaited inline by its door.
//!
//! The change feed (`GET /sessions/changes`, [`crate::session_meta`]) diffs the
//! binding and classification columns and never `updated_at`, so a restore that
//! does change the binding is still reported while the chat keeps its place.

use tokio::task_local;

task_local! {
    static NOT_ACTIVITY: ();
}

/// Run `f` as part of opening a chat: the setup writes beneath it do not move
/// the chat's activity time. See the module documentation for which writes
/// those are, and why nothing else is affected.
pub async fn opening<F>(f: F) -> F::Output
where
    F: std::future::Future,
{
    NOT_ACTIVITY.scope((), f).await
}

/// Run `f` as machine-wide housekeeping: a sweep that touches many chats at
/// once because of something done to the machine (uninstalling an extension
/// prunes it from every saved roster), not to any one chat. The setup writes
/// beneath it do not move those chats, exactly as inside [`opening`].
pub async fn housekeeping<F>(f: F) -> F::Output
where
    F: std::future::Future,
{
    NOT_ACTIVITY.scope((), f).await
}

/// Does a setup write made right now count as activity in its chat?
///
/// `true` outside any [`opening`] or [`housekeeping`] scope, which is the
/// pre-R-30 behaviour.
pub(crate) fn setup_write_is_activity() -> bool {
    NOT_ACTIVITY.try_with(|_| ()).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_write_outside_any_scope_is_activity() {
        // The direction of the default is the safety property: a forgotten
        // scope must leave a chat that jumps, never a chat that stays put.
        assert!(setup_write_is_activity());
    }

    #[tokio::test]
    async fn the_scope_covers_everything_awaited_inside_it_and_ends_with_it() {
        opening(async {
            assert!(!setup_write_is_activity());
            tokio::task::yield_now().await;
            assert!(!setup_write_is_activity());
            // Nested scopes are allowed (a route that opens, then a restore
            // that opens again) and stay inside.
            opening(async { assert!(!setup_write_is_activity()) }).await;
            assert!(!setup_write_is_activity());
        })
        .await;
        assert!(setup_write_is_activity());
    }

    #[tokio::test]
    async fn housekeeping_is_the_same_scope_under_its_own_name() {
        assert!(housekeeping(async { !setup_write_is_activity() }).await);
        assert!(opening(housekeeping(async { !setup_write_is_activity() })).await);
        assert!(setup_write_is_activity());
    }

    /// Every production door that enters a scope, by file (ruling 16).
    ///
    /// A deliberate change to a chat is activity: an explicit model switch
    /// (`/agent/update_provider`), a composer extension or skill toggle
    /// (`/agent/add_extension`, `/agent/remove_extension`, `/skills/session`),
    /// `workspace_set_tools` aimed at it and a Crew grant on it. Their writes
    /// stamp because none of them runs inside a scope, so wrapping one of them
    /// would freeze a chat the person just changed while every other test
    /// stayed green. This census is what fails instead. A new opening door, or
    /// a new machine-wide sweep, adds its row here on purpose.
    #[test]
    fn only_the_opening_and_housekeeping_doors_enter_a_scope() {
        use std::collections::BTreeMap;
        use std::path::{Path, PathBuf};

        fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }

        let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/")
            .to_path_buf();
        let needles = [
            format!("activity::{}(", "opening"),
            format!("activity::{}(", "housekeeping"),
        ];
        let mut found: BTreeMap<String, [usize; 2]> = BTreeMap::new();
        let mut files = Vec::new();
        for krate in std::fs::read_dir(&crates)
            .expect("crates/ is readable")
            .flatten()
        {
            rust_files(&krate.path().join("src"), &mut files);
        }
        for file in files {
            if file.ends_with("session/activity.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&file).expect("source is readable");
            let mut counts = [0_usize; 2];
            for line in source.lines() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for (count, needle) in counts.iter_mut().zip(&needles) {
                    *count += line.matches(needle.as_str()).count();
                }
            }
            if counts != [0, 0] {
                let relative = file
                    .strip_prefix(&crates)
                    .expect("under crates/")
                    .to_string_lossy()
                    .replace('\\', "/");
                found.insert(relative, counts);
            }
        }

        // [opening, housekeeping]
        let expected: BTreeMap<String, [usize; 2]> = [
            // A cold agent restores the model the chat records.
            ("biorouter/src/agents/agent.rs", [1, 0]),
            // Uninstalling an extension prunes every saved roster.
            (
                "biorouter/src/agents/extension_manager_extension.rs",
                [0, 1],
            ),
            // ACP binds on new/load and applies the load-time directory.
            ("biorouter-acp/src/server.rs", [2, 0]),
            // The CLI resume's bind and its roster save.
            ("biorouter-cli/src/session/builder.rs", [2, 0]),
            // `POST /agent/resume`.
            ("biorouter-server/src/routes/agent.rs", [1, 0]),
            // An app page's connect.
            ("biorouter-server/src/routes/apps.rs", [1, 0]),
        ]
        .into_iter()
        .map(|(file, counts)| (file.to_owned(), counts))
        .collect();
        assert_eq!(
            found, expected,
            "the doors that enter an activity scope changed. A deliberate change to a chat \
             (a model switch, a composer toggle, workspace_set_tools, a Crew grant) must stay \
             activity and never run inside one (ruling 16)"
        );
    }

    #[tokio::test]
    async fn a_spawned_task_does_not_inherit_the_scope() {
        // Pinned so nobody relies on it: a door must await its setup writes
        // inline, or they stamp.
        let inherited = opening(async {
            tokio::spawn(async { setup_write_is_activity() })
                .await
                .unwrap()
        })
        .await;
        assert!(inherited);
    }
}
