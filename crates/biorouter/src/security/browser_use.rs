//! Host-owned, in-memory consent for the Biorouter Browser in one chat (BB-17).
//!
//! The shape is Biorouter Copilot's ([`super::computer_use`]): a scope bound to the chat's
//! provider, model and destination, a challenge the person answers through a route that requires
//! the user-action proof, a wait of up to 900 s that ends in [`APPROVAL_REQUIRED`] where no
//! person can answer, and revocation on Stop, on a model change and in Chat mode. The model can
//! never grant it.
//!
//! It differs in one way, because reading a page and acting on a site carry different risk:
//! there are two grants.
//!
//! * **Observe** (the chat's lifetime): reading the pages the agent has open, snapshots,
//!   screenshots, scrolling, hovering, waiting, stopping a load, and following a plain link to
//!   the page's own origin.
//! * **Act** (the request, or the chat when the person chooses "Allow for this chat"): opening
//!   an address, going back, forward or reloading, following a link to another origin,
//!   clicking anything other than a link, filling, typing, pressing keys, choosing options,
//!   answering dialogs, downloading. An address the agent opens is text it chose, so a page that
//!   slipped it instructions could have it carry page content off in a query string; the act
//!   grant is the person's say in that (BB-17).
//!
//! A Copilot grant does not admit browser tools and a browser grant does not admit Copilot tools
//! (BB-18, "No conflicts when both are on"); one Stop revokes both (the revoke routes call both).
use anyhow::{bail, ensure, Result};
use rmcp::model::JsonObject;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::agents::types::SharedProvider;

/// The refusal that re-asking can never change: this chat's mode does not run browser tools.
#[derive(Debug, thiserror::Error)]
#[error("Chat mode does not run Biorouter Browser tools")]
pub struct ModeForbidsBrowser;

/// The sentinel an approval that nobody can answer ends in, as Copilot's does.
pub const APPROVAL_REQUIRED: &str = "BROWSER_APPROVAL_REQUIRED";

/// The platform extension's key; its tools are advertised as `browser__browser_*`.
pub const EXTENSION_KEY: &str = "browser";

/// Every tool of the Biorouter Browser, by bare name. No name equals one of Copilot's
/// ([`super::computer_use::TOOLS`]) or any other built-in tool (BB-09).
pub const TOOLS: &[&str] = &[
    "browser_open",
    "browser_navigate",
    "browser_tabs",
    "browser_snapshot",
    "browser_read",
    "browser_screenshot",
    "browser_scroll",
    "browser_hover",
    "browser_wait",
    "browser_click",
    "browser_fill",
    "browser_type",
    "browser_press",
    "browser_select",
    "browser_dialog",
    "browser_drag",
];

/// `browser__<tool>` for one of [`TOOLS`].
pub fn is_browser_tool(prefixed: &str) -> bool {
    prefixed
        .strip_prefix("browser__")
        .is_some_and(|tool| TOOLS.contains(&tool))
}

/// Names that only the built-in Biorouter Browser may answer to, whatever extension claims them.
pub fn is_reserved_browser_name(prefixed: &str) -> bool {
    prefixed.starts_with("browser__browser_")
}

/// Which grant a browser operation needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserGrant {
    Observe,
    Act,
}

impl BrowserGrant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Act => "act",
        }
    }
}

/// How long an act grant lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActLifetime {
    /// Until the reply finishes or the person stops it.
    Request,
    /// Until the person stops it, the model changes or the chat closes.
    Chat,
}

/// Keys a page cannot be changed with: moving focus, scrolling and dismissing. Everything else a
/// key can do (submit, type, delete, a shortcut) needs the act grant.
const OBSERVE_KEYS: &[&str] = &[
    "escape",
    "esc",
    "tab",
    "arrowup",
    "arrowdown",
    "arrowleft",
    "arrowright",
    "up",
    "down",
    "left",
    "right",
    "pageup",
    "pagedown",
    "home",
    "end",
];

/// Whether a `browser_press` key needs only the observe grant: a single navigation key, with at
/// most Shift held (Shift+Tab moves focus back).
pub fn press_is_observation(key: &str) -> bool {
    let parts: Vec<String> = key
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    let Some((last, modifiers)) = parts.split_last() else {
        return false;
    };
    modifiers.iter().all(|modifier| modifier == "shift") && OBSERVE_KEYS.contains(&last.as_str())
}

/// The grant a call is ADMITTED with.
///
/// Every navigation the agent chooses needs the act grant: opening an address (the address is
/// the agent's own text, so a page that injected instructions could have it carry page content
/// to a site of its choosing) and going back, forward or reloading. Stopping a load does not.
///
/// `browser_click` starts at observe: main finds out when it resolves the reference whether the
/// target is a plain http(s) link to the page's own origin, and answers `needs_act_grant`
/// without acting when it is not (a link to another origin, a button, a form control), so the
/// extension asks for the act grant and sends the same operation again.
pub fn grant_for_call(tool: &str, arguments: Option<&JsonObject>) -> BrowserGrant {
    let bare = tool.strip_prefix("browser__").unwrap_or(tool);
    match bare {
        "browser_open" | "browser_fill" | "browser_type" | "browser_select" | "browser_dialog"
        | "browser_drag" => BrowserGrant::Act,
        "browser_navigate" => {
            let action = arguments
                .and_then(|args| args.get("action"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if action.eq_ignore_ascii_case("stop") {
                BrowserGrant::Observe
            } else {
                BrowserGrant::Act
            }
        }
        "browser_press" => {
            let key = arguments
                .and_then(|args| args.get("key"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if press_is_observation(key) {
                BrowserGrant::Observe
            } else {
                BrowserGrant::Act
            }
        }
        _ => BrowserGrant::Observe,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BrowserUseStatus {
    /// The capability is on for this chat.
    pub enabled: bool,
    /// This daemon serves a desktop app that hosts the browser (BB-16).
    pub available: bool,
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub destination: String,
    pub public_model: bool,
    /// `idle`, `approval_required`, `active` or `stopped`.
    pub state: String,
    /// The grant a call is waiting for, if any.
    pub requested: Option<BrowserGrant>,
    /// Reading is allowed for this chat.
    pub observe: bool,
    /// Acting is allowed, and for how long.
    pub act: Option<ActLifetime>,
    pub challenge_id: String,
    /// The consent the card shows for `requested` (or for the active grant).
    pub disclosure: String,
    /// The most recent request that used the browser in this chat.
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
}

struct State {
    task: Option<String>,
    task_binding: Option<Scope>,
    scope: Option<Scope>,
    challenge: String,
    observe: bool,
    act: Option<ActLifetime>,
    requested: Option<BrowserGrant>,
    activity_id: Option<String>,
    stopped: bool,
    /// Cancelled by every revoke, so an operation in flight under a grant stops with it.
    cancel: CancellationToken,
}

impl Default for State {
    fn default() -> Self {
        Self {
            task: None,
            task_binding: None,
            scope: None,
            challenge: Uuid::new_v4().to_string(),
            observe: false,
            act: None,
            requested: None,
            activity_id: None,
            stopped: false,
            cancel: CancellationToken::new(),
        }
    }
}

impl State {
    fn holds(&self, grant: BrowserGrant) -> bool {
        match grant {
            BrowserGrant::Observe => self.observe,
            BrowserGrant::Act => self.act.is_some(),
        }
    }

    fn release(&mut self) {
        self.observe = false;
        self.act = None;
        self.requested = None;
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.challenge = Uuid::new_v4().to_string();
    }
}

/// What an admitted call carries: the grant it was admitted under and the token every revoke
/// cancels.
#[derive(Clone, Debug)]
pub struct BrowserPermit {
    pub grant: BrowserGrant,
    pub cancel: CancellationToken,
}

pub struct BrowserUseConsent {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
    execution_allowed: AtomicBool,
}

impl Default for BrowserUseConsent {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            changed: tokio::sync::Notify::new(),
            execution_allowed: AtomicBool::new(true),
        }
    }
}

/// The consent sentences (BB-17), with the browser's facts in Copilot's words.
pub fn disclosure(
    grant: BrowserGrant,
    provider: &str,
    model: &str,
    destination: &str,
    public_model: bool,
) -> String {
    let private_note = if public_model {
        String::new()
    } else {
        " Private classification does not mean on-device processing.".to_string()
    };
    match grant {
        BrowserGrant::Observe => format!(
            "Let Biorouter read web pages with {provider}/{model}? The pages it has open in the \
             Biorouter Browser, including sites where you are logged in, and screenshots of them \
             go to {destination}. It can read, scroll and follow links within the same site for \
             this chat until you stop it. Opening another address needs your permission to \
             act.{private_note}"
        ),
        BrowserGrant::Act => format!(
            "Let Biorouter open websites, click, type and submit forms in the Biorouter Browser \
             using {provider}/{model}? It acts on websites as you, including where you are logged \
             in, and the pages it opens and screenshots of them go to {destination}. Allow for \
             this request, until the reply finishes or you stop it?{private_note} Stop cannot \
             undo changes already made."
        ),
    }
}

impl BrowserUseConsent {
    pub fn set_mode(&self, mode: crate::config::BioRouterMode) {
        self.execution_allowed.store(
            mode != crate::config::BioRouterMode::Chat,
            Ordering::Release,
        );
        if mode == crate::config::BioRouterMode::Chat {
            self.revoke();
        }
    }

    /// A new request. The observe grant lives on (it is the chat's); a request lifetime act grant
    /// and any unanswered ask end with the request the guard belongs to.
    pub fn task_guard(self: &Arc<Self>) -> BrowserUseTaskGuard {
        let mut state = self.lock();
        let task = Uuid::new_v4().to_string();
        state.task = Some(task.clone());
        state.task_binding = None;
        state.stopped = false;
        state.requested = None;
        if state.act == Some(ActLifetime::Request) {
            state.act = None;
        }
        drop(state);
        self.changed.notify_waiters();
        BrowserUseTaskGuard {
            consent: self.clone(),
            task,
        }
    }

    pub async fn bind_task(&self, session: &str, provider: &SharedProvider) -> Result<()> {
        let scope = Self::scope(session, provider).await?;
        let mut state = self.lock();
        ensure!(
            state.task.is_some(),
            "The Biorouter Browser requires an active request"
        );
        if state.task_binding.is_none() {
            state.task_binding = Some(scope);
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn scope(session: &str, provider: &SharedProvider) -> Result<Scope> {
        let provider = provider.lock().await;
        let provider = provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bind a model before using the Biorouter Browser"))?;
        let binding = serde_json::to_value(provider.restore_binding())?;
        let mut endpoints = Vec::new();
        super::computer_use::collect_endpoints(&binding, &mut endpoints);
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
            // An instance change invalidates consent even when a registry-backed provider cannot
            // describe its endpoint in the restoration recipe.
            binding: format!(
                "{:p}:{}:{:?}",
                Arc::as_ptr(provider),
                binding,
                provider.computer_use_destination_identity()
            ),
            public_model: !provider.tier().is_private(),
        })
    }

    pub async fn status(
        &self,
        session: &str,
        provider: &SharedProvider,
    ) -> Result<BrowserUseStatus> {
        if !self.execution_allowed.load(Ordering::Acquire) {
            return Err(ModeForbidsBrowser.into());
        }
        let scope = Self::scope(session, provider).await?;
        let mut state = self.lock();
        ensure!(
            state
                .scope
                .as_ref()
                .is_none_or(|prior| prior.session == scope.session),
            "The Biorouter Browser consent belongs to a different chat"
        );
        if state.scope.as_ref() != Some(&scope) {
            // A model, provider or destination change ends every grant (BB-17): consent was
            // given to the model that will see the pages, not to whichever one comes next.
            if state.scope.is_some() {
                state.release();
                state.stopped |= state.task_binding.as_ref().is_some_and(|b| b != &scope);
                self.changed.notify_waiters();
            }
            state.scope = Some(scope.clone());
        }
        Ok(self.describe(&scope, &state))
    }

    fn describe(&self, scope: &Scope, state: &State) -> BrowserUseStatus {
        let active = state.observe || state.act.is_some();
        let state_word = if state.requested.is_some() && !state.stopped {
            "approval_required"
        } else if active {
            "active"
        } else if state.activity_id.is_none() {
            "idle"
        } else {
            "stopped"
        };
        let shown = state.requested.unwrap_or(if state.act.is_some() {
            BrowserGrant::Act
        } else {
            BrowserGrant::Observe
        });
        BrowserUseStatus {
            enabled: true,
            available: true,
            session_id: scope.session.clone(),
            provider: scope.provider.clone(),
            model: scope.model.clone(),
            destination: scope.destination.clone(),
            public_model: scope.public_model,
            state: state_word.into(),
            requested: state.requested.filter(|_| !state.stopped),
            observe: state.observe,
            act: state.act,
            challenge_id: state.challenge.clone(),
            disclosure: disclosure(
                shown,
                &scope.provider,
                &scope.model,
                &scope.destination,
                scope.public_model,
            ),
            activity_id: state.activity_id.clone(),
        }
    }

    /// Call only from a human-authenticated host surface, never a model tool.
    pub async fn approve(
        &self,
        session: &str,
        challenge: &str,
        grant: BrowserGrant,
        lifetime: ActLifetime,
        provider: &SharedProvider,
    ) -> Result<BrowserUseStatus> {
        let scope = Self::scope(session, provider).await?;
        let mut state = self.lock();
        ensure!(
            state.task.is_some() && !state.stopped && state.requested.is_some(),
            "Biorouter Browser approval requires a live request waiting to use the browser. Ask in this chat first."
        );
        ensure!(
            state.requested == Some(grant),
            "The Biorouter Browser is asking for a different permission now. Review the current request and allow again."
        );
        ensure!(
            state.scope.as_ref() == Some(&scope) && state.challenge == challenge && !challenge.is_empty(),
            "The Biorouter Browser request changed. Review the current acknowledgement and allow again."
        );
        ensure!(
            state.task_binding.as_ref() == Some(&scope),
            "The model changed during this request. Start a new request before allowing the Biorouter Browser."
        );
        state.observe = true;
        if grant == BrowserGrant::Act {
            state.act = Some(lifetime);
        }
        state.requested = None;
        state.challenge = Uuid::new_v4().to_string();
        state.stopped = false;
        let status = self.describe(&scope, &state);
        drop(state);
        self.changed.notify_waiters();
        Ok(status)
    }

    /// Stop, Cancel on the card, a model change, Chat mode: every grant ends and every operation
    /// in flight under one is cancelled.
    pub fn revoke(&self) {
        let mut state = self.lock();
        state.release();
        state.stopped = true;
        drop(state);
        self.changed.notify_waiters();
    }

    /// Whether this chat holds `grant` right now (for tests and the status line).
    pub fn holds(&self, grant: BrowserGrant) -> bool {
        self.lock().holds(grant)
    }

    pub async fn permit(
        &self,
        session: &str,
        provider: &SharedProvider,
        grant: BrowserGrant,
        cancel: &CancellationToken,
    ) -> Result<BrowserPermit> {
        ensure!(
            !crate::user_surface::no_human_surface(),
            "{APPROVAL_REQUIRED}: no person is present to approve the Biorouter Browser here (a scheduled run, or an API call that is not a chat request), so it is unavailable; it runs only within a person's chat request"
        );
        if !self.execution_allowed.load(Ordering::Acquire) {
            return Err(ModeForbidsBrowser.into());
        }
        let status = self.status(session, provider).await?;
        let (expected_scope, expected_task) = {
            let mut state = self.lock();
            ensure!(
                state.task.is_some(),
                "The Biorouter Browser requires an active chat request"
            );
            ensure!(
                !state.stopped,
                "The Biorouter Browser was stopped or declined in this chat. Do not retry, and do not move this task to Biorouter Copilot, scripts or another tool. Tell the person; they can ask again in a new message."
            );
            ensure!(
                state.task_binding == state.scope,
                "The model changed during this request. Start a new request before using the Biorouter Browser."
            );
            state.activity_id = state.task.clone();
            if state.holds(grant) {
                return Ok(BrowserPermit {
                    grant,
                    cancel: state.cancel.clone(),
                });
            }
            // The wider ask wins: an act ask covers an observe ask already shown.
            state.requested = match (state.requested, grant) {
                (Some(BrowserGrant::Act), _) => Some(BrowserGrant::Act),
                _ => Some(grant),
            };
            state.challenge = Uuid::new_v4().to_string();
            (state.scope.clone(), state.task.clone())
        };
        self.changed.notify_waiters();
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let state = self.lock();
                    ensure!(
                        !state.stopped,
                        "The person declined or stopped the Biorouter Browser. This is a stop condition: do not retry, and do not move this task to Biorouter Copilot, scripts or another tool."
                    );
                    ensure!(
                        state.scope == expected_scope,
                        "The model or destination changed while the Biorouter Browser was waiting for approval"
                    );
                    ensure!(
                        state.task == expected_task,
                        "The request changed while the Biorouter Browser was waiting for approval"
                    );
                    if state.holds(grant) {
                        return Ok(BrowserPermit {
                            grant,
                            cancel: state.cancel.clone(),
                        });
                    }
                }
                tokio::select! {
                    _ = cancel.cancelled() => {
                        let mut state = self.lock();
                        state.requested = None;
                        drop(state);
                        self.changed.notify_waiters();
                        bail!("The Biorouter Browser was canceled while waiting for approval")
                    },
                    _ = &mut notified => {}
                }
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(900), wait).await {
            Ok(result) => result,
            Err(_) => {
                let mut state = self.lock();
                state.requested = None;
                state.stopped = true;
                drop(state);
                self.changed.notify_waiters();
                bail!(
                    "{APPROVAL_REQUIRED}: timed out waiting for approval. {}",
                    status.disclosure
                )
            }
        }
    }
}

/// Ends a request: its request lifetime act grant and any unanswered ask go with it.
pub struct BrowserUseTaskGuard {
    consent: Arc<BrowserUseConsent>,
    task: String,
}

impl Drop for BrowserUseTaskGuard {
    fn drop(&mut self) {
        let mut state = self.consent.lock();
        if state.task.as_deref() == Some(&self.task) {
            state.task = None;
            state.task_binding = None;
            state.requested = None;
            if state.act == Some(ActLifetime::Request) {
                state.act = None;
            }
            drop(state);
            self.consent.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::ProviderTier;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};

    struct Model(ProviderTier, &'static str);

    #[async_trait::async_trait]
    impl Provider for Model {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "browser-consent-test"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new_or_fail(self.1)
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
            Arc::new(Model(tier, "m1")) as Arc<dyn Provider>
        )))
    }

    /// Ask for `grant`, answer the card, and return the permit.
    async fn allow(
        consent: &BrowserUseConsent,
        provider: &SharedProvider,
        grant: BrowserGrant,
        lifetime: ActLifetime,
    ) -> BrowserPermit {
        let cancel = CancellationToken::new();
        let mut pending = Box::pin(consent.permit("chat", provider, grant, &cancel));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err(),
            "a grant nobody gave must wait"
        );
        let status = consent.status("chat", provider).await.unwrap();
        assert_eq!(status.state, "approval_required");
        assert_eq!(status.requested, Some(grant));
        consent
            .approve("chat", &status.challenge_id, grant, lifetime, provider)
            .await
            .unwrap();
        pending.await.unwrap()
    }

    #[test]
    fn grants_follow_the_tool_table() {
        for tool in [
            "browser_tabs",
            "browser_snapshot",
            "browser_read",
            "browser_screenshot",
            "browser_scroll",
            "browser_hover",
            "browser_wait",
            "browser_click",
        ] {
            assert_eq!(grant_for_call(tool, None), BrowserGrant::Observe, "{tool}");
        }
        for tool in [
            "browser_fill",
            "browser_type",
            "browser_select",
            "browser_dialog",
            "browser_drag",
        ] {
            assert_eq!(grant_for_call(tool, None), BrowserGrant::Act, "{tool}");
        }
    }

    /// An address the agent opens is text it chose, so every navigation it chooses needs the
    /// act grant: a page that injected instructions must not be able to have the agent carry
    /// page content to another site under the reading grant alone. Stopping a load moves
    /// nothing anywhere and stays a reading operation.
    #[test]
    fn every_navigation_the_agent_chooses_needs_the_act_grant() {
        let call = |tool: &str, args: serde_json::Value| grant_for_call(tool, args.as_object());
        for args in [
            serde_json::json!({ "url": "https://attacker.example/?q=page+text" }),
            serde_json::json!({ "url": "example.org", "new_tab": true }),
        ] {
            assert_eq!(call("browser__browser_open", args), BrowserGrant::Act);
        }
        assert_eq!(grant_for_call("browser_open", None), BrowserGrant::Act);
        for action in ["back", "forward", "reload", "Reload", ""] {
            assert_eq!(
                call(
                    "browser__browser_navigate",
                    serde_json::json!({ "action": action })
                ),
                BrowserGrant::Act,
                "{action}"
            );
        }
        assert_eq!(
            call(
                "browser__browser_navigate",
                serde_json::json!({ "action": "stop" })
            ),
            BrowserGrant::Observe
        );
        let press = |key: &str| {
            let args = serde_json::json!({ "key": key });
            grant_for_call("browser__browser_press", args.as_object())
        };
        for key in [
            "Escape",
            "Tab",
            "Shift+Tab",
            "ArrowDown",
            "PageDown",
            "Home",
            "End",
        ] {
            assert_eq!(press(key), BrowserGrant::Observe, "{key}");
        }
        for key in [
            "Enter",
            "Meta+A",
            "Control+Tab",
            "a",
            "Backspace",
            "Alt+ArrowLeft",
            "",
        ] {
            assert_eq!(press(key), BrowserGrant::Act, "{key}");
        }
    }

    #[test]
    fn no_browser_tool_shares_a_name_with_copilot() {
        for tool in TOOLS {
            assert!(
                !super::super::computer_use::TOOLS.contains(tool),
                "{tool} is also a Copilot tool"
            );
            assert!(tool.starts_with("browser_"));
            assert!(is_browser_tool(&format!("browser__{tool}")));
        }
        assert!(!is_browser_tool("computercontroller__click"));
        assert!(!is_browser_tool("browser__click"));
        assert!(is_reserved_browser_name("browser__browser_anything"));
    }

    #[tokio::test]
    async fn observe_lasts_the_chat_and_act_lasts_the_request() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        allow(
            &consent,
            &provider,
            BrowserGrant::Observe,
            ActLifetime::Request,
        )
        .await;
        allow(&consent, &provider, BrowserGrant::Act, ActLifetime::Request).await;
        assert!(consent.holds(BrowserGrant::Act));
        drop(task);
        assert!(
            consent.holds(BrowserGrant::Observe),
            "observe is the chat's"
        );
        assert!(
            !consent.holds(BrowserGrant::Act),
            "act ended with the request"
        );

        // The next request reads without asking again.
        let _task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        let cancel = CancellationToken::new();
        let permit = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            consent.permit("chat", &provider, BrowserGrant::Observe, &cancel),
        )
        .await
        .expect("an observe grant needs no second card")
        .unwrap();
        assert_eq!(permit.grant, BrowserGrant::Observe);
    }

    #[tokio::test]
    async fn allow_for_this_chat_survives_the_request() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Private);
        let task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        let status = consent.status("chat", &provider).await.unwrap();
        assert!(!status.public_model);
        allow(&consent, &provider, BrowserGrant::Act, ActLifetime::Chat).await;
        drop(task);
        assert!(consent.holds(BrowserGrant::Act));
        assert!(
            consent.holds(BrowserGrant::Observe),
            "acting implies reading"
        );
    }

    #[tokio::test]
    async fn stop_revokes_everything_and_cancels_operations_in_flight() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let _task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        let permit = allow(&consent, &provider, BrowserGrant::Act, ActLifetime::Chat).await;
        consent.revoke();
        assert!(permit.cancel.is_cancelled(), "an operation in flight stops");
        assert!(!consent.holds(BrowserGrant::Observe));
        assert!(!consent.holds(BrowserGrant::Act));
        let cancel = CancellationToken::new();
        let refused = consent
            .permit("chat", &provider, BrowserGrant::Observe, &cancel)
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("Do not retry"), "{refused}");
        assert!(refused.contains("Biorouter Copilot"), "{refused}");
    }

    #[tokio::test]
    async fn a_model_change_ends_every_grant() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        let permit = allow(
            &consent,
            &provider,
            BrowserGrant::Observe,
            ActLifetime::Request,
        )
        .await;
        drop(task);
        *provider.lock().await = Some(Arc::new(Model(ProviderTier::Public, "m2")));
        let status = consent.status("chat", &provider).await.unwrap();
        assert!(!status.observe);
        assert!(permit.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn declining_is_a_stop_condition() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let _task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        let cancel = CancellationToken::new();
        let pending = consent.permit("chat", &provider, BrowserGrant::Observe, &cancel);
        let decline = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            consent.revoke();
        };
        let (result, ()) = tokio::join!(pending, decline);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("declined or stopped"), "{error}");
    }

    #[tokio::test]
    async fn chat_mode_forbids_and_revokes() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let _task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        allow(
            &consent,
            &provider,
            BrowserGrant::Observe,
            ActLifetime::Request,
        )
        .await;
        consent.set_mode(crate::config::BioRouterMode::Chat);
        assert!(!consent.holds(BrowserGrant::Observe));
        let error = consent.status("chat", &provider).await.unwrap_err();
        assert!(error.downcast_ref::<ModeForbidsBrowser>().is_some());
    }

    #[tokio::test]
    async fn approval_needs_the_current_challenge_and_grant() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let _task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        // Nothing is waiting: setup does not grant.
        let status = consent.status("chat", &provider).await.unwrap();
        assert!(consent
            .approve(
                "chat",
                &status.challenge_id,
                BrowserGrant::Observe,
                ActLifetime::Request,
                &provider
            )
            .await
            .is_err());
        let cancel = CancellationToken::new();
        let mut pending =
            Box::pin(consent.permit("chat", &provider, BrowserGrant::Observe, &cancel));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let status = consent.status("chat", &provider).await.unwrap();
        assert!(consent
            .approve(
                "chat",
                "stale",
                BrowserGrant::Observe,
                ActLifetime::Request,
                &provider
            )
            .await
            .is_err());
        assert!(
            consent
                .approve(
                    "chat",
                    &status.challenge_id,
                    BrowserGrant::Act,
                    ActLifetime::Chat,
                    &provider
                )
                .await
                .is_err(),
            "a card for reading cannot be answered with acting"
        );
        assert!(consent
            .status("chat", &provider)
            .await
            .unwrap()
            .disclosure
            .contains("browse"));
    }

    #[tokio::test]
    async fn an_identity_free_call_cannot_use_a_grant() {
        let consent = Arc::new(BrowserUseConsent::default());
        let provider = model(ProviderTier::Public);
        let _task = consent.task_guard();
        consent.bind_task("chat", &provider).await.unwrap();
        let cancel = CancellationToken::new();
        let error = crate::user_surface::without_human_surface(consent.permit(
            "chat",
            &provider,
            BrowserGrant::Observe,
            &cancel,
        ))
        .await
        .unwrap_err()
        .to_string();
        assert!(error.starts_with(APPROVAL_REQUIRED), "{error}");
    }

    #[test]
    fn the_consent_text_names_the_destination_and_the_private_caveat() {
        let public = disclosure(BrowserGrant::Observe, "p", "m", "https://api.example", true);
        assert!(public.contains("p/m") && public.contains("https://api.example"));
        assert!(public.contains("logged in"));
        assert!(!public.contains("Private classification"));
        assert!(public.contains("Opening another address needs your permission to act."));
        let private = disclosure(BrowserGrant::Act, "p", "m", "d", false);
        assert!(private.contains("Private classification does not mean on-device processing."));
        assert!(private.contains("Stop cannot undo changes already made."));
        // An approved act grant also allows reading, so its text names where pages go.
        let act = disclosure(BrowserGrant::Act, "p", "m", "https://api.example", true);
        assert!(act.contains("open websites") && act.contains("https://api.example"));
    }
}
