//! The inherited cloud dictation is gone, and stays gone (DD-89, PR-01 of
//! `docs/history/local-dictation-2026-11/implementation-plan.md`).
//!
//! Until PR-01 the daemon served three routes that took recorded audio and
//! forwarded it to OpenAI's transcription API or to ElevenLabs, on the daemon
//! secret alone. Nothing in the desktop app mounted the settings that switched
//! them on, but the routes answered any caller holding the secret. While they
//! existed, no Biorouter document could say that dictated audio is not sent
//! anywhere else, so the local dictation's disclosure depends on this file.
//!
//! Two checks hold it:
//!
//! * the three paths answer 404 on the daemon's own composition, every route
//!   module merged by `routes::configure` and wrapped in `check_token` the way
//!   `commands/agent.rs` wraps it, with the correct secret, so the 404 is the
//!   router's answer and not an authentication refusal;
//! * a source scan of `crates/` and `ui/desktop/src` finds neither service's
//!   audio API address, so a route that forwards audio to either one cannot
//!   come back under another path without failing here.
//!
//! The local dictation transcribes on the host that runs the daemon and adds
//! no outbound address, so neither check needs an exception for it.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::{Path, PathBuf};

use axum::{body::Body, http::Request, http::StatusCode, middleware, Router};
use biorouter_server::auth::check_token;
use tower::ServiceExt;

const SECRET: &str = "the-daemons-secret-key";

/// The paths the deleted `routes/audio.rs` served, with their methods.
const RETIRED_ROUTES: [(&str, &str); 3] = [
    ("POST", "/audio/transcribe"),
    ("POST", "/audio/transcribe/elevenlabs"),
    ("GET", "/audio/config"),
];

/// The daemon's own composition: every route module merged by
/// `routes::configure`, wrapped in the same secret-key middleware
/// `commands/agent.rs` wraps it in.
async fn daemon() -> Router {
    let state = biorouter_server::state::AppState::new()
        .await
        .expect("app state");
    biorouter_server::routes::configure(state, SECRET.to_string()).layer(
        middleware::from_fn_with_state(SECRET.to_string(), check_token),
    )
}

fn request(method: &str, uri: &str, secret: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(secret) = secret {
        builder = builder.header("X-Secret-Key", secret);
    }
    if method == "POST" {
        // The body the deleted route accepted: base64 audio and its type. A
        // route that came back would see a well formed request, not a 400.
        builder = builder.header("content-type", "application/json");
        return builder
            .body(Body::from(
                serde_json::json!({"audio": "dGVzdA==", "mime_type": "audio/webm"}).to_string(),
            ))
            .unwrap();
    }
    builder.body(Body::empty()).unwrap()
}

async fn status_of(app: Router, request: Request<Body>) -> StatusCode {
    app.oneshot(request).await.unwrap().status()
}

#[tokio::test]
#[serial_test::serial]
async fn the_cloud_audio_routes_answer_404_on_the_daemon() {
    let app = daemon().await;

    // Not vacuous: the same composition and the same secret reach a route that
    // exists, so a 404 below is the router's answer and not a broken setup.
    assert_eq!(
        status_of(app.clone(), request("GET", "/system_info", Some(SECRET))).await,
        StatusCode::OK,
        "the composed daemon did not serve a route that exists"
    );

    for (method, path) in RETIRED_ROUTES {
        assert_eq!(
            status_of(app.clone(), request(method, path, Some(SECRET))).await,
            StatusCode::NOT_FOUND,
            "{method} {path} is served again; the cloud dictation it belonged to was deleted (DD-89)"
        );
    }
}

/// Without the secret the middleware refuses before routing, as it does for
/// every other path, so a caller without it learns nothing about these paths.
#[tokio::test]
#[serial_test::serial]
async fn the_cloud_audio_paths_are_refused_without_the_secret() {
    let app = daemon().await;
    for (method, path) in RETIRED_ROUTES {
        assert_eq!(
            status_of(app.clone(), request(method, path, None)).await,
            StatusCode::UNAUTHORIZED,
            "{method} {path} without the secret"
        );
    }
}

/// The route was never in the OpenAPI document, and nothing may put an audio
/// route there now: the generated client is how the renderer would reach one.
#[test]
fn the_openapi_document_names_no_audio_route() {
    let schema: serde_json::Value =
        serde_json::from_str(&biorouter_server::openapi::generate_schema())
            .expect("the generated schema is JSON");
    let paths = schema["paths"]
        .as_object()
        .expect("the schema has a paths object");
    assert!(
        paths.len() > 50,
        "the schema lists only {} paths; this check has stopped seeing the document",
        paths.len()
    );
    let audio: Vec<&String> = paths
        .keys()
        .filter(|path| path.starts_with("/audio"))
        .collect();
    assert!(audio.is_empty(), "audio routes in OpenAPI: {audio:?}");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the repository root resolves")
}

/// Files a URL could be written in. Binary assets are left out on purpose: the
/// desktop's staged daemon under `ui/desktop/src/bin/` is gitignored build
/// output, and one built before PR-01 still carries the deleted addresses.
const TEXT_EXTENSIONS: &[&str] = &[
    "rs", "toml", "ts", "tsx", "mts", "cts", "js", "mjs", "cjs", "jsx", "json", "ndjson", "yaml",
    "yml", "html", "css", "md", "sh", "ps1", "py", "txt",
];

/// Every text file under `dir`, skipping build output and dependency trees.
fn text_files(dir: &Path, skip: &[PathBuf], out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        let name = path
            .file_name()
            .expect("an entry has a name")
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            let excluded = name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || skip.contains(&path);
            if !excluded {
                text_files(&path, skip, out);
            }
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if TEXT_EXTENSIONS.contains(&extension.as_str()) {
            out.push(path);
        }
    }
}

/// No source under `crates/` or `ui/desktop/src` names either cloud service's
/// audio API.
///
/// The needles are assembled at run time so this file does not contain them.
/// The OpenAI one is the API's audio path segment and not the full address:
/// the deleted route joined it to a configurable host with `format!`, so the
/// full address never appeared in its source and a scan for it alone would
/// have passed while the route existed.
#[test]
fn no_source_names_a_cloud_audio_api() {
    let root = repo_root();
    let needles = [["elevenlabs", ".io"].concat(), ["/v1/", "audio"].concat()];

    let ui_src = root.join("ui").join("desktop").join("src");
    // Gitignored build output under the renderer's source tree: the staged
    // binaries and the `serve` web bundle.
    let skip = [ui_src.join("bin"), ui_src.join("web")];

    let mut files = Vec::new();
    text_files(&root.join("crates"), &skip, &mut files);
    text_files(&ui_src, &skip, &mut files);

    // Not vacuous: the scan reaches both trees, including this file.
    assert!(
        files.len() > 1000,
        "the scan found only {} files; it has stopped reading the trees",
        files.len()
    );
    for expected in [
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .expect("the crate directory resolves")
            .join("tests")
            .join("no_cloud_audio_routes.rs"),
        ui_src.join("renderer.tsx"),
    ] {
        assert!(
            files.contains(&expected),
            "the scan did not reach {}",
            expected.display()
        );
    }

    let mut found = Vec::new();
    for file in &files {
        let Ok(bytes) = std::fs::read(file) else {
            continue;
        };
        let text = bytes.to_ascii_lowercase();
        for needle in &needles {
            if text
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
            {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                found.push(format!("{} names {needle}", shown.display()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a cloud audio API is named in source; dictation is local only (DD-89):\n{}",
        found.join("\n")
    );
}
