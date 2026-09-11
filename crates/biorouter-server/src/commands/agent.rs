use crate::configuration;
use crate::state;
use anyhow::Result;
use axum::middleware;
use biorouter_server::auth::check_token;
use http::HeaderValue;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::info;

// Graceful shutdown signal
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = sigint.recv() => {},
        _ = sigterm.recv() => {},
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Read the launcher's SHA-256 user-action digest off stdin, as one hex line
/// (issue #56, DR-16).
///
/// The pipe is at EOF once this returns, so every process the daemon later
/// spawns inherits an fd 0 that carries nothing: the digest is not re-readable
/// by a child, and the raw key was never there to begin with.
///
/// It must **never block a hand-started daemon**, so it is guarded twice.
async fn read_user_action_digest() -> Option<[u8; 32]> {
    use std::io::IsTerminal;
    // (1) A terminal is a human at a prompt, not a launcher with a key. Reading
    //     it would hang `just run-server` forever waiting for a line.
    if std::io::stdin().is_terminal() {
        return None;
    }
    // (2) And a pipe whose writer never closes would hang just as hard, so the
    //     read is bounded. 2s is far longer than a local `write` + `end`.
    //
    //     On a PLAIN OS THREAD rather than `spawn_blocking`, because a blocking
    //     read cannot be cancelled: on the timeout path the reader stays parked
    //     in `read_line`, holding `stdin().lock()`, for the life of the process.
    //     A tokio runtime waits for started blocking tasks when it drops, so a
    //     launcher that opens the pipe and never closes it would convert this
    //     bound from a 2s delay at startup into a hang at SHUTDOWN — the exact
    //     case this guard exists for. A detached thread is not the runtime's to
    //     wait on.
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)
            .ok()
            .map(|_| line);
        // The receiver is gone on the timeout path; nothing to report to.
        let _ = tx.send(read);
    });
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
        .await
        .ok()?
        .ok()??;
    let bytes = hex::decode(line.trim()).ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

/// The tier SD-1 pins for every session a serve daemon runs: the DECLARED tier
/// of the provider the operator configured, reduced with `least` over the lead
/// provider when a lead model is configured — the reduction a bound lead/worker
/// pair gets, since its transcript reaches both.
///
/// Read ONCE, at launch. The operator made this choice at the terminal before
/// anyone opened a tab (SD-1), and `config.yaml` is agent-writable (DR-17), so a
/// value re-read per request would be one a model could raise by editing a file.
/// Unconfigured, and a name this install does not publish, both read Public —
/// the fail-safe side, and the reach this interface had for every private chat
/// before it had any.
async fn served_operator_capability() -> biorouter::privacy::ProviderTier {
    use biorouter::privacy::ProviderTier;
    use biorouter::workflow::privacy::declared_provider_tier;
    let config = biorouter::config::Config::global();
    let Ok(provider) = config.get_biorouter_provider() else {
        return ProviderTier::Public;
    };
    let mut capability = declared_provider_tier(&provider).await;
    if config.get_param::<String>("BIOROUTER_LEAD_MODEL").is_ok() {
        let lead = config
            .get_param::<String>("BIOROUTER_LEAD_PROVIDER")
            .unwrap_or_else(|_| provider.clone());
        capability = ProviderTier::least(capability, declared_provider_tier(&lead).await);
    }
    capability
}

pub async fn run() -> Result<()> {
    crate::logging::setup_logging(Some("biorouterd"))?;

    let settings = configuration::Settings::new()?;

    // Issue #56 Task 30, hardening measure (3): the master privacy switch is
    // read ONCE, here, and the authoritative value then lives in daemon memory
    // for the life of the process. The FIRST of the toggle's exactly two
    // writers; the second is `/config/upsert`'s gated arm, which is the channel
    // Settings > Privacy uses.
    //
    // ⚠ Task 42 (DR-22): the value comes from `privacy-tiers.json` beside
    // `config.yaml`, NOT from `config.yaml` itself — a key in that file was a
    // next-launch disable for anything that could write files. This call also
    // runs the one-time migration that carries a pre-DR-22 key across and
    // retires it, which is why it must stay before anything that could serve a
    // request.
    //
    // It is deliberately not read through `Config::get_param`, whose middle
    // branch resolves an environment variable: the agent holds
    // `developer__shell`, so an env-readable value would make
    // `BIOROUTER_PRIVACY_TIERS=off biorouterd` — or a line in the user's shell
    // profile — a one-token disable of the control the agent is subject to. See
    // `biorouter::privacy::load_privacy_tiers_from_config`.
    //
    // Before `AppState::new()` and before any route is mounted, so no request
    // can be served against the fail-safe default when the user turned the
    // feature off, and none against `off` when they did not.
    biorouter::privacy::load_privacy_tiers_from_config();
    // Issue #56 Task 52 (DR-27), and here for the same three reasons: it reads
    // its own record beside `config.yaml` rather than the agent-writable file,
    // it is never resolved through an environment variable, and it lands before
    // any route is mounted so no request is served against `standard` on a
    // machine whose user chose `strict`.
    biorouter::privacy::load_mixing_policy_from_record();

    let secret_key = std::env::var("BIOROUTER_SERVER__SECRET_KEY").unwrap_or_else(|_| {
        let bytes: [u8; 16] = rand::random();
        let key = hex::encode(bytes);
        tracing::warn!(
            "BIOROUTER_SERVER__SECRET_KEY not set; using randomly generated key for this session"
        );
        key
    });

    // Issue #56 DR-16. The proof-of-user, minted by whoever launched this
    // daemon and handed over on **stdin** — never in the environment and never
    // on argv, because AR-11 measured both to be recoverable in-process by any
    // tool that reads a caller-named path (`/proc/self/environ`) or, on macOS,
    // by `sysctl(KERN_PROCARGS2)`, which is not a path at all and which no
    // sandbox profile can gate.
    let user_action_digest = read_user_action_digest().await;
    if user_action_digest.is_none() {
        tracing::warn!(
            "no user-action key on stdin: this daemon will refuse every request that raises a \
             session's privacy capability, including one made by the person at the keyboard"
        );
    }
    // A tool whose approval can never be granted must not be offered. `serve`
    // spawns this daemon with `Stdio::null()`, so it holds no key and every
    // proof-backed approval refuses forever — the install and delete tools take
    // themselves off the surface rather than letting the model propose something
    // whose card has three buttons that cannot work.
    biorouter::pending_user_action::set_user_proof_available(user_action_digest.is_some());
    biorouter_server::auth::install_user_action_digest(user_action_digest);

    let app_state = state::AppState::new().await?;

    // BR-71: publish the daemon's platform services to the `biorouter` crate so
    // the workspace extension's tools can reach the turn lock, the detached turn
    // runner and (Slice 2) the GUI bridge. Without this the tools degrade to
    // their headless behaviour even inside the daemon.
    crate::workspace::services::install_workspace_services(app_state.clone());

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            biorouter_server::routes::is_local_origin(origin.to_str().unwrap_or(""))
        }))
        .allow_methods(Any)
        .allow_headers(Any);

    let app = crate::routes::configure(app_state.clone(), secret_key.clone())
        .layer(middleware::from_fn_with_state(
            secret_key.clone(),
            check_token,
        ))
        .layer(cors);

    // The web interface, when this daemon was asked to serve one. It is added
    // AFTER `check_token` on purpose: `Router::layer` wraps only what was added
    // before it, so the shell and the static bundle sit structurally outside
    // that middleware rather than being exempted from it by path. See
    // `routes::web_ui` for what gates them instead, and
    // `docs/deployment/serve-architecture.md` for why the daemon serves them at
    // all rather than a separate binary proxying to it.
    let app = match settings.serve_ui.as_deref() {
        Some(dir) => {
            let web_dir = std::path::PathBuf::from(dir);
            // A browser token is minted by whoever launched this daemon --
            // `biorouter serve` -- and refused outright for a non-loopback bind
            // there, so its absence here means a loopback bind whose launcher
            // chose not to require one.
            let browser_token = std::env::var("BIOROUTER_BROWSER_TOKEN").ok();
            // Issue #56, QA 2026-09-10 (SD-10): the interface this daemon serves is
            // the operator's, and SD-1 pins the provider every session here runs
            // on — so that provider's tier is the reach the listing and
            // knowledge-base gates give a request carrying the served document's
            // cookie. Without a token there is no such cookie, and the interface
            // cannot be told from any other local caller, so it gets none.
            if let Some(token) = browser_token.as_deref().filter(|t| !t.is_empty()) {
                let capability = served_operator_capability().await;
                info!(
                    ?capability,
                    "the served interface is given the configured provider's tier on listings \
                     and knowledge bases"
                );
                biorouter_server::auth::install_served_operator(token.to_string(), capability);
            }
            let ui = crate::routes::web_ui::WebUi::new(&web_dir, &secret_key, browser_token)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "could not read the web interface at {}: {e}",
                        web_dir.display()
                    )
                })?;
            info!("serving the web interface from {}", web_dir.display());
            crate::routes::web_ui::attach(app, web_dir, ui)
        }
        None => app,
    };

    // gzip large JSON payloads (config/providers/tools/session bodies), and the
    // interface bundle when one is served. The default predicate skips small
    // bodies and `text/event-stream`, so the streaming `/reply` SSE response is
    // left unbuffered/uncompressed. Outermost, so it covers the interface too.
    let app = app.layer(CompressionLayer::new());

    let listener = tokio::net::TcpListener::bind(settings.socket_addr()).await?;
    let local_addr = listener.local_addr()?;
    info!("listening on {}", local_addr);
    // Publish the base URL so in-process MCP tools (e.g. Agent Drafter's
    // `launch_app`) can emit absolute http://host:port/apps/<id>/ URLs.
    std::env::set_var("BIOROUTER_APP_BASE_URL", format!("http://{local_addr}"));
    // Publish the same base to the coding-agent tool bridge, so a bridged child
    // (`claude`, `codex`) can be handed an absolute URL for the session's tools.
    // Until this is set no grant can be issued at all, which is the correct
    // behaviour in a CLI process with no HTTP server: there would be nothing for a
    // child to connect to, and the providers then run tool-less rather than failing.
    biorouter::providers::coding_agent::bridge::publish_base_url(format!("http://{local_addr}"));

    let tunnel_manager = app_state.tunnel_manager.clone();
    tokio::spawn(async move {
        tunnel_manager.check_auto_start().await;
    });

    // Issue #112. Watch `config.yaml` for extension changes made outside this
    // process — a `biorouter extension install` run in another terminal, a deep
    // link, a hand edit. The daemon's own writes announce themselves through
    // the config choke points; this is the only thing that can see the others,
    // and without it a running app needs a restart to notice them.
    biorouter::catalog::spawn_config_watcher();

    // `into_make_service_with_connect_info` is what puts the real peer address
    // in request extensions, so the auth throttle can key on it instead of the
    // client-supplied `x-forwarded-for` header.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Take the llama-server sidecar down with us.
    //
    // The sidecar is a child process owned by a `OnceLock` static, and statics
    // never drop — so tokio's `kill_on_drop` cannot cover process exit, and a
    // llama-server holding 5-20 GB of weights and KV cache outlived every app
    // quit. Nothing reclaimed it until the NEXT `ensure()` in some future
    // Biorouter process ran `reap_orphans`, which meant quitting the app left
    // the memory pinned indefinitely.
    //
    // This is the graceful path only, and that is fine: the pidfile written at
    // spawn is what covers SIGKILL, a panic, or a crash, and `reap_orphans`
    // still collects those on the next launch. This just stops the common case
    // — an ordinary quit — from relying on that later sweep.
    biorouter::providers::llamacpp_sidecar::global()
        .stop()
        .await;

    info!("server shutdown complete");
    Ok(())
}
