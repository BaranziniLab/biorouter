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
//! An opening path runs its work inside [`opening`]. Inside that scope the
//! **setup** writes leave `updated_at` alone:
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
    static OPENING: ();
}

/// Run `f` as part of opening a chat: the setup writes beneath it do not move
/// the chat's activity time. See the module documentation for which writes
/// those are, and why nothing else is affected.
pub async fn opening<F>(f: F) -> F::Output
where
    F: std::future::Future,
{
    OPENING.scope((), f).await
}

/// Does a setup write made right now count as activity in its chat?
///
/// `true` outside any [`opening`] scope, which is the pre-R-30 behaviour.
pub(crate) fn setup_write_is_activity() -> bool {
    OPENING.try_with(|_| ()).is_err()
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
