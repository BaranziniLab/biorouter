pub mod lapstone;
pub mod ws_connect;

#[cfg(test)]
mod lapstone_test;

use crate::configuration::Settings;
use biorouter::config::{paths::Paths, Config};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use utoipa::ToSchema;

fn get_server_port() -> anyhow::Result<u16> {
    let settings = Settings::new()?;
    Ok(settings.port)
}

fn get_lock_path() -> std::path::PathBuf {
    Paths::config_dir().join("tunnel.lock")
}

fn try_acquire_tunnel_lock() -> anyhow::Result<File> {
    let lock_path = get_lock_path();

    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)?;

    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("Another biorouter instance is already running the tunnel"))?;

    writeln!(file, "{}", std::process::id())?;
    file.sync_all()?;

    Ok(file)
}

fn is_tunnel_locked_by_another() -> bool {
    let lock_path = get_lock_path();

    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return false,
    };

    if file.try_lock_exclusive().is_err() {
        return true;
    }

    // Lock released when file is dropped
    false
}

/// A connection that stayed up this long was healthy, so the next redial is
/// immediate again. Above `lapstone`'s 30 s connect timeout, so a connect that
/// hangs until it times out still counts as a failure.
const HEALTHY_CONNECTION: Duration = Duration::from_secs(60);
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);

/// How long the watchdog waits before redialling, from its previous wait and how
/// long the connection that just ended lived. Without it, a relay that refuses
/// the websocket was redialled about 5 to 9 times a second for the daemon's
/// whole life, each attempt reloading the platform root store: about a quarter of a
/// core, idle, long after the app had quit.
fn next_retry_delay(previous: Duration, lived: Duration) -> Duration {
    if lived >= HEALTHY_CONNECTION {
        Duration::ZERO
    } else if previous.is_zero() {
        FIRST_RETRY_DELAY
    } else {
        (previous * 2).min(MAX_RETRY_DELAY)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TunnelState {
    #[default]
    Idle,
    Starting,
    Running,
    Error,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TunnelInfo {
    pub state: TunnelState,
    pub url: String,
    pub hostname: String,
    pub secret: String,
}

pub struct TunnelManager {
    state: Arc<RwLock<TunnelState>>,
    info: Arc<RwLock<Option<TunnelInfo>>>,
    lapstone_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    restart_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    watchdog_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    lock_file: Arc<std::sync::Mutex<Option<File>>>,
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelManager {
    pub fn new() -> Self {
        TunnelManager {
            state: Arc::new(RwLock::new(TunnelState::Idle)),
            info: Arc::new(RwLock::new(None)),
            lapstone_handle: Arc::new(RwLock::new(None)),
            restart_tx: Arc::new(RwLock::new(None)),
            watchdog_handle: Arc::new(RwLock::new(None)),
            lock_file: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn get_auto_start() -> bool {
        Config::global()
            .get_param("tunnel_auto_start")
            .unwrap_or(false)
    }

    fn get_secret() -> Option<String> {
        Config::global().get_secret("tunnel_secret").ok()
    }

    fn get_agent_id() -> Option<String> {
        Config::global().get_secret("tunnel_agent_id").ok()
    }

    pub async fn check_auto_start(&self) {
        let auto_start = Self::get_auto_start();
        let state = self.state.read().await.clone();

        if auto_start && state == TunnelState::Idle {
            if is_tunnel_locked_by_another() {
                tracing::info!(
                    "Tunnel already running on another biorouter instance, skipping auto-start"
                );
                return;
            }

            tracing::info!("Auto-starting tunnel");
            match self.start().await {
                Ok(info) => {
                    tracing::info!("Tunnel auto-started successfully: {:?}", info.url);
                }
                Err(e) => {
                    tracing::info!("Tunnel auto-start skipped: {}", e);
                }
            }
        }
    }

    fn is_tunnel_disabled() -> bool {
        if let Ok(val) = std::env::var("BIOROUTER_TUNNEL") {
            let val = val.to_lowercase();
            val == "no" || val == "none"
        } else {
            false
        }
    }

    pub async fn get_info(&self) -> TunnelInfo {
        if Self::is_tunnel_disabled() {
            return TunnelInfo {
                state: TunnelState::Disabled,
                url: String::new(),
                hostname: String::new(),
                secret: String::new(),
            };
        }

        let state = self.state.read().await.clone();
        let info = self.info.read().await.clone();

        match info {
            Some(mut tunnel_info) => {
                tunnel_info.state = state;
                tunnel_info
            }
            None => {
                let effective_state = if state == TunnelState::Idle && is_tunnel_locked_by_another()
                {
                    TunnelState::Running
                } else {
                    state
                };
                TunnelInfo {
                    state: effective_state,
                    url: String::new(),
                    hostname: String::new(),
                    secret: String::new(),
                }
            }
        }
    }

    pub fn set_auto_start(auto_start: bool) -> anyhow::Result<()> {
        Config::global()
            .set_param("tunnel_auto_start", auto_start)
            .map_err(|e| anyhow::anyhow!("Failed to save tunnel config: {}", e))
    }

    pub fn set_secret(secret: &str) -> anyhow::Result<()> {
        Config::global()
            .set_secret("tunnel_secret", &secret.to_string())
            .map_err(|e| anyhow::anyhow!("Failed to save tunnel secret: {}", e))
    }

    pub fn set_agent_id(agent_id: &str) -> anyhow::Result<()> {
        Config::global()
            .set_secret("tunnel_agent_id", &agent_id.to_string())
            .map_err(|e| anyhow::anyhow!("Failed to save tunnel agent_id: {}", e))
    }

    async fn start_tunnel_internal(&self) -> anyhow::Result<(TunnelInfo, mpsc::Receiver<()>)> {
        let server_port = get_server_port()?;
        let tunnel_secret = Self::get_secret().unwrap_or_else(generate_secret);
        let server_secret =
            std::env::var("BIOROUTER_SERVER__SECRET_KEY").unwrap_or_else(|_| "test".to_string());
        let agent_id = Self::get_agent_id().unwrap_or_else(generate_agent_id);

        Self::set_secret(&tunnel_secret)?;
        Self::set_agent_id(&agent_id)?;

        let (restart_tx, restart_rx) = mpsc::channel::<()>(1);
        *self.restart_tx.write().await = Some(restart_tx.clone());

        let result = lapstone::start(
            server_port,
            tunnel_secret,
            server_secret,
            agent_id,
            self.lapstone_handle.clone(),
            restart_tx,
        )
        .await;

        match result {
            Ok(info) => Ok((info, restart_rx)),
            Err(e) => Err(e),
        }
    }

    pub async fn start(&self) -> anyhow::Result<TunnelInfo> {
        if Self::is_tunnel_disabled() {
            anyhow::bail!("Tunnel is disabled via BIOROUTER_TUNNEL environment variable");
        }

        let mut state = self.state.write().await;
        if *state != TunnelState::Idle {
            anyhow::bail!("Tunnel is already running or starting");
        }

        let lock = try_acquire_tunnel_lock()?;
        *self.lock_file.lock().unwrap() = Some(lock);

        *state = TunnelState::Starting;
        drop(state);

        match self.start_tunnel_internal().await {
            Ok((info, mut restart_rx)) => {
                *self.state.write().await = TunnelState::Running;
                *self.info.write().await = Some(info.clone());
                let _ = Self::set_auto_start(true);

                let state = self.state.clone();
                let lapstone_handle = self.lapstone_handle.clone();
                let watchdog_handle_arc = self.watchdog_handle.clone();
                let manager = Arc::new(self.clone_for_watchdog());

                let watchdog = tokio::spawn(async move {
                    let mut retry_delay = Duration::ZERO;
                    let mut attempt_started = Instant::now();
                    while restart_rx.recv().await.is_some() {
                        let auto_start = Self::get_auto_start();
                        if !auto_start {
                            tracing::info!("Tunnel connection lost but auto_start is disabled");
                            break;
                        }

                        tracing::warn!("Tunnel connection lost, initiating restart...");
                        // The wait keeps the state Running and tunnel.lock held, and
                        // `stop()` aborts this task, which ends the wait at once.
                        retry_delay = next_retry_delay(retry_delay, attempt_started.elapsed());
                        tokio::time::sleep(retry_delay).await;
                        lapstone::stop(lapstone_handle.clone()).await;
                        *state.write().await = TunnelState::Idle;
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        *state.write().await = TunnelState::Starting;

                        match manager.start_tunnel_internal().await {
                            Ok((_, new_restart_rx)) => {
                                *state.write().await = TunnelState::Running;
                                tracing::info!("Tunnel restarted successfully");
                                restart_rx = new_restart_rx;
                                attempt_started = Instant::now();
                            }
                            Err(e) => {
                                tracing::error!("Failed to restart tunnel: {}", e);
                                *state.write().await = TunnelState::Error;
                                break;
                            }
                        }
                    }
                });

                *watchdog_handle_arc.write().await = Some(watchdog);

                Ok(info)
            }
            Err(e) => {
                self.release_lock();
                *self.state.write().await = TunnelState::Error;
                Err(e)
            }
        }
    }

    fn clone_for_watchdog(&self) -> Self {
        TunnelManager {
            state: self.state.clone(),
            info: self.info.clone(),
            lapstone_handle: self.lapstone_handle.clone(),
            restart_tx: self.restart_tx.clone(),
            watchdog_handle: self.watchdog_handle.clone(),
            lock_file: self.lock_file.clone(),
        }
    }

    fn release_lock(&self) {
        if let Ok(mut guard) = self.lock_file.lock() {
            // Dropping the file releases the lock
            guard.take();
        }
    }

    pub async fn stop(&self, clear_auto_start: bool) {
        if let Some(handle) = self.watchdog_handle.write().await.take() {
            handle.abort();
        }

        *self.restart_tx.write().await = None;

        lapstone::stop(self.lapstone_handle.clone()).await;

        self.release_lock();

        *self.state.write().await = TunnelState::Idle;
        *self.info.write().await = None;

        if clear_auto_start {
            let _ = Self::set_auto_start(false);
        }
    }
}

fn generate_secret() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

pub(super) fn generate_agent_id() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn next_retry_delay_backs_off_quick_failures_and_resets_after_a_healthy_connection() {
        let quick = Duration::from_millis(50);
        let mut delay = Duration::ZERO;
        let mut delays = Vec::new();
        for _ in 0..12 {
            delay = next_retry_delay(delay, quick);
            delays.push(delay.as_secs());
        }
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        // A connect that hangs until its 30 s timeout failed; it was not a healthy connection.
        assert_eq!(
            next_retry_delay(Duration::ZERO, Duration::from_secs(30)),
            Duration::from_secs(1)
        );
        // One that stayed up, such as the 300 s idle-timeout reconnect, redials at once.
        assert_eq!(
            next_retry_delay(Duration::from_secs(300), Duration::from_secs(301)),
            Duration::ZERO
        );
    }

    /// A relay on 127.0.0.1 that turns down every websocket upgrade with a 404, as the
    /// default relay does today, and counts the connections it accepts. It accepts
    /// rather than refusing the port, so Windows' connect retry does not skew the count.
    async fn refusing_relay() -> (std::net::SocketAddr, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let counted = accepts.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (addr, accepts)
    }

    /// The idle daemon that burned a quarter of a core after the app quit: Remote access
    /// on, a relay that refuses the websocket, and a watchdog that redialled it about 5
    /// to 9 times a second for the daemon's whole life. Now the redials back off, 1 s, 2 s, ….
    #[tokio::test]
    async fn a_relay_that_refuses_the_websocket_is_not_redialled_in_a_tight_loop() {
        // Saves the tunnel's secrets, so only where they live in the sandbox's file.
        if !crate::test_sandbox::global_config_reads_secrets_from_a_file() {
            return;
        }
        // Writes tunnel_auto_start and takes tunnel.lock under its own sandbox root.
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let (relay, accepts) = refusing_relay().await;
        let relay_url = format!("http://{relay}");
        let _env = env_lock::lock_env([
            ("BIOROUTER_TUNNEL_WORKER_URL", Some(relay_url.as_str())),
            ("BIOROUTER_TUNNEL", None),
        ]);

        let manager = TunnelManager::new();
        manager.start().await.expect("the tunnel starts");
        tokio::time::sleep(Duration::from_secs(4)).await;
        manager.stop(true).await;

        // Dialled at about 0, 1.2 and 3.5 s. Without the backoff, about 35 times.
        let dialled = accepts.load(Ordering::SeqCst);
        assert!(
            (2..=4).contains(&dialled),
            "the refusing relay was dialled {dialled} times in 4 s"
        );
    }
}
