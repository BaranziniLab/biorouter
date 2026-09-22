//! Host-owned, in-memory consent for one chat's current Biorouter Copilot task.
use anyhow::{bail, ensure, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::agents::types::SharedProvider;

/// The ONE refusal from [`ComputerUseConsent::status`] that re-asking can never
/// change: this chat's MODE does not run Biorouter Copilot tools.
///
/// ⚠ It is a TYPE and not a sentence because the caller that has to tell it
/// apart is an HTTP route in another crate, and what it does with the answer is
/// permanent: the interface renders a 409 as "this chat cannot use Biorouter
/// Copilot", hides the panel, and tears down its poll for the life of the chat --
/// taking the **Stop** button, the safety control of a desktop-control feature,
/// with it.
///
/// `status` also fails for reasons that are merely NOT YET true: the chat has
/// no model bound, or another chat holds the runtime. While all of them
/// collapsed into one 409, a chat that was a moment from working was written
/// off forever. Matching on the message text instead would put that distinction
/// in a string literal two crates apart.
#[derive(Debug, thiserror::Error)]
#[error("Chat mode does not run Biorouter Copilot tools")]
pub struct ModeForbidsComputerUse;

pub const APPROVAL_REQUIRED: &str = "COMPUTER_USE_APPROVAL_REQUIRED";
pub const TOOLS: &[&str] = &[
    "list_apps",
    "get_app_state",
    "click",
    "perform_secondary_action",
    "scroll",
    "drag",
    "type_text",
    "press_key",
    "set_value",
    "screen_capture",
];

pub fn is_computer_use_tool(name: &str) -> bool {
    name.strip_prefix("computercontroller__")
        .is_some_and(|tool| TOOLS.contains(&tool))
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ComputerUseStatus {
    pub runtime: serde_json::Value,
    pub enabled: bool,
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub destination: String,
    pub target: String,
    pub disclosure: String,
    pub state: String,
    pub challenge_id: String,
    pub public_model: bool,
    pub handoff_required: bool,
    pub requested: bool,
    /// The most recent actual Copilot request, independent of routine task cleanup.
    #[serde(default)]
    pub activity_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    session: String,
    provider: String,
    model: String,
    destination: String,
    binding: String,
    public_model: bool,
    target: String,
}

struct Grant {
    generation: String,
    cancel: CancellationToken,
}

#[derive(Default)]
struct State {
    task: Option<String>,
    task_binding: Option<Scope>,
    scope: Option<Scope>,
    challenge: String,
    desktop_epoch: u64,
    grant: Option<Grant>,
    requested: bool,
    activity_id: Option<String>,
    stopped: bool,
}

#[derive(Default)]
struct Desktop {
    owner: Option<(String, CancellationToken)>,
    lock: Option<Arc<File>>,
    epoch: u64,
}

fn desktop() -> &'static Mutex<Desktop> {
    static DESKTOP: OnceLock<Mutex<Desktop>> = OnceLock::new();
    DESKTOP.get_or_init(Mutex::default)
}

fn action_lock() -> Arc<tokio::sync::Mutex<()>> {
    static ACTIONS: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
    ACTIONS
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub struct ComputerUseConsent {
    identity: String,
    state: Mutex<State>,
    changed: tokio::sync::Notify,
    execution_allowed: AtomicBool,
}

impl Default for ComputerUseConsent {
    fn default() -> Self {
        Self {
            identity: Uuid::new_v4().to_string(),
            state: Mutex::default(),
            changed: tokio::sync::Notify::new(),
            execution_allowed: AtomicBool::new(true),
        }
    }
}

impl ComputerUseConsent {
    pub fn set_mode(&self, mode: crate::config::BioRouterMode) {
        self.execution_allowed.store(
            mode != crate::config::BioRouterMode::Chat,
            Ordering::Release,
        );
        if mode == crate::config::BioRouterMode::Chat {
            self.revoke();
        }
    }
    pub fn task_guard(self: &Arc<Self>) -> ComputerUseTaskGuard {
        self.revoke();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let task = Uuid::new_v4().to_string();
        state.task = Some(task.clone());
        state.task_binding = None;
        state.stopped = false;
        state.challenge = Uuid::new_v4().to_string();
        ComputerUseTaskGuard {
            consent: self.clone(),
            task,
        }
    }
    pub async fn bind_task(&self, session: &str, provider: &SharedProvider) -> Result<()> {
        let scope = Self::scope(session, provider).await?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(
            state.task.is_some(),
            "Biorouter Copilot requires an active request"
        );
        ensure!(
            state.task_binding.is_none(),
            "Biorouter Copilot request model is already bound"
        );
        state.task_binding = Some(scope);
        Ok(())
    }

    async fn scope(session: &str, provider: &SharedProvider) -> Result<Scope> {
        let provider = provider.lock().await;
        let provider = provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bind a model before starting Biorouter Copilot"))?;
        let binding = serde_json::to_value(provider.restore_binding())?;
        let mut endpoints = Vec::new();
        collect_endpoints(&binding, &mut endpoints);
        endpoints.sort();
        endpoints.dedup();
        let destination = if let Some(destination) = provider.computer_use_destination() {
            destination
        } else if endpoints.is_empty() {
            format!("{} (provider-configured destination)", provider.get_name())
        } else {
            endpoints.join(", ")
        };
        Ok(Scope {
            session: session.to_owned(),
            provider: provider.get_name().to_owned(),
            model: provider.get_active_model_name(),
            destination,
            // An instance change invalidates consent even when a registry-backed
            // provider cannot describe its endpoint in the restoration recipe.
            binding: format!(
                "{:p}:{}:{:?}",
                Arc::as_ptr(provider),
                binding,
                provider.computer_use_destination_identity()
            ),
            public_model: !provider.tier().is_private(),
            target: format!(
                "{} ({})",
                sys_info::hostname().unwrap_or_else(|_| "BioRouter backend host".into()),
                std::env::consts::OS
            ),
        })
    }

    pub async fn status(
        &self,
        session: &str,
        provider: &SharedProvider,
    ) -> Result<ComputerUseStatus> {
        if !self.execution_allowed.load(Ordering::Acquire) {
            return Err(ModeForbidsComputerUse.into());
        }
        let scope = Self::scope(session, provider).await?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut desktop = desktop().lock().unwrap_or_else(|e| e.into_inner());
        ensure!(
            state
                .scope
                .as_ref()
                .is_none_or(|prior| prior.session == scope.session),
            "Biorouter Copilot runtime belongs to a different chat"
        );
        if state.scope.as_ref() != Some(&scope) {
            Self::release(&self.identity, &mut state, &mut desktop);
            state.scope = Some(scope.clone());
            state.challenge = Uuid::new_v4().to_string();
            state.desktop_epoch = desktop.epoch;
            state.stopped |= state
                .task_binding
                .as_ref()
                .is_some_and(|binding| binding != &scope);
            state.requested = false;
        }
        if state.desktop_epoch != desktop.epoch {
            state.challenge = Uuid::new_v4().to_string();
            state.desktop_epoch = desktop.epoch;
        }
        if state
            .grant
            .as_ref()
            .is_some_and(|grant| grant.cancel.is_cancelled())
        {
            Self::release(&self.identity, &mut state, &mut desktop);
            state.stopped = true;
            state.requested = false;
        }
        Ok(self.describe(&scope, &state, &desktop))
    }

    fn describe(&self, scope: &Scope, state: &State, desktop: &Desktop) -> ComputerUseStatus {
        let active = state
            .grant
            .as_ref()
            .is_some_and(|g| !g.cancel.is_cancelled());
        // Other backend processes and human activity may leave private content
        // visible even when this process has never owned the desktop.
        let switching = desktop
            .owner
            .as_ref()
            .is_some_and(|(owner, _)| owner != &self.identity);
        let handoff = (scope.public_model && !active) || switching;
        let mut disclosure = if scope.public_model {
            format!("Let Biorouter Copilot use {}/{} to control {}? Screenshots, open-window information and app text may be sent to {}, including sensitive information visible on this computer. BioRouter can move the cursor, change focus, type, click and make changes on your behalf. Allow control and sharing for this user request, including all its tool actions, until the reply finishes or you stop it? {}", scope.provider, scope.model, scope.target, scope.destination, if handoff { "Content left visible by another task or person may be shared. Close or hide anything you do not want shared before allowing this chat to continue." } else { "The desktop, files, clipboard and app logins are shared with other tasks." })
        } else {
            format!("Allow BioRouter to view and control {} for this user request using {}/{} at {}? It can read screenshots and app content, move the cursor, change focus, type, click and make changes on your behalf. Private classification does not mean on-device processing. The desktop, files, clipboard and app logins remain shared. Approval ends when the reply finishes. You can stop it at any time.", scope.target, scope.provider, scope.model, scope.destination)
        };
        if switching {
            disclosure.push_str(" Allowing switches control from the other active Biorouter Copilot task and stops its queued actions.");
        }
        ComputerUseStatus {
            runtime: biorouter_mcp::computer_use::diagnostics(),
            enabled: true,
            session_id: scope.session.clone(),
            provider: scope.provider.clone(),
            model: scope.model.clone(),
            destination: scope.destination.clone(),
            target: scope.target.clone(),
            disclosure,
            state: if active {
                "active"
            } else if state.activity_id.is_none() {
                "idle"
            } else if state.stopped || !state.requested {
                "stopped"
            } else {
                "approval_required"
            }
            .into(),
            challenge_id: state.challenge.clone(),
            public_model: scope.public_model,
            handoff_required: handoff,
            requested: state.requested,
            activity_id: state.activity_id.clone(),
        }
    }

    /// Call only from a human-authenticated host surface, never a model tool.
    pub async fn approve(
        &self,
        session: &str,
        challenge: &str,
        provider: &SharedProvider,
    ) -> Result<ComputerUseStatus> {
        let scope = Self::scope(session, provider).await?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut desktop = desktop().lock().unwrap_or_else(|e| e.into_inner());
        ensure!(
            state.task.is_some() && state.requested && !state.stopped,
            "Biorouter Copilot approval requires a live request waiting to use the computer. Ask in this chat first; setup does not grant control."
        );
        ensure!(
            state.scope.as_ref() == Some(&scope)
                && state.challenge == challenge
                && !challenge.is_empty()
                && state.desktop_epoch == desktop.epoch,
            "Biorouter Copilot scope changed. Review the current acknowledgement and allow again."
        );
        ensure!(state.task_binding.as_ref() == Some(&scope), "The model changed during this request. Start a new request before approving Biorouter Copilot.");
        if desktop.lock.is_none() {
            let path = desktop_lock_path()?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            file.try_lock_exclusive().map_err(|_| anyhow::anyhow!("Biorouter Copilot is busy in another BioRouter process. Stop its task before switching control."))?;
            desktop.lock = Some(Arc::new(file));
        }
        if let Some((_, cancel)) = desktop.owner.take() {
            cancel.cancel();
        }
        if let Some(grant) = state.grant.take() {
            grant.cancel.cancel();
        }
        let cancel = CancellationToken::new();
        desktop.owner = Some((self.identity.clone(), cancel.clone()));
        desktop.epoch += 1;
        state.desktop_epoch = desktop.epoch;
        state.grant = Some(Grant {
            generation: Uuid::new_v4().to_string(),
            cancel,
        });
        state.challenge = Uuid::new_v4().to_string();
        state.stopped = false;
        state.requested = true;
        self.changed.notify_waiters();
        Ok(self.describe(&scope, &state, &desktop))
    }

    pub fn revoke(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut desktop = desktop().lock().unwrap_or_else(|e| e.into_inner());
        self.revoke_locked(&mut state, &mut desktop);
    }

    fn revoke_locked(&self, state: &mut State, desktop: &mut Desktop) {
        Self::release(&self.identity, state, desktop);
        state.stopped = true;
        state.requested = false;
        state.challenge = Uuid::new_v4().to_string();
        self.changed.notify_waiters();
    }

    fn release(identity: &str, state: &mut State, desktop: &mut Desktop) {
        let grant = state.grant.take();
        if let Some(grant) = &grant {
            grant.cancel.cancel();
        }
        let mut lease = None;
        if desktop
            .owner
            .as_ref()
            .is_some_and(|(owner, _)| owner == identity)
        {
            desktop.owner = None;
            lease = desktop.lock.take();
            desktop.epoch += 1;
        }
        if let (Some(grant), Some(scope), Ok(runtime)) =
            (grant, &state.scope, tokio::runtime::Handle::try_current())
        {
            let session = scope.session.clone();
            runtime.spawn(async move {
                let _lease = lease;
                biorouter_mcp::computer_use::stop_session(&session, &grant.generation).await;
            });
        }
    }

    pub async fn permit(
        &self,
        session: &str,
        provider: &SharedProvider,
        cancel: &CancellationToken,
    ) -> Result<ComputerUsePermit> {
        ensure!(!crate::user_surface::no_human_surface(), "{APPROVAL_REQUIRED}: Biorouter Copilot must run within its approved chat request; an identity-free API call cannot consume a chat's grant");
        if !self.execution_allowed.load(Ordering::Acquire) {
            return Err(ModeForbidsComputerUse.into());
        }
        let status = self.status(session, provider).await?;
        let (expected_scope, expected_task) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            ensure!(
                state.task.is_some(),
                "Biorouter Copilot requires an active chat request"
            );
            ensure!(
                !state.stopped,
                "Biorouter Copilot is stopped. Use the chat's Allow control button to start a new task."
            );
            ensure!(state.task_binding == state.scope, "The model changed during this request. Start a new request before using Biorouter Copilot.");
            state.requested = true;
            state.activity_id = state.task.clone();
            (state.scope.clone(), state.task.clone())
        };
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    ensure!(!state.stopped, "Biorouter Copilot was stopped or declined");
                    ensure!(
                        state.scope == expected_scope,
                        "Biorouter Copilot destination changed while awaiting approval"
                    );
                    ensure!(
                        state.task == expected_task,
                        "Biorouter Copilot request changed while awaiting approval"
                    );
                    if let Some(grant) = state.grant.as_ref().filter(|g| !g.cancel.is_cancelled()) {
                        let desktop = desktop().lock().unwrap_or_else(|e| e.into_inner());
                        ensure!(
                            desktop
                                .owner
                                .as_ref()
                                .is_some_and(|(owner, _)| owner == &self.identity),
                            "Biorouter Copilot control belongs to another task"
                        );
                        return Ok(ComputerUsePermit {
                            generation: grant.generation.clone(),
                            cancel: grant.cancel.clone(),
                            lease: desktop.lock.clone(),
                        });
                    }
                }
                tokio::select! {
                    _ = cancel.cancelled() => { self.revoke(); bail!("Biorouter Copilot cancelled while awaiting consent") },
                    _ = &mut notified => {}
                }
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(900), wait).await {
            Ok(result) => result,
            Err(_) => {
                self.revoke();
                bail!(
                    "{APPROVAL_REQUIRED}: Timed out waiting for approval. {}",
                    status.disclosure
                )
            }
        }
    }
}

impl Drop for ComputerUseConsent {
    fn drop(&mut self) {
        self.revoke();
    }
}

pub struct ComputerUseTaskGuard {
    consent: Arc<ComputerUseConsent>,
    task: String,
}

impl Drop for ComputerUseTaskGuard {
    fn drop(&mut self) {
        let mut state = self.consent.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.task.as_deref() == Some(&self.task) {
            let mut desktop = desktop().lock().unwrap_or_else(|e| e.into_inner());
            state.task = None;
            self.consent.revoke_locked(&mut state, &mut desktop);
        }
    }
}

pub struct ComputerUsePermit {
    pub generation: String,
    pub cancel: CancellationToken,
    // Keep the OS lock until a cancelled operation has actually settled.
    lease: Option<Arc<File>>,
}

impl ComputerUsePermit {
    pub async fn lock(&self) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let _lease = &self.lease;
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => bail!("Biorouter Copilot stopped; no further actions are authorized"),
            guard = action_lock().lock_owned() => Ok(guard),
        }
    }
}

fn desktop_lock_path() -> Result<std::path::PathBuf> {
    #[cfg(test)]
    {
        static TEST_ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
        Ok(TEST_ROOT
            .get_or_init(|| tempfile::tempdir().unwrap())
            .path()
            .join("computer-use.desktop.lock"))
    }
    #[cfg(not(test))]
    Ok(dirs::data_local_dir()
        .ok_or_else(|| {
            anyhow::anyhow!("Cannot resolve this OS user's desktop ownership directory")
        })?
        .join("BioRouter")
        .join("computer-use.desktop.lock"))
}

fn collect_endpoints(value: &serde_json::Value, endpoints: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "endpoint" | "base_url" | "host" | "api_base") {
                    if let Some(raw) = value.as_str() {
                        if let Ok(url) = url::Url::parse(raw) {
                            if let Some(host) = url.host_str() {
                                endpoints.push(format!(
                                    "{}://{}{}",
                                    url.scheme(),
                                    host,
                                    url.port().map(|p| format!(":{p}")).unwrap_or_default()
                                ));
                            }
                        }
                    }
                } else {
                    collect_endpoints(value, endpoints);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_endpoints(value, endpoints);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::privacy::ProviderTier;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};

    struct Model(ProviderTier);

    #[async_trait::async_trait]
    impl Provider for Model {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "consent-test-model"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new_or_fail("test")
        }
        fn tier(&self) -> ProviderTier {
            self.0
        }
        async fn complete_with_model(
            &self,
            _: &crate::model::ModelConfig,
            _: &str,
            _: &[crate::conversation::message::Message],
            _: &[rmcp::model::Tool],
        ) -> Result<
            (crate::conversation::message::Message, ProviderUsage),
            crate::providers::errors::ProviderError,
        > {
            unreachable!()
        }
    }

    fn model(tier: ProviderTier) -> SharedProvider {
        Arc::new(tokio::sync::Mutex::new(Some(
            Arc::new(Model(tier)) as Arc<dyn Provider>
        )))
    }

    async fn grant(
        consent: &ComputerUseConsent,
        session: &str,
        provider: &SharedProvider,
    ) -> ComputerUseStatus {
        let needs_binding = consent.state.lock().unwrap().task_binding.is_none();
        if needs_binding {
            consent.bind_task(session, provider).await.unwrap();
        }
        let cancel = CancellationToken::new();
        let mut pending = Box::pin(consent.permit(session, provider, &cancel));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut pending)
                .await
                .is_err()
        );
        let status = consent.status(session, provider).await.unwrap();
        let approved = consent
            .approve(session, &status.challenge_id, provider)
            .await
            .unwrap();
        drop(pending);
        approved
    }

    pub(crate) fn test_serial() -> &'static tokio::sync::Mutex<()> {
        static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        &SERIAL
    }

    #[tokio::test]
    async fn ordinary_requests_never_report_copilot_activity() {
        let consent = Arc::new(ComputerUseConsent::default());
        let provider = model(ProviderTier::Public);
        for _ in 0..3 {
            let task = consent.task_guard();
            consent.bind_task("unused", &provider).await.unwrap();
            let before = consent.status("unused", &provider).await.unwrap();
            assert_eq!(before.state, "idle");
            assert_eq!(before.activity_id, None);
            drop(task);
            let after = consent.status("unused", &provider).await.unwrap();
            assert_eq!(after.state, "idle");
            assert_eq!(after.activity_id, None);
            assert!(consent.state.lock().unwrap().stopped);
        }
    }

    #[tokio::test]
    async fn activity_belongs_to_the_request_and_never_to_another_chat() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let provider = model(ProviderTier::Public);
        let task = consent.task_guard();
        let approved = grant(&consent, "used", &provider).await;
        let activity = approved.activity_id.unwrap();
        drop(task);
        let stopped = consent.status("used", &provider).await.unwrap();
        assert_eq!(stopped.state, "stopped");
        assert_eq!(stopped.activity_id.as_deref(), Some(activity.as_str()));

        let unused = ComputerUseConsent::default();
        assert_eq!(
            unused
                .status("unused", &provider)
                .await
                .unwrap()
                .activity_id,
            None
        );
        assert!(consent.status("unused", &provider).await.is_err());

        let ordinary = consent.task_guard();
        consent.bind_task("used", &provider).await.unwrap();
        drop(ordinary);
        assert_eq!(
            consent
                .status("used", &provider)
                .await
                .unwrap()
                .activity_id
                .as_deref(),
            Some(activity.as_str())
        );
        let _next_task = consent.task_guard();
        let next = grant(&consent, "used", &provider).await;
        assert_ne!(next.activity_id.as_deref(), Some(activity.as_str()));
    }

    #[tokio::test]
    async fn initial_call_waits_then_one_grant_covers_multiple_actions_and_revoke_blocks() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let _task = consent.task_guard();
        let provider = model(ProviderTier::Private);
        consent.bind_task("one", &provider).await.unwrap();
        let cancel = CancellationToken::new();
        let waiting = consent.permit("one", &provider, &cancel);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut waiting)
                .await
                .is_err()
        );
        let status = consent.status("one", &provider).await.unwrap();
        assert!(status.requested);
        assert!(status
            .disclosure
            .contains("Private classification does not mean on-device"));
        let approved = consent
            .approve("one", &status.challenge_id, &provider)
            .await
            .unwrap();
        assert_eq!(approved.state, "active");
        let first = waiting.await.unwrap();
        let second = consent.permit("one", &provider, &cancel).await.unwrap();
        assert_eq!(first.generation, second.generation);
        for mode in [
            crate::config::BioRouterMode::Auto,
            crate::config::BioRouterMode::Approve,
            crate::config::BioRouterMode::SmartApprove,
        ] {
            consent.set_mode(mode);
            assert_eq!(
                consent
                    .permit("one", &provider, &cancel)
                    .await
                    .unwrap()
                    .generation,
                first.generation
            );
        }
        consent.revoke();
        assert!(first.cancel.is_cancelled());
        assert!(second.lock().await.is_err());
        assert!(consent.permit("one", &provider, &cancel).await.is_err());
    }

    #[tokio::test]
    async fn another_chat_requires_its_own_acknowledgement_and_handoff_cancels_old_grant() {
        let _serial = test_serial().lock().await;
        let private = Arc::new(ComputerUseConsent::default());
        let _private_task = private.task_guard();
        let public = Arc::new(ComputerUseConsent::default());
        let _public_task = public.task_guard();
        let private_model = model(ProviderTier::Private);
        let public_model = model(ProviderTier::Public);
        let cancel = CancellationToken::new();
        grant(&private, "private", &private_model).await;
        let old = private
            .permit("private", &private_model, &cancel)
            .await
            .unwrap();
        public.bind_task("public", &public_model).await.unwrap();
        let pending = public.permit("public", &public_model, &cancel);
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut pending)
                .await
                .is_err()
        );
        let next = public.status("public", &public_model).await.unwrap();
        assert_ne!(next.state, "active");
        assert!(next.handoff_required);
        assert!(next.disclosure.contains("switches control"));
        assert!(public
            .approve("public", "model-invented-approval", &public_model)
            .await
            .is_err());
        public
            .approve("public", &next.challenge_id, &public_model)
            .await
            .unwrap();
        assert!(old.cancel.is_cancelled());
        let current = public
            .permit("public", &public_model, &cancel)
            .await
            .unwrap();
        assert_ne!(old.generation, current.generation);
        assert!(private.status("public", &private_model).await.is_err());
        public.revoke();
        private.revoke();
    }

    #[tokio::test]
    async fn model_rebinding_invalidates_scope_and_stale_acknowledgements() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let _task = consent.task_guard();
        let provider = model(ProviderTier::Private);
        consent.bind_task("chat", &provider).await.unwrap();
        let before = consent.status("chat", &provider).await.unwrap();
        *provider.lock().await = Some(Arc::new(Model(ProviderTier::Public)));
        assert!(consent
            .approve("chat", &before.challenge_id, &provider)
            .await
            .is_err());
        let now = consent.status("chat", &provider).await.unwrap();
        assert!(now.public_model);
        assert_ne!(before.challenge_id, now.challenge_id);
        assert!(now.disclosure.contains("may be sent"));
    }

    #[tokio::test]
    async fn same_tier_provider_swap_cannot_authorize_an_old_request() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let _task = consent.task_guard();
        let provider = model(ProviderTier::Public);
        consent.bind_task("chat", &provider).await.unwrap();
        *provider.lock().await = Some(Arc::new(Model(ProviderTier::Public)));
        assert!(consent
            .permit("chat", &provider, &CancellationToken::new())
            .await
            .is_err());
        let status = consent.status("chat", &provider).await.unwrap();
        assert!(!status.requested);
        assert!(consent
            .approve("chat", &status.challenge_id, &provider)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn idle_chat_setup_cannot_hold_a_standing_grant() {
        let _serial = test_serial().lock().await;
        let consent = ComputerUseConsent::default();
        let provider = model(ProviderTier::Public);
        let status = consent.status("chat", &provider).await.unwrap();
        assert!(consent
            .approve("chat", &status.challenge_id, &provider)
            .await
            .is_err());
        assert!(consent
            .permit("chat", &provider, &CancellationToken::new())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn task_completion_revokes_and_os_lease_outlives_active_cancelled_permit() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let provider = model(ProviderTier::Public);
        let task = consent.task_guard();
        grant(&consent, "chat", &provider).await;
        let permit = consent
            .permit("chat", &provider, &CancellationToken::new())
            .await
            .unwrap();
        drop(task);
        assert!(permit.cancel.is_cancelled());
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(desktop_lock_path().unwrap())
            .unwrap();
        assert!(
            contender.try_lock_exclusive().is_err(),
            "revocation cannot release ownership while a native call is settling"
        );
        drop(permit);
        tokio::task::yield_now().await;
        contender.try_lock_exclusive().unwrap();
        assert_eq!(
            consent.status("chat", &provider).await.unwrap().state,
            "stopped"
        );
    }

    #[tokio::test]
    async fn anonymous_api_cannot_wait_for_or_borrow_a_chat_grant_and_chat_mode_cannot_run() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let _task = consent.task_guard();
        let provider = model(ProviderTier::Public);
        let cancel = CancellationToken::new();
        let refused =
            crate::user_surface::without_human_surface(consent.permit("chat", &provider, &cancel))
                .await;
        assert!(refused.is_err());
        assert!(!consent.status("chat", &provider).await.unwrap().requested);
        grant(&consent, "chat", &provider).await;
        assert!(crate::user_surface::without_human_surface(
            consent.permit("chat", &provider, &cancel)
        )
        .await
        .is_err());
        consent.set_mode(crate::config::BioRouterMode::Chat);
        assert!(consent.permit("chat", &provider, &cancel).await.is_err());
    }

    #[tokio::test]
    async fn declining_a_pending_request_wakes_it_without_running() {
        let _serial = test_serial().lock().await;
        let consent = Arc::new(ComputerUseConsent::default());
        let _task = consent.task_guard();
        let provider = model(ProviderTier::Private);
        consent.bind_task("chat", &provider).await.unwrap();
        let cancel = CancellationToken::new();
        let waiting = consent.permit("chat", &provider, &cancel);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut waiting)
                .await
                .is_err()
        );
        consent.revoke();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .is_err()
        );
    }
    #[test]
    fn only_reviewed_builtin_identity_uses_computer_consent() {
        for name in TOOLS {
            assert!(is_computer_use_tool(&format!("computercontroller__{name}")));
        }
        for name in [
            "agentdrafter__list_apps",
            "list_apps",
            "developer__screen_capture",
            "computercontroller__computer_control",
            "computercontroller__automation_script",
        ] {
            assert!(!is_computer_use_tool(name));
        }
    }

    #[test]
    fn computer_use_clients_are_never_generically_pooled() {
        let config = crate::agents::ExtensionConfig::Builtin {
            name: "computercontroller".into(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: vec![],
        };
        assert!(config
            .pool_key(std::path::Path::new("/same-working-directory"))
            .is_none());
    }

    #[test]
    fn endpoint_disclosure_does_not_expose_credentials_or_query() {
        let mut endpoints = Vec::new();
        collect_endpoints(
            &serde_json::json!({"endpoint":"https://user:secret@example.com:8443/api?token=secret"}),
            &mut endpoints,
        );
        assert_eq!(endpoints, ["https://example.com:8443"]);
    }

    #[tokio::test]
    async fn revoked_permit_cannot_acquire_desktop() {
        let permit = ComputerUsePermit {
            generation: "test".into(),
            cancel: CancellationToken::new(),
            lease: None,
        };
        permit.cancel.cancel();
        assert!(permit.lock().await.is_err());
    }
}
