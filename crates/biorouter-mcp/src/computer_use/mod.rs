pub mod contract;
pub mod manifest;
mod runtime;
#[cfg(windows)]
mod windows_job;

use rmcp::model::{CallToolResult, ErrorData};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(crate) struct SessionRuntime {
    session_id: Option<String>,
    generation: Option<String>,
    runtime: Option<runtime::Runtime>,
    inspected_app: Option<String>,
}

struct RuntimeRegistration {
    generation: String,
    runtime: std::sync::Weak<tokio::sync::Mutex<SessionRuntime>>,
    cancellation: CancellationToken,
}

static SESSION_RUNTIMES: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, Vec<RuntimeRegistration>>>,
> = once_cell::sync::Lazy::new(Default::default);

pub(crate) fn register_session(
    session: &str,
    generation: &str,
    runtime: &std::sync::Arc<tokio::sync::Mutex<SessionRuntime>>,
    cancellation: CancellationToken,
) {
    let mut registry = SESSION_RUNTIMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.retain(|_, entries| {
        entries.retain(|entry| entry.runtime.strong_count() > 0);
        !entries.is_empty()
    });
    let entries = registry.entry(session.to_owned()).or_default();
    entries.retain(|entry| !entry.runtime.ptr_eq(&std::sync::Arc::downgrade(runtime)));
    entries.push(RuntimeRegistration {
        generation: generation.to_owned(),
        runtime: std::sync::Arc::downgrade(runtime),
        cancellation,
    });
}

pub async fn stop_session(session_id: &str, generation: &str) {
    let entries = {
        let mut registry = SESSION_RUNTIMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entries) = registry.get_mut(session_id) else {
            return;
        };
        let mut removed = Vec::new();
        let mut index = 0;
        while index < entries.len() {
            if entries[index].generation == generation {
                removed.push(entries.remove(index));
            } else {
                index += 1;
            }
        }
        removed
    };
    for entry in entries {
        entry.cancellation.cancel();
        if let Some(runtime) = entry.runtime.upgrade() {
            let mut session = runtime.lock().await;
            if session.generation.as_deref() == Some(generation) {
                if let Some(mut runtime) = session.runtime.take() {
                    runtime.shutdown().await;
                }
                session.inspected_app.take();
                session.generation.take();
            }
        }
    }
}

fn tool_error(message: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(message.into(), None)
}

impl SessionRuntime {
    pub(crate) fn ensure_session(&self, session_id: &str) -> Result<(), ErrorData> {
        if self
            .session_id
            .as_deref()
            .is_some_and(|bound| bound != session_id)
        {
            return Err(tool_error(
                "computer_use_session_mismatch: observations cannot cross chat connections",
            ));
        }
        Ok(())
    }

    pub async fn call(
        &mut self,
        session_id: &str,
        generation: &str,
        name: &str,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.ensure_session(session_id)?;
        self.session_id = Some(session_id.to_owned());
        if self.generation.as_deref() != Some(generation) {
            if let Some(mut runtime) = self.runtime.take() {
                runtime.shutdown().await;
            }
            self.inspected_app.take();
            self.generation = Some(generation.to_owned());
        }
        let app = arguments
            .get("app")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let observes = matches!(name, "list_apps" | "get_app_state" | "screen_capture");
        if !observes && (self.runtime.is_none() || app.as_deref() != self.inspected_app.as_deref())
        {
            return Err(tool_error(
                "computer_use_stale_state: call get_app_state for this app before acting",
            ));
        }
        if self.runtime.is_none() {
            let startup = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(tool_error("computer_use_cancelled")),
                result = tokio::time::timeout(Duration::from_secs(30), runtime::Runtime::start()) => result,
            };
            self.runtime = Some(
                startup
                    .map_err(|_| tool_error("computer_use_timeout: native startup timed out"))?
                    .map_err(|error| tool_error(error.to_string()))?,
            );
        }
        let mut runtime = self.runtime.take().expect("runtime initialized above");
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(anyhow::anyhow!("computer_use_cancelled: operation stopped; outcome may be uncertain; inspect before any further action")),
            result = tokio::time::timeout(Duration::from_secs(60), runtime.call(name, arguments)) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("computer_use_timeout: operation may have completed; do not replay it; inspect app state first"))),
        };
        match outcome {
            Ok(result) => {
                self.runtime = Some(runtime);
                if result.is_error == Some(true) {
                    self.inspected_app.take();
                } else if name == "get_app_state" {
                    self.inspected_app = app;
                }
                Ok(result)
            }
            Err(error) => {
                runtime.shutdown().await;
                self.runtime.take();
                self.inspected_app.take();
                Err(tool_error(error.to_string()))
            }
        }
    }
}

pub fn diagnostics() -> Value {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "backend host".into());
    match manifest::probe() {
        Ok(payload) => {
            json!({"status":"probe_pending","runtime_version":manifest::UPSTREAM_VERSION,"target":manifest::target(),"executable":payload.executable,"development_override":payload.development_override,"permissions":"unknown","integrity":"verified_on_start","host":host})
        }
        Err(error) => {
            json!({"status":if error.to_string().contains("incompatible_runtime") {"incompatible_runtime"} else {"missing_runtime"},"runtime_version":manifest::UPSTREAM_VERSION,"target":manifest::target(),"permissions":"unknown","integrity":"verified_on_start","host":host,"error":error.to_string()})
        }
    }
}

type ReadinessCache = tokio::sync::Mutex<Option<(std::time::Instant, Value)>>;
static READINESS: once_cell::sync::Lazy<ReadinessCache> =
    once_cell::sync::Lazy::new(Default::default);

pub async fn probe_readiness(refresh: bool) -> Value {
    let mut cache = READINESS.lock().await;
    if !refresh {
        if let Some((observed, value)) = cache.as_ref() {
            if observed.elapsed() < Duration::from_secs(30) {
                return value.clone();
            }
        }
    }
    let mut result = diagnostics();
    if result.get("status").and_then(Value::as_str) == Some("probe_pending") {
        match runtime::Runtime::doctor().await {
            Ok(probe) => {
                result["status"] = probe["state"].clone();
                result["permissions"] = json!({"accessibility":probe["accessibility"],"screen_recording":probe["screen_recording"]});
                for key in ["desktop_available", "capture_available", "message"] {
                    result[key] = probe[key].clone();
                }
                result["integrity"] = json!("verified");
            }
            Err(error) => {
                result["status"] = json!("probe_failed");
                result["message"] = json!(error.to_string());
            }
        }
    }
    *cache = Some((std::time::Instant::now(), result.clone()));
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn state_is_bound_to_chat_and_requires_matching_app_observation() {
        let (_root, runtime) = runtime::tests::fixture("success").await;
        let mut session = SessionRuntime {
            session_id: Some("private-chat".into()),
            generation: Some("task".into()),
            runtime: Some(runtime),
            inspected_app: None,
        };
        let token = CancellationToken::new();
        let foreign = session
            .call("public-chat", "task", "list_apps", json!({}), token.clone())
            .await
            .unwrap_err();
        assert!(foreign.message.contains("session_mismatch"));
        let stale = session
            .call(
                "private-chat",
                "task",
                "click",
                json!({"app":"A","element_index":"0"}),
                token.clone(),
            )
            .await
            .unwrap_err();
        assert!(stale.message.contains("stale_state"));
        session
            .call(
                "private-chat",
                "task",
                "get_app_state",
                json!({"app":"A"}),
                token.clone(),
            )
            .await
            .unwrap();
        let other = session
            .call(
                "private-chat",
                "task",
                "click",
                json!({"app":"B","element_index":"0"}),
                token.clone(),
            )
            .await
            .unwrap_err();
        assert!(other.message.contains("stale_state"));
        session
            .call(
                "private-chat",
                "task",
                "click",
                json!({"app":"A","element_index":"0"}),
                token,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn task_stop_closes_idle_connection_without_revoking_new_generation() {
        let (_root, runtime) = runtime::tests::fixture("success").await;
        let chat = uuid::Uuid::new_v4().to_string();
        let state = std::sync::Arc::new(tokio::sync::Mutex::new(SessionRuntime {
            session_id: Some(chat.clone()),
            generation: Some("new-task".into()),
            runtime: Some(runtime),
            inspected_app: Some("A".into()),
        }));
        let token = CancellationToken::new();
        register_session(&chat, "new-task", &state, token.clone());
        stop_session(&chat, "old-task").await;
        assert!(state.lock().await.runtime.is_some());
        assert!(!token.is_cancelled());
        stop_session(&chat, "new-task").await;
        let stopped = state.lock().await;
        assert!(token.is_cancelled());
        assert!(stopped.runtime.is_none());
        assert!(stopped.inspected_app.is_none());
        assert!(stopped.generation.is_none());
    }

    #[tokio::test]
    async fn cancellation_kills_native_descendants_and_invalidates_references() {
        let (root, runtime) = runtime::tests::fixture("block").await;
        let mut session = SessionRuntime {
            session_id: Some("chat".into()),
            generation: Some("task".into()),
            runtime: Some(runtime),
            inspected_app: Some("A".into()),
        };
        let token = CancellationToken::new();
        let stop = token.clone();
        let pid_file = root.path().join("descendant.pid");
        let watch = pid_file.clone();
        let cancel = tokio::spawn(async move {
            for _ in 0..200 {
                if std::fs::read_to_string(&watch)
                    .ok()
                    .and_then(|value| value.parse::<i32>().ok())
                    .is_some()
                {
                    stop.cancel();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("native fixture did not begin action");
        });
        let result = session
            .call(
                "chat",
                "task",
                "click",
                json!({"app":"A","element_index":"0"}),
                token,
            )
            .await
            .unwrap_err();
        assert!(result.message.contains("cancelled"));
        cancel.await.unwrap();
        assert!(session.runtime.is_none());
        assert!(session.inspected_app.is_none());
        let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        let mut gone = false;
        for _ in 0..100 {
            // The test only probes the exact descendant spawned by its fixture.
            let terminated = unsafe { libc::kill(pid, 0) } == -1;
            #[cfg(target_os = "linux")]
            let terminated = terminated
                || std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .is_ok_and(|stat| stat.split_whitespace().nth(2) == Some("Z"));
            if terminated {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(gone, "native descendant survived request cancellation");
    }
}
