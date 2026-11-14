//! Updating Biorouter from a `biorouter serve` browser page (SD-19).
//!
//! Three routes beside the other `/headless/*` ones, and like them reached by
//! the renderer's browser bridge with hand-written `fetch` calls rather than the
//! generated client, so they carry no `utoipa` annotation:
//!
//! - `GET /headless/update` — what is installed, what is offered, and how far an
//!   update has got. The secret is enough: reading it changes nothing.
//! - `POST /headless/update/check` — look for a release now.
//! - `POST /headless/update/install` — install the downloaded release (or, when
//!   a newer one is already on disk, only restart) and exit for `serve` to
//!   restart this daemon on it.
//!
//! ⚠ **The two POSTs are the person's, never an agent's.** Each must be the
//! person in the served page, the proof SD-18 set up for providers
//! (`auth::served_person_present`: the served document's cookie, and Fetch
//! Metadata saying that page's own script sent it), on top of the secret
//! `check_token` already demanded: a chat's model holds `developer__shell` and
//! can recover the secret, and the secret alone must not restart the server out
//! from under everyone using it. A `--no-token` daemon has no such cookie, and
//! there the Fetch Metadata alone stands in, which is as open as everything
//! else on a daemon that chose no token.
//!
//! ⚠ **A daemon that `serve` did not start with the restart contract offers
//! nothing here** (`biorouter::serve_update::SUPERVISOR_ENV`): the desktop app's
//! own daemon updates through the app, and a daemon nothing would restart must
//! never exit asking to be.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use biorouter::serve_update::updater::{self, Installed, Status};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// How long the install answer has to reach the browser before the daemon
/// starts shutting down.
const EXIT_AFTER_ANSWER: Duration = Duration::from_millis(400);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    /// False on a daemon that cannot update itself from the browser.
    available: bool,
    /// The version running, said even when `available` is false.
    version: &'static str,
    #[serde(flatten)]
    status: Option<Status>,
    /// Chats with a turn in flight, which a restart stops.
    running_turns: usize,
    /// Background shell jobs and subagents still running, which it stops too.
    running_work: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallRequest {
    /// The person saw that chats are still working and chose to stop them.
    #[serde(default)]
    stop_running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Refusal {
    reason: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    running_turns: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    running_work: Option<usize>,
}

fn refuse(status: StatusCode, reason: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(Refusal {
            reason,
            message: message.into(),
            running_turns: None,
            running_work: None,
        }),
    )
        .into_response()
}

/// Is this request the person in this daemon's browser page? See the module
/// documentation for why the secret is not enough.
pub(crate) fn from_the_page(headers: &HeaderMap) -> bool {
    page_verdict(
        biorouter_server::auth::serves_a_tokened_interface(),
        biorouter_server::auth::served_person_present(headers),
        headers,
    )
}

/// [`from_the_page`], with the process-global answers passed in so the rule is
/// testable. A tokened daemon goes by `person_present` alone (the cookie and the
/// page script's Fetch Metadata together); Fetch Metadata never stands in for a
/// token that exists.
fn page_verdict(tokened: bool, person_present: bool, headers: &HeaderMap) -> bool {
    if tokened {
        person_present
    } else {
        biorouter_server::auth::sent_by_same_origin_page_script(headers)
    }
}

const NOT_FROM_THE_PAGE: &str =
    "Only the person using Biorouter's browser page can update or restart it. This request \
     carries the daemon's secret but did not come from that page, so it was refused.";

/// Background work a restart would stop: shell jobs and subagents.
fn running_work() -> usize {
    biorouter_mcp::active_work::active_work().list().len()
}

/// What a refusal for running work says, in one line.
fn still_running(turns: usize, work: usize) -> String {
    let chats = match turns {
        0 => None,
        1 => Some("1 chat".to_string()),
        n => Some(format!("{n} chats")),
    };
    let jobs = match work {
        0 => None,
        1 => Some("1 background job".to_string()),
        n => Some(format!("{n} background jobs")),
    };
    let what = match (chats, jobs) {
        (Some(c), Some(j)) => format!("{c} and {j}"),
        (Some(one), None) | (None, Some(one)) => one,
        (None, None) => "Nothing".to_string(),
    };
    let them = if turns + work == 1 { "it" } else { "them" };
    format!("{what} still running, and restarting would stop {them}.")
}

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusBody> {
    let running_turns = state.active_turn_session_ids().len();
    let running_work = running_work();
    match updater::global() {
        Some(updater) => Json(StatusBody {
            available: true,
            version: biorouter::serve_update::running_version(),
            status: Some(updater.status().await),
            running_turns,
            running_work,
        }),
        None => Json(StatusBody {
            available: false,
            version: biorouter::serve_update::running_version(),
            status: None,
            running_turns,
            running_work,
        }),
    }
}

async fn check(headers: HeaderMap) -> Response {
    if !from_the_page(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "not_from_the_page",
            NOT_FROM_THE_PAGE,
        );
    }
    let Some(updater) = updater::global() else {
        return refuse(
            StatusCode::NOT_FOUND,
            "unavailable",
            "This Biorouter cannot update itself from the browser.",
        );
    };
    // A download can take minutes; the page follows it through the status.
    tokio::spawn(async move { updater.check().await });
    StatusCode::ACCEPTED.into_response()
}

async fn install(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Option<Json<InstallRequest>>,
) -> Response {
    if !from_the_page(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "not_from_the_page",
            NOT_FROM_THE_PAGE,
        );
    }
    let Some(updater) = updater::global() else {
        return refuse(
            StatusCode::NOT_FOUND,
            "unavailable",
            "This Biorouter cannot update itself from the browser.",
        );
    };
    if !updater.installable() {
        return refuse(
            StatusCode::CONFLICT,
            "not_ready",
            "There is no downloaded update to install yet.",
        );
    }
    let request = body.map(|Json(request)| request).unwrap_or_default();
    let turns = state.active_turn_session_ids().len();
    let work = running_work();
    if turns + work > 0 && !request.stop_running {
        return (
            StatusCode::CONFLICT,
            Json(Refusal {
                reason: "turns_running",
                message: still_running(turns, work),
                running_turns: Some(turns),
                running_work: Some(work),
            }),
        )
            .into_response();
    }
    // The install itself can take a minute (a package manager); the page
    // follows it through the status, and finds the daemon gone once it restarts.
    tokio::spawn(async move {
        match updater.install().await {
            Ok(Installed::RestartNow) => {
                tokio::time::sleep(EXIT_AFTER_ANSWER).await;
                updater::request_restart();
                tracing::info!("serve update installed; stopping so serve restarts on it");
                biorouter_server::daemon_service::shutdown_token().cancel();
            }
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "serve update was not installed")
            }
        }
    });
    StatusCode::ACCEPTED.into_response()
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/headless/update", get(status))
        .route("/headless/update/check", post(check))
        .route("/headless/update/install", post(install))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn running_work_is_said_in_one_line() {
        assert_eq!(
            still_running(1, 0),
            "1 chat still running, and restarting would stop it."
        );
        assert_eq!(
            still_running(2, 1),
            "2 chats and 1 background job still running, and restarting would stop them."
        );
        assert_eq!(
            still_running(0, 3),
            "3 background jobs still running, and restarting would stop them."
        );
    }

    fn page_script() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
        headers
    }

    #[test]
    fn a_tokened_daemon_goes_by_the_person_in_the_page_alone() {
        // The secret and a browser's headers, but not the person in the page (no
        // cookie): an agent's curl can send the headers, so it is refused.
        assert!(!page_verdict(true, false, &page_script()));
        assert!(page_verdict(true, true, &HeaderMap::new()));
    }

    #[test]
    fn without_a_tokened_document_only_the_page_s_own_script_counts() {
        assert!(
            !page_verdict(false, false, &HeaderMap::new()),
            "a bare request with the secret is not the page"
        );
        let mut navigation = page_script();
        navigation.insert("sec-fetch-mode", HeaderValue::from_static("navigate"));
        assert!(!page_verdict(false, false, &navigation));
        let mut cross = page_script();
        cross.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!page_verdict(false, false, &cross));
        assert!(page_verdict(false, false, &page_script()));
    }
}
