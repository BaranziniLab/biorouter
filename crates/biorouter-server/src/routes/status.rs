use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
use biorouter::session::{generate_diagnostics, get_system_info, SystemInfo};
use std::sync::Arc;

use crate::state::AppState;

#[utoipa::path(get, path = "/status",
    responses(
        (status = 200, description = "ok", body = String),
    )
)]
async fn status() -> String {
    "ok".to_string()
}

#[utoipa::path(get, path = "/system_info",
    responses(
        (status = 200, description = "System information", body = SystemInfo),
    )
)]
async fn system_info() -> Json<SystemInfo> {
    Json(get_system_info())
}

// `GET /diagnostics/{session_id}` — the support bundle for one chat.
//
// ⚠ **This route returns the transcript.** `generate_diagnostics` writes
// `session.json` into the zip from `SessionManager::export_session`, which is
// `get_session(id, true)` — byte for byte the payload `GET /sessions/{id}` and
// `GET /sessions/{id}/export` return, both of which have been gated since
// Task 58. It ships this session's log files beside it, which carry its
// prompts. So it is a third spelling of the same read, and
// `routes::session_reach`'s own header names that shape: an unguarded sibling
// of a guarded read is the defect this campaign keeps shipping.
//
// ⚠ Deliberately `//` and not `///`: utoipa publishes a handler's doc comment
// as the operation's `description` in `ui/desktop/openapi.json`, and this is
// internal reasoning about a defect, not something to ship to every consumer of
// the spec. The sibling `session::export_session` is written the same way.
#[utoipa::path(get, path = "/diagnostics/{session_id}",
    responses(
        (status = 200, description = "Diagnostics zip file", content_type = "application/zip", body = Vec<u8>),
        (status = 403, description = "Out of reach - a private or unreadable session named without the user-action proof"),
        (status = 500, description = "Failed to generate diagnostics"),
    )
)]
async fn diagnostics(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Issue #56 Task 58 / #47. `session_id` is a request parameter, not a
    // credential; see `routes::session_reach`.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        // ⚠ NOT `refusal.into_response()`, and only because of the keyless arm —
        // see `refusal_body`.
        return (refusal.status, refusal_body(refusal, &session_id)).into_response();
    }
    match generate_diagnostics(state.session_manager(), &session_id).await {
        Ok(zip_data) => {
            let filename = format!("attachment; filename=\"diagnostics_{}.zip\"", session_id);
            let Ok(disposition) = HeaderValue::from_str(&filename) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            let response_headers = [
                (
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/zip"),
                ),
                (http::header::CONTENT_DISPOSITION, disposition),
            ];

            (response_headers, Body::from(zip_data)).into_response()
        }
        // ⚠ Say what went wrong, in the log AND in the body.
        //
        // This arm used to be `Err(_) => INTERNAL_SERVER_ERROR`: the error was
        // dropped, nothing was logged, and the response had no body. The
        // renderer's generated client turns an empty body into `{}`, which is
        // neither an `Error` nor a string, so the modal fell through to its
        // literal fallback and the user was told "Failed to generate
        // diagnostics." That is the same sentence a refusal produced, so the
        // one screen where a person is already trying to report a bug gave
        // them nothing to report, and left no trace on the daemon side either.
        //
        // `generate_diagnostics` is best-effort now, so reaching here at all
        // means something structural failed rather than one file being
        // unreadable. Which is exactly the case worth naming.
        Err(error) => {
            tracing::error!(
                session_id = %session_id,
                "failed to generate the diagnostics bundle: {error:#}"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not generate the diagnostics bundle: {error:#}"),
            )
                .into_response()
        }
    }
}
/// The refusal this route sends, which is the shared constant plus — on the
/// keyless arm only — the command that actually works where the reader is.
///
/// ## The dead end this closes
///
/// `SESSION_REACH_NO_KEY` ends "This control is unavailable on this daemon; use
/// the desktop app", and every daemon that reaches it is one where there is no
/// desktop app to use: `just run-server`, a hand-run `biorouterd agent`, a
/// systemd unit, a container, headless Linux. So a support bundle for a private
/// chat on such a box was refused with advice that cannot be followed there, and
/// there was no second sentence to fall back on.
///
/// There is a path, and it needs neither the daemon nor a key:
/// `biorouter session diagnostics --session-id <id>` calls the same
/// `generate_diagnostics` this handler does, against the session store on disk,
/// with no reach gate in front of it. Naming it turns a dead end into a
/// one-line instruction.
///
/// ## ⚠ OPERATOR DECISION OUTSTANDING — who is actually reading this
///
/// The paragraph above used to say the reader is "the person whose chat is
/// private, on a box with no GUI". That is who the sentence is *for*; it is not
/// established to be who reaches this arm, and the difference matters enough to
/// be written down rather than assumed:
///
/// * The desktop app always installs a user-action key, so the GUI never reaches
///   the keyless arm at all. ⚠ Note what that rests on, because an earlier
///   version of this comment got it wrong and said so in the word "measured":
///   the generated `diagnostics` binding IS called from the renderer
///   (`ui/desktop/src/components/ui/Diagnostics.tsx`, which imports it and
///   invokes it directly; IPC is used only to save the resulting bundle). The
///   GUI avoids this arm not because it never calls the route, but because the
///   call carries `userActionHeaders()` and this arm keys on the DAEMON holding
///   no `USER_ACTION_DIGEST` at all.
/// * So the callers that do arrive here are `curl`, scripts, and agents.
///
/// And `SESSION_OUT_OF_REACH` next door is deliberately worded to foreclose a
/// retry — "Do not retry as you are… no setting, hook or permission mode changes
/// it", pinned by a test in `session_reach.rs` — precisely because these bodies
/// are read by models. A refusal that hands the reader a working command is the
/// opposite move on the adjacent arm.
///
/// Under this project's own model the capability is unchanged: the CLI path is
/// ungated by design, the session store is an ordinary directory, and privacy
/// here is a barrier against *mistakes*, not against a determined path. So this
/// converts a stop sign into a signpost without moving the wall. Two mitigations
/// are in place and a third question is open:
///
/// 1. The sentence is addressed to the human and asks the reader to *ask* them,
///    in the same register `SESSION_OUT_OF_REACH` uses ("stop and ask the user").
///    An agent that reads and obeys it stops, exactly as on the other arm.
/// 2. It is said only on the keyless arm, which separates credential states of
///    the **caller** — a fact the caller already knows — and never states of a
///    session.
/// 3. **Open, and for a human to settle:** whether a keyless daemon should say
///    this at all, or whether the dead end is the intended behaviour of a daemon
///    deliberately started without a way to verify a person. Gating it behind an
///    explicit opt-in, or dropping it and documenting the CLI in the headless
///    guide instead, are the two alternatives. Do not treat the presence of this
///    code as that decision having been made.
///
/// ## Why the constant itself is left alone
///
/// `SESSION_REACH_NO_KEY` is shared by seven session-addressing routes, and a
/// diagnostics command is the wrong advice for six of them — it does not reply
/// to a chat, cancel its turn, export it or add an extension to it. A sentence
/// appended there would be wrong far more often than it is right, which is
/// exactly how a refusal stops being read. So the specialisation lives in the
/// handler that can be specific, and the other six keep the general wording.
///
/// ## What is deliberately NOT specialised
///
/// The `SESSION_OUT_OF_REACH` arm is returned byte for byte. That refusal is
/// answered identically for a **private** session and for one that does not
/// exist, on purpose (§14.4) — it must not become an oracle that enumerates a
/// machine's private chats one id at a time — and the CLI hint is no different
/// in that regard: this function's inputs are the refusal and the id the caller
/// already supplied, and it can therefore say nothing about the session that the
/// caller did not already know. The reason the keyless arm may be extended at
/// all is that it separates *credential states of the caller*, which is not a
/// fact about any chat.
fn refusal_body(
    refusal: crate::routes::session_reach::SessionOutOfReach,
    session_id: &str,
) -> String {
    if refusal.message != crate::routes::session_reach::SESSION_REACH_NO_KEY {
        return refusal.message.to_string();
    }
    // ⚠ Addressed to the person at the keyboard, and phrased as something to ASK
    // them for — deliberately, and in the same register `SESSION_OUT_OF_REACH`
    // uses when it ends "stop and ask the user to open it for you". The readers
    // that reach the keyless arm are scripts and agents (the desktop app always
    // installs a user-action key, so the GUI never gets here), so a body written
    // as an instruction to the *caller* would read to a model as a sanctioned way
    // around the gate it was just refused by. "Ask the person… to run" is a stop
    // for an agent and a working answer for a human, which is the only shape that
    // serves both readers of one string. See this function's OPERATOR DECISION
    // note before rewording it.
    format!(
        "{}\n\nFor this bundle there is a way that does not need the desktop app. Ask the person \
         at the keyboard to run this in a terminal on the machine running this daemon:\n\n    \
         biorouter session diagnostics --session-id {}\n\nIt reads the session store on disk \
         directly — no daemon, no user-action key — so it works for a private chat and writes \
         the same zip this route would have returned. Do not retry this request as you are; it \
         will be refused again.",
        refusal.message,
        quoted_session_id(session_id),
    )
}

/// The session id as it may appear inside a command line printed for a human to
/// **paste into a shell**.
///
/// ⚠ The id is an unvalidated path parameter, so this is the one place in the
/// refusal where an attacker-chosen string would be echoed into something the
/// reader is being invited to execute. `…/diagnostics/x%3B%20rm%20-rf%20~` is a
/// perfectly ordinary request, and a body reading `--session-id x; rm -rf ~`
/// turns this daemon into a very small social-engineering vector aimed at
/// exactly the person who is already confused enough to be reading a refusal.
///
/// Rather than quote or escape — which invites an argument about which shell —
/// an id that is not a plain identifier is not printed at all. Every session id
/// this build mints is a timestamp (`20250921_143022`), so the substitution is
/// the normal case and the placeholder is the case where the user typed
/// something odd and knows what they typed.
fn quoted_session_id(session_id: &str) -> &str {
    let plain = !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if plain {
        session_id
    } else {
        "<the session id>"
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/system_info", get(system_info))
        .route("/diagnostics/{session_id}", get(diagnostics))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::session_reach::{
        SessionOutOfReach, SESSION_OUT_OF_REACH, SESSION_REACH_NO_KEY,
    };

    fn refusal(message: &'static str) -> SessionOutOfReach {
        SessionOutOfReach {
            status: StatusCode::FORBIDDEN,
            message,
        }
    }

    /// The half the three tests below cannot see: that the handler actually
    /// *sends* what `refusal_body` composes.
    ///
    /// A source scan, and for the reason `session_reach`'s own census gives —
    /// `AppState::new()` opens the ONE session DB this binary shares, so this
    /// handler cannot be driven from a unit test. Driving it over HTTP does not
    /// help either: the keyless arm is reached only when `USER_ACTION_DIGEST` was
    /// never set, and it is a `OnceLock` that any other serial test in this crate
    /// may have already filled, so an HTTP probe would silently measure the
    /// `Unproven` arm instead and pass with this fix reverted.
    ///
    /// The negative control is what proves the extractor did not over-read into
    /// the neighbouring function and report someone else's body.
    #[test]
    fn the_diagnostics_handler_sends_the_body_this_module_composes() {
        let src = include_str!("status.rs");
        let handler = crate::routes::body_of(src, "async fn diagnostics(");
        assert!(
            handler.contains("refusal_body(refusal, &session_id)"),
            "the handler refuses without the composed body, so the keyless arm is back to \
             telling a headless box to use the desktop app"
        );
        assert!(
            !handler.contains("return refusal.into_response()"),
            "the handler returns the shared refusal verbatim again"
        );
        assert!(
            !crate::routes::body_of(src, "async fn system_info(").contains("refusal_body("),
            "the body scan is over-reading: system_info is not the diagnostics handler"
        );
    }

    /// The defect: on a daemon with no user-action key, a private chat's
    /// diagnostics were refused with "use the desktop app" — on a box that has
    /// none. The refusal has to name the command that works there, with the
    /// caller's own session id already substituted, because a person reading it
    /// is mid-way through reporting a bug and should not have to reconstruct an
    /// invocation from the CLI's help output.
    #[test]
    fn the_keyless_refusal_names_the_offline_command_with_the_session_id_in_it() {
        let body = refusal_body(refusal(SESSION_REACH_NO_KEY), "20250921_143022");
        assert!(
            body.starts_with(SESSION_REACH_NO_KEY),
            "the shared refusal must still be said in full, first: {body}"
        );
        assert!(
            body.contains("biorouter session diagnostics --session-id 20250921_143022"),
            "the refusal does not name the command that works on this machine: {body}"
        );
    }

    /// ⚠ The mitigation the OPERATOR DECISION note rests on, as an assertion
    /// rather than a comment.
    ///
    /// The callers that reach the keyless arm are scripts and agents — the
    /// desktop app always installs a user-action key, so its own call (which the
    /// renderer does make, carrying `userActionHeaders()`) never lands here —
    /// so this body is read by a model far more often than by a person. The
    /// arm next door, `SESSION_OUT_OF_REACH`, is worded to foreclose a retry for
    /// exactly that reason. Handing a model a working command instead would be
    /// the opposite move on the adjacent arm, so the hint is addressed to the
    /// **person at the keyboard**, asks the reader to *ask* them, and repeats the
    /// no-retry instruction. A rewording that drops any of the three should fail
    /// here and be argued for, not slip through as a tidy-up.
    #[test]
    fn the_offline_hint_is_addressed_to_the_human_and_still_forecloses_a_retry() {
        let body = refusal_body(refusal(SESSION_REACH_NO_KEY), "20250921_143022");
        assert!(
            body.contains("Ask the person at the keyboard to run"),
            "the hint reads as an instruction to the caller rather than a request \
             to the user, which to a model is a sanctioned way around the gate it \
             was just refused by: {body}"
        );
        assert!(
            body.contains("Do not retry this request as you are"),
            "the keyless arm dropped the no-retry instruction that the \
             out-of-reach arm beside it is pinned to: {body}"
        );
    }

    /// The other arm is byte-identical to the shared constant, and that is a
    /// property rather than an omission: `SESSION_OUT_OF_REACH` is answered the
    /// same way for a private chat and for one that does not exist so that the
    /// refusal cannot be used to enumerate a machine's private chats, and this
    /// route is not the place to start decorating it.
    #[test]
    fn the_out_of_reach_refusal_is_passed_through_untouched() {
        assert_eq!(
            refusal_body(refusal(SESSION_OUT_OF_REACH), "20250921_143022"),
            SESSION_OUT_OF_REACH,
        );
    }

    /// The id is an unvalidated path parameter and the body invites the reader
    /// to paste a command into a shell, so an id that is not a plain identifier
    /// is replaced rather than quoted. Without this, `GET /diagnostics/x;%20rm%20-rf%20~`
    /// would have the daemon print `--session-id x; rm -rf ~` to a confused user.
    #[test]
    fn a_session_id_that_is_not_a_plain_identifier_is_never_pasted_into_the_command() {
        for hostile in [
            "x; rm -rf ~",
            "x`whoami`",
            "x$(id)",
            "x\nrm -rf ~",
            "x'y",
            "",
        ] {
            let body = refusal_body(refusal(SESSION_REACH_NO_KEY), hostile);
            assert!(
                body.contains("--session-id <the session id>"),
                "a hostile id reached the command line: {body}"
            );
            // Newline-terminated, because the command is the last thing on its
            // line: without it the empty-id case matches the placeholder's own
            // `--session-id ` prefix and this assertion accuses the fix of a
            // failure it did not have.
            assert!(
                !body.contains(&format!("--session-id {hostile}\n")),
                "a hostile id reached the command line: {body}"
            );
        }
        // …and the ordinary shapes still substitute, or the guard has simply
        // turned the feature off.
        for ordinary in ["20250921_143022", "my-chat.1", "abc123"] {
            assert!(
                refusal_body(refusal(SESSION_REACH_NO_KEY), ordinary)
                    .contains(&format!("--session-id {ordinary}")),
                "an ordinary session id was withheld from the command"
            );
        }
    }
}
