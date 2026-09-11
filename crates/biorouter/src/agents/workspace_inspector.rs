//! BR-71 §5, the always-confirm special case: *"Removing security-relevant
//! extensions or adding process-spawning ones on any target surfaces a
//! confirmation regardless of mode."*
//!
//! Sibling of [`crate::security::sensitive_ops::SensitiveOpsInspector`], with
//! one deliberate difference: that inspector returns early outside Auto mode
//! because every other mode already gates file writes. This one has no mode
//! gate at all, because no mode gates a cross-session capability change — the
//! capability `workspace_set_tools` exercises did not exist before BR-71.
//!
//! Precedence is free: `apply_inspection_results_to_permissions` promotes any
//! `RequireApproval` over another inspector's `Allow`
//! (`crate::tool_inspection`, `:205` / `:273` / `:277-280`), so this beats Auto
//! mode's blanket allow and a per-tool always-allow grant alike.

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::JsonObject;
use uuid::Uuid;

use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::session::Session;
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};

/// Extensions that spawn or execute code, named as builtins/platform entries.
/// `ExtensionConfig::Stdio` and `::InlinePython` are process-spawning **by
/// construction**, so they are caught structurally below rather than by name —
/// this list is only for the in-process ones (`Builtin`/`Platform`) whose
/// capability is not visible in the config shape.
const PROCESS_SPAWNING_EXTENSIONS: &[&str] = &["developer", "computercontroller", "code_execution"];

/// How dangerous an extension is *by its config shape*, independent of its name.
///
/// Exhaustive on purpose: `ExtensionConfig` has SEVEN variants
/// (`agents/extension.rs:236-358` — `sse` :238, `stdio` :251, `builtin` :272,
/// `platform` :288, `streamable_http` :301, `frontend` :324, `inline_python`
/// :341) and a `matches!(…, Stdio { .. })` covers one of them. `InlinePython`
/// is documented as "Inline Python code that will be executed using uvx"
/// (`:340`) and `extension_manager.rs:802-833` proves it — it writes the code
/// to a tempdir and builds `Command::new("uvx")` (`:814`) — so it is
/// process-spawning by exactly the same reasoning that makes `Stdio` structural.
/// `Sse` and `StreamableHttp` carry `uri` + `envs` + `env_keys` + `headers`
/// (`:301-323`): adding one points another conversation's traffic at an
/// arbitrary remote MCP endpoint, with credentials.
///
/// A `match` rather than a `matches!` so that an eighth variant is a compile
/// error here instead of silently defaulting to "harmless".
enum AddShapeRisk {
    /// Spawns a child process on the target's machine.
    ProcessSpawning,
    /// Sends the target's traffic (and credentials) to a remote endpoint.
    NetworkEgress,
    /// Shape alone says nothing; fall through to the name list.
    Opaque,
}

fn add_shape_risk(name: &str) -> AddShapeRisk {
    use crate::agents::extension::ExtensionConfig as E;
    match crate::config::get_extension_by_name(name) {
        Some(E::Stdio { .. }) | Some(E::InlinePython { .. }) => AddShapeRisk::ProcessSpawning,
        Some(E::Sse { .. }) | Some(E::StreamableHttp { .. }) => AddShapeRisk::NetworkEgress,
        Some(E::Builtin { .. }) | Some(E::Platform { .. }) | Some(E::Frontend { .. }) | None => {
            AddShapeRisk::Opaque
        }
    }
}

/// Compare extension names the way the EXECUTOR does.
///
/// `Agent::remove_extension` forwards to `ExtensionManager::remove_extension`,
/// whose first line is `let sanitized_name = normalize(name);`
/// (`extension_manager.rs:976-981`, `normalize` itself at `:159`) — lower-cased,
/// whitespace-stripped, non-`[A-Za-z0-9_-]` mapped to `_`. So `"Workspace"`,
/// `"WORKSPACE"` and `"work space"` all really do remove the `workspace`
/// extension, while a raw-string `contains("workspace")` check here would see
/// none of them and return `None` — no confirmation, in any mode, for the
/// removal of the audit trail. (The ADD path fails closed instead:
/// `find_entry_by_name` is an exact match on `config.name()`
/// (`config/extensions.rs:183-190`), so a mis-cased add is rejected as an
/// unknown extension.)
///
/// ⚠ **Keep this as plain `normalize`. Do NOT "modernize" it to PR #48's
/// `BundledExtensionTarget::key()`** (ruled 2026-07-28). #48 landed a richer
/// name resolver — `resolve_bundled_extension` (`extension_manager.rs:257`) →
/// `BundledExtensionTarget` (`:183`) with `key()` (`:199`) — and deleted
/// `canonical_builtin_extension_name`. It is the right tool for *resolving a
/// `/ext:` marker to a registry entry*, and `key()` is
/// `normalize(name_to_key(name))`, i.e. a strictly different function. This
/// helper's entire job is to compare names **the way the executor does**, and
/// the executor is `ExtensionManager::remove_extension`, whose first line is
/// still literally `let sanitized_name = normalize(name);` (`:977`). Swapping in
/// `key()` would re-open reconciliation #19's hole from the other side: a name
/// this check normalizes differently from the code that acts on it is a name
/// that can be removed without confirming. If `remove_extension` ever changes
/// resolver, change this in the same commit — and only then.
fn norm(name: &str) -> String {
    crate::agents::extension_manager::normalize(name)
}

/// Extensions whose REMOVAL narrows what a human or a later check can *see*,
/// rather than what the agent can do. That is the asymmetry §5 is about: an
/// agent quietly removing the machinery that would reveal its next action.
/// Both keys below are real `PLATFORM_EXTENSIONS` registry keys
/// (`agents/extension.rs:43-106`: `todo`, `chatrecall`, `extensionmanager`,
/// `skills`, `code_execution`), plus `workspace` once Task 12 adds it.
const SECURITY_RELEVANT_EXTENSIONS: &[&str] = &[
    // Every cross-session action the workspace surface performs is recorded as
    // a tool call in the acting session and toasted on the target tab. Removing
    // it from a session removes that audit trail.
    "workspace",
    // How a session enumerates and re-enables what it is allowed to use; without
    // it, a `workspace_set_tools` change becomes invisible from inside the
    // target session.
    "extensionmanager",
];

/// Both name forms reach dispatch: extension-advertised tools are prefixed
/// `{extension}__{tool}` (`extension_manager.rs:1121`, inside the
/// `is_tool_available` advertisement filter at `:1119`), and the code already
/// tolerates models that strip the prefix (`dispatch_tool_call` `:1438`, the
/// prefix-repair block `:1444-1457`).
pub(crate) fn is_set_tools_call(tool_name: &str) -> bool {
    tool_name == "workspace_set_tools" || tool_name == "workspace__workspace_set_tools"
}

/// `workspace_open`'s `new.extensions` is a SECOND way to grant a
/// process-spawning extension, and by an easier route than `workspace_set_tools`:
/// `start_session` resolves any name the config knows with no policy check
/// (Task 9), and `new.prompt` starts a turn on the result immediately (Task 24).
/// Inspecting only `workspace_set_tools` would mean an Auto-mode agent that is
/// stopped from calling
/// `workspace_set_tools { add_extensions: ["developer"] }` can call
/// `workspace_open { new: { extensions: ["developer"], prompt: "…" } }`
/// and get a live process-spawning agent with no prompt at all — the design's
/// "no self-escalation" bullet, defeated.
pub(crate) fn is_workspace_open_call(tool_name: &str) -> bool {
    tool_name == "workspace_open" || tool_name == "workspace__workspace_open"
}

fn string_list(args: &JsonObject, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Why adding this extension must confirm, if it must. Shared by
/// `workspace_set_tools`'s `add_extensions` and `workspace_open`'s
/// `new.extensions`.
///
/// `risk` is passed in rather than looked up here so the caller owns the single
/// global-config read (see `set_tools_confirmation_reason_with`).
fn add_extension_reason(name: &str, risk: AddShapeRisk) -> Option<String> {
    match risk {
        AddShapeRisk::ProcessSpawning => {
            Some(format!("adds the process-spawning extension '{name}'"))
        }
        AddShapeRisk::NetworkEgress => Some(format!(
            "adds '{name}', which sends this conversation's traffic to a remote endpoint"
        )),
        AddShapeRisk::Opaque => {
            // `developer` / `computercontroller` / `code_execution` are
            // `Builtin` config entries, so only the name list catches them.
            let n = norm(name);
            PROCESS_SPAWNING_EXTENSIONS
                .iter()
                .any(|known| norm(known) == n)
                .then(|| format!("adds the process-spawning extension '{name}'"))
        }
    }
}

/// The whole policy, as one pure function so it is testable without an agent.
/// `Some(reason)` means "confirm, in every mode".
pub(crate) fn set_tools_confirmation_reason(args: &JsonObject) -> Option<String> {
    set_tools_confirmation_reason_with(
        args,
        add_shape_risk,
        &crate::config::persisted_extension_names(),
    )
}

/// The policy with its two config-derived inputs supplied by the caller.
///
/// Both `add_shape_risk` and `persisted_extension_names` read
/// `Config::global()`, i.e. the developer's or the operator's real
/// `config.yaml`. Left inline, which branch a test reaches becomes a property
/// of the machine it runs on — the extensions the tests name resolve to
/// [`AddShapeRisk::Opaque`] here and the operator-authored set is empty, so
/// both the structural risk check and the operator-authored check were dead
/// code under test. Measured, not assumed: collapsing `ProcessSpawning` and
/// `NetworkEgress` into the name list AND deleting the operator-authored branch
/// outright left all nine of this module's original tests green. Hoisting the
/// two reads to the single production caller above lets the tests cover every
/// branch on every machine, without a test ever reading — let alone writing —
/// the user's config.
///
/// `operator_authored_raw` takes the names as the config file spells them;
/// normalizing them is part of the behaviour under test.
fn set_tools_confirmation_reason_with(
    args: &JsonObject,
    shape_risk: impl Fn(&str) -> AddShapeRisk,
    operator_authored_raw: &std::collections::HashSet<String>,
) -> Option<String> {
    let mut reasons: Vec<String> = Vec::new();

    for name in string_list(args, "add_extensions") {
        if let Some(reason) = add_extension_reason(&name, shape_risk(&name)) {
            reasons.push(reason);
        }
    }

    // An operator-authored entry is a human decision the agent must not undo
    // silently. `persisted_extension_names` is exactly "entries present in the
    // config FILE", before platform defaults are injected
    // (`config/extensions.rs`, added for #42) — so an injected default-off
    // platform extension is NOT treated as operator-authored.
    //
    // Both sides of every comparison go through `norm`, because the executor
    // does (see `norm`'s docs). The operator-authored set holds raw config-file
    // names, so it is normalized here rather than at its source.
    let operator_authored: std::collections::HashSet<String> =
        operator_authored_raw.iter().map(|n| norm(n)).collect();
    for name in string_list(args, "remove_extensions") {
        let n = norm(&name);
        if SECURITY_RELEVANT_EXTENSIONS.iter().any(|s| norm(s) == n) {
            reasons.push(format!("removes the security-relevant extension '{name}'"));
        } else if operator_authored.contains(&n) {
            reasons.push(format!(
                "removes '{name}', which the user configured explicitly"
            ));
        }
    }

    // Decision b added provider/model switching to this tool, and it is the
    // single highest-consequence change it can make: the target's ENTIRE stored
    // conversation is then sent to whatever endpoint that provider names, and a
    // custom/declarative provider is a user-defined base URL whose
    // `allows_unlisted_models` flag (`providers/base.rs:163`) waves the model
    // check through. Decision 1's "regardless of mode" cannot be scoped to a
    // subset of the tool that no longer matches what the tool does.
    if let Some(provider) = args.get("provider").and_then(serde_json::Value::as_str) {
        reasons.push(format!(
            "switches this conversation to provider '{provider}', which sends its \
             whole history to that provider's endpoint"
        ));
    }

    // Decision c: a skill injects instructions into the target's prompt.
    let added_skills = string_list(args, "add_skills");
    if !added_skills.is_empty() {
        reasons.push(format!(
            "adds skills to this conversation's prompt ({})",
            added_skills.join(", ")
        ));
    }

    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    }
}

/// The same policy for `workspace_open`'s `new.extensions`. A NEW session has
/// nothing to remove and no provider to switch, so this reads only the grant.
pub(crate) fn open_confirmation_reason(args: &JsonObject) -> Option<String> {
    open_confirmation_reason_with(args, add_shape_risk)
}

/// The open path's half of the seam described on
/// [`set_tools_confirmation_reason_with`]. There is no operator-authored set
/// here because a new session has nothing to remove.
fn open_confirmation_reason_with(
    args: &JsonObject,
    shape_risk: impl Fn(&str) -> AddShapeRisk,
) -> Option<String> {
    let names: Vec<String> = args
        .get("new")
        .and_then(serde_json::Value::as_object)
        .map(|new| {
            new.get("extensions")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let reasons: Vec<String> = names
        .iter()
        .filter_map(|name| add_extension_reason(name, shape_risk(name)))
        .map(|r| r.replacen("adds", "starts a new conversation that has", 1))
        .collect();

    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    }
}

/// [`WorkspaceMutationInspector`]'s `name()`. Shared with the agent loop's
/// denied-call arm, which hands a `Deny` reason from this inspector to the model
/// verbatim — it is Biorouter's refusal of an impossible change, and the stock
/// "the user has declined to run this tool" would be false.
pub(crate) const WORKSPACE_MUTATION_INSPECTOR_NAME: &str = "workspace_mutation";

pub struct WorkspaceMutationInspector {
    /// Sampled for the pre-flight's privacy gates when no capability is pinned,
    /// exactly as [`WorkspaceCrossingInspector`] samples its own.
    provider: crate::agents::types::SharedProvider,
    /// The store the pre-flight resolves the target against — the agent's own,
    /// which is the one its workspace handler reads.
    session_manager: std::sync::Arc<crate::session::SessionManager>,
    /// The operator-authored extension names, when a test supplies them. `None`
    /// reads the real config at inspection time; see
    /// [`set_tools_confirmation_reason_with`] for why that input is a seam.
    operator_authored: Option<std::collections::HashSet<String>>,
}

impl WorkspaceMutationInspector {
    pub fn new(
        provider: crate::agents::types::SharedProvider,
        session_manager: std::sync::Arc<crate::session::SessionManager>,
    ) -> Self {
        Self {
            provider,
            session_manager,
            operator_authored: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_operator_authored(mut self, names: &[&str]) -> Self {
        self.operator_authored = Some(names.iter().map(|n| (*n).to_string()).collect());
        self
    }

    fn set_tools_reason(&self, args: &JsonObject) -> Option<String> {
        match &self.operator_authored {
            Some(names) => set_tools_confirmation_reason_with(args, add_shape_risk, names),
            None => set_tools_confirmation_reason(args),
        }
    }

    async fn inspect_with_pinned_capability(
        &self,
        tool_requests: &[ToolRequest],
        session: &Session,
        capability: Option<crate::privacy::CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        // NOTE: deliberately no mode gate. See the module docs.
        let mut results = Vec::new();
        // Sampled at most once per batch, and only when a call needs it, so an
        // ordinary tool batch never touches the provider lock here.
        let mut sampled = capability;
        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            let Some(args) = tool_call.arguments.as_ref() else {
                continue;
            };
            // TWO tool families grant capabilities to another conversation, and
            // both must be inspected: `workspace_set_tools` changes an existing
            // one, `workspace_open { new: { extensions } }` mints one with the
            // grant baked in and (with `new.prompt`) starts it running. Scoping
            // this to `set_tools` alone leaves the strictly larger capability
            // reachable by the strictly easier route.
            let reason = if is_set_tools_call(&tool_call.name) {
                // F4: ask whether the change CAN be made before asking the user
                // to approve it. A card for an impossible change is a request
                // for authority over nothing — the QA run approved "removes
                // 'autovisualiser'" and was told, afterwards, that a built-in
                // capability cannot be removed this way. The refusal is the
                // handler's own (one function, `preflight_set_tools`), so the
                // model reads the sentence it would have read after the card.
                let cap = match sampled {
                    Some(cap) => cap,
                    None => {
                        let cap = crate::privacy::CallCapability::sample(&self.provider).await;
                        sampled = Some(cap);
                        cap
                    }
                };
                if let Some(refusal) =
                    crate::agents::workspace_extension::WorkspaceClient::set_tools_preflight_refusal(
                        std::sync::Arc::clone(&self.session_manager),
                        &session.id,
                        cap,
                        args,
                    )
                    .await
                {
                    tracing::info!(
                        counter.biorouter.workspace_mutation_preflight_refused = 1,
                        tool_request_id = %request.id,
                        "Workspace tool-set change refused before any approval (F4)"
                    );
                    results.push(InspectionResult {
                        tool_request_id: request.id.clone(),
                        action: InspectionAction::Deny,
                        reason: format!("Error: {refusal}"),
                        confidence: 1.0,
                        inspector_name: self.name().to_string(),
                        finding_id: Some(format!("WSPRE-{}", Uuid::new_v4().simple())),
                    });
                    continue;
                }
                self.set_tools_reason(args)
            } else if is_workspace_open_call(&tool_call.name) {
                open_confirmation_reason(args)
            } else {
                continue;
            };
            let Some(reason) = reason else {
                continue;
            };
            let target = args
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<a new conversation>");
            tracing::warn!(
                counter.biorouter.workspace_mutation_escalated = 1,
                tool_request_id = %request.id,
                target_session = %target,
                "Workspace tool-set change escalated to approval (BR-71 §5)"
            );
            results.push(InspectionResult {
                tool_request_id: request.id.clone(),
                action: InspectionAction::RequireApproval(Some(format!(
                    "🔒 An agent is changing another conversation's capabilities.\n\
                     Target conversation: {target}\n\
                     This change {reason}.\n\
                     This confirmation appears in every permission mode, including \
                     Fully Automatic."
                ))),
                reason: format!("Workspace tool-set change ({reason})"),
                confidence: 1.0,
                inspector_name: self.name().to_string(),
                finding_id: Some(format!("WSMUT-{}", Uuid::new_v4().simple())),
            });
        }
        Ok(results)
    }
}

#[async_trait]
impl ToolInspector for WorkspaceMutationInspector {
    fn name(&self) -> &'static str {
        WORKSPACE_MUTATION_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        _biorouter_mode: BioRouterMode,
        session: &Session,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_with_pinned_capability(tool_requests, session, None)
            .await
    }

    async fn inspect_with_capability(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        _biorouter_mode: BioRouterMode,
        session: &Session,
        capability: Option<crate::privacy::CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_with_pinned_capability(tool_requests, session, capability)
            .await
    }

    // `is_enabled` uses the trait default (always registered): there is no mode
    // gate to honour, and the tool-name filter above already makes it inert for
    // every other call.
}

/// The tool families that carry a PAYLOAD into another conversation, and so
/// have something for a first crossing to disclose. `workspace_close` is
/// deliberately absent: it carries no text.
///
/// ⚠ **Two halves ask this, and they must be the same half.** The inspector
/// below asks it to decide whether to raise the card; `handle_send_prompt` and
/// `handle_set_tools` ask it to decide whether a landed write may mark the pair
/// as having crossed. A record for a change this function calls payload-free
/// would consume the pair's one disclosure without ever having shown one — and
/// that is not hypothetical: `workspace_set_tools { set_knowledge_bases: [] }`
/// is a real, accepted change (it CLEARS the target's bases) that produces no
/// payload here, so a handler recording on "the write succeeded" alone let a
/// caller silence the disclosure with a call the user never saw. Asking one
/// function is what makes the two halves unable to disagree.
pub(crate) fn crossing_payload(tool_name: &str, args: &JsonObject) -> Option<(String, String)> {
    let target = args.get("session_id").and_then(serde_json::Value::as_str)?;
    let payload = if is_send_prompt_call(tool_name) {
        let mode = args
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("turn");
        let text = args.get("text").and_then(serde_json::Value::as_str)?;
        format!("mode {mode} — {text}")
    } else if is_set_tools_call(tool_name) {
        // Not the prose a model wrote, but still caller-chosen content that
        // reconfigures a public conversation: which extensions, skills, model
        // and knowledge bases it will hold.
        let mut parts: Vec<String> = Vec::new();
        for key in [
            "add_extensions",
            "remove_extensions",
            "add_skills",
            "remove_skills",
            "set_knowledge_bases",
        ] {
            let names = string_list(args, key);
            if !names.is_empty() {
                parts.push(format!("{key}: {}", names.join(", ")));
            }
        }
        for key in ["provider", "model"] {
            if let Some(v) = args.get(key).and_then(serde_json::Value::as_str) {
                parts.push(format!("{key}: {v}"));
            }
        }
        if parts.is_empty() {
            return None;
        }
        parts.join("; ")
    } else {
        return None;
    };
    Some((target.to_string(), payload))
}

/// The payload a `workspace_open { new: { prompt } }` would carry into a
/// conversation that **does not exist yet**.
///
/// ⚠ **Its own function because it has no `session_id`, and that absence is
/// exactly why it was missed.** [`crossing_payload`] opens by requiring
/// `args["session_id"]`, so `workspace_open` fell out of every crossing check at
/// the first line — no card, no ledger entry, nothing. A private-model chat
/// could therefore write caller-chosen text into a brand-new conversation in
/// silence, while `workspace_send_prompt` carrying the SAME text into the
/// session it had just created raised the 🔒 approval. The easier route was the
/// unguarded one.
///
/// Having no target is not a reason to skip the disclosure; it is the reason the
/// disclosure cannot be skipped. `start_session` creates a `SessionType::User`
/// row at the default (public) classification, and a conversation that did not
/// exist a moment ago has by construction never crossed with this caller — so
/// for a private caller this is ALWAYS a first crossing, with no ledger to
/// consult.
pub(crate) fn open_new_session_payload(tool_name: &str, args: &JsonObject) -> Option<String> {
    if !is_workspace_open_call(tool_name) {
        return None;
    }
    let new = args.get("new").and_then(serde_json::Value::as_object)?;
    let prompt = new.get("prompt").and_then(serde_json::Value::as_str)?;
    if prompt.trim().is_empty() {
        return None;
    }
    Some(prompt.to_string())
}

/// **The one question every half of the first-crossing disclosure asks**: what
/// would this call write into another conversation, and which one?
///
/// A `None` target means *a conversation this call is about to create*, which
/// the caller must treat as a first crossing outright rather than looking it up.
///
/// Three halves read this and they must not be able to disagree: the inspector
/// that raises the card, [`uninspected_crossing_refusal`] at the boundaries no
/// inspector sees, and `WorkspaceClient::record_crossing_if_disclosed`, which
/// decides a landed write may mark the pair as crossed. The `workspace_open`
/// hole is what happens when one of them is missing an arm — and the record half
/// asking a *different* function from the one the card half asks is how a change
/// nobody was shown consumes the pair's single disclosure.
pub(crate) fn crossing_disclosure(
    tool_name: &str,
    args: &JsonObject,
) -> Option<(Option<String>, String)> {
    if let Some((target, payload)) = crossing_payload(tool_name, args) {
        return Some((Some(target), payload));
    }
    open_new_session_payload(tool_name, args).map(|prompt| (None, prompt))
}

pub(crate) fn is_send_prompt_call(tool_name: &str) -> bool {
    tool_name == "workspace_send_prompt" || tool_name == "workspace__workspace_send_prompt"
}

/// **The first-crossing disclosure** (issue #56, DR-16's `✓!` cells): a
/// private-capability conversation writing into a PUBLIC one shows the user the
/// exact payload, once per (caller, target) pair, in every permission mode.
///
/// ⚠ **This predicate shipped unwired, and widening the write rule is what made
/// wiring it non-optional.** While WRITE was `VIS ∧ L ∈ {self, child}`, the only
/// public targets a private caller could write into were ones it had spawned
/// itself. It can now write into any public conversation on the machine — one
/// the user opened, is reading, and never connected to this agent — so the
/// moment private-origin text leaves for a public model is a moment the user has
/// to be able to see. `crates/biorouter/tests/privacy_guard_wiring.rs` carried
/// the predicate as `Status::Unwired("OPERATOR DECISION OUTSTANDING")` until
/// this inspector existed.
///
/// Sibling of [`WorkspaceMutationInspector`] and deliberately a SEPARATE
/// inspector rather than another `reason` arm inside it: that one asks a pure
/// question about the arguments, where this one has to resolve the caller's
/// bound provider and the target's stored classification. Folding an async,
/// I/O-bearing decision into a pure one is how the pure one stops being
/// testable.
///
/// Like its sibling it has **no mode gate**: `apply_inspection_results_to_permissions`
/// promotes `RequireApproval` over another inspector's `Allow`, so this beats
/// Auto mode's blanket allow. Unlike its sibling it cannot be a unit-pure
/// function, so the decision is split — [`crossing_payload`] and
/// [`crate::privacy::crossing::needs_disclosure`] are both testable without an
/// agent, and this `inspect` is the two of them plus two lookups.
/// The refusal a dispatch boundary that never reaches a [`ToolInspector`] must
/// return for a workspace write that would be a **first crossing**.
///
/// ⚠ **An inspector is not a gate at every door.** `ExtensionManager::dispatch_tool_call`
/// is reached from four places and only one of them runs the inspector stack;
/// the JS sandbox's tool handler hands a script's inner calls straight to it
/// (`agents/code_execution_extension.rs`), which is why that file already
/// carries boundary refusals for the global memory store and the session
/// database. This is the third, and it is needed for a sharper reason than the
/// other two: skipping the card would not merely let ONE undisclosed write
/// through — the handler would then **record the pair as crossed**, so every
/// later, properly-inspected write to that same conversation would be silent
/// too. One un-inspected call would permanently disable the disclosure.
///
/// Deliberately narrow. It refuses only what would otherwise have raised a
/// card: a same-tier write is untouched, so is a pair the user has already
/// approved, and so is every tool that carries no payload. A script that needs
/// to make a first crossing is told to make the call where the user can see it.
pub async fn uninspected_crossing_refusal(
    cap: crate::privacy::CallCapability,
    caller_session_id: &str,
    tool_name: &str,
    args: Option<&JsonObject>,
    boundary: crate::security::UninspectedBoundary,
) -> Option<String> {
    if !cap.enforced() || !cap.tier().is_private() {
        return None;
    }
    let (target, _) = crossing_disclosure(tool_name, args?)?;
    // No target means the call MINTS its conversation, so there is no row to
    // resolve and no pair to look up: a private caller seeding a brand-new
    // (public-by-default) session is a first crossing every time.
    let target = match target {
        Some(target) => target,
        None => {
            return Some(format!(
                "Refused: this conversation runs on a model hosted inside your institution, and \
                 {tool_name} would start a NEW conversation — which is public — and send text \
                 into it. The first time that happens the user has to see the exact payload and \
                 approve it, and a call made from inside a script never reaches the approval. \
                 Call {tool_name} directly instead of from `execute_code`."
            ));
        }
    };
    let row = crate::session::session_manager::SessionManager::instance()
        .get_session(&target, false)
        .await
        .ok()?;
    if !crate::privacy::crossing::needs_disclosure(
        cap.tier(),
        row.privacy_tier,
        caller_session_id,
        &target,
    ) {
        return None;
    }
    tracing::warn!(
        counter.biorouter.workspace_crossing_uninspected_refused = 1,
        tool_name = %tool_name,
        target_session = %target,
        boundary = ?boundary,
        "Refused a first-crossing workspace write at a boundary no inspector sees"
    );
    Some(format!(
        "Refused: this conversation runs on a model hosted inside your institution, and \
         {tool_name} would send text to conversation {target}, which does not. The first \
         time that happens the user has to see the exact payload and approve it — and a \
         call made from inside a script never reaches the approval. Call {tool_name} \
         directly instead of from `execute_code`; after the user approves once, this pair \
         of conversations stops asking."
    ))
}

pub struct WorkspaceCrossingInspector {
    provider: crate::agents::types::SharedProvider,
}

impl WorkspaceCrossingInspector {
    pub fn new(provider: crate::agents::types::SharedProvider) -> Self {
        Self { provider }
    }

    async fn inspect_with_pinned_capability(
        &self,
        tool_requests: &[ToolRequest],
        session: &Session,
        capability: Option<crate::privacy::CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        // Reject non-workspace-write batches before sampling a provider or
        // touching session storage. This inspector is on every tool batch.
        let candidates: Vec<(&ToolRequest, Option<String>, String)> = tool_requests
            .iter()
            .filter_map(|request| {
                let tool_call = request.tool_call.as_ref().ok()?;
                let args = tool_call.arguments.as_ref()?;
                let (target, payload) = crossing_disclosure(&tool_call.name, args)?;
                Some((request, target, payload))
            })
            .collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // A bridge grant supplies the capability it sampled at issue time. The
        // ordinary agent path has no pinned value, so sample exactly once for
        // the whole batch rather than letting calls observe different models.
        let cap = match capability {
            Some(capability) => capability,
            None => crate::privacy::CallCapability::sample(&self.provider).await,
        };
        if !cap.enforced() || !cap.tier().is_private() {
            return Ok(Vec::new());
        }

        let session_manager = crate::session::session_manager::SessionManager::instance();
        let mut results = Vec::new();
        for (request, target, payload) in candidates {
            // ⚠ `None` is `workspace_open { new: { prompt } }`: the conversation
            // this write lands in does not exist yet. There is no row to resolve
            // and no pair to look up, and neither absence is a reason to stay
            // quiet — a session minted a moment from now is created public and
            // has never crossed with this caller, so a private caller seeding one
            // is a first crossing outright. Skipping it here is precisely the
            // hole this arm closes: the card fired for `workspace_send_prompt`
            // into that same session one call later, and never for the call that
            // created and seeded it.
            let target = match target {
                Some(target) => {
                    // Metadata only: resolving the disclosure boundary must never
                    // load the target conversation. An absent row is left to the
                    // handler's anti-oracle refusal rather than disclosed here.
                    let Ok(row) = session_manager.get_session(&target, false).await else {
                        continue;
                    };
                    if !crate::privacy::crossing::needs_disclosure(
                        cap.tier(),
                        row.privacy_tier,
                        &session.id,
                        &target,
                    ) {
                        continue;
                    }
                    target
                }
                None => {
                    if !crate::privacy::visibility::requires_first_crossing_approval(
                        cap.tier(),
                        crate::privacy::SessionClassification::Public,
                    ) {
                        continue;
                    }
                    "a new conversation this call is about to create".to_string()
                }
            };
            tracing::warn!(
                counter.biorouter.workspace_tier_crossing_disclosed = 1,
                tool_request_id = %request.id,
                target_session = %target,
                "Private-to-public workspace write escalated to approval (issue #56)"
            );
            results.push(InspectionResult {
                tool_request_id: request.id.clone(),
                action: InspectionAction::RequireApproval(Some(format!(
                    // Keep the untrusted payload last and fenced. The desktop
                    // card collapses whitespace, so prose after it would blur
                    // the boundary between quoted text and Biorouter's warning.
                    "🔒 This conversation is on a model hosted inside your institution. \
                     It is about to send text to conversation {target}, which is NOT. \
                     Approving sends it now, and stops asking for this pair of \
                     conversations. This confirmation appears in every permission mode, \
                     including Fully Automatic. ⟪WHAT IT WOULD SEND⟫ {payload} ⟪END⟫"
                ))),
                reason: format!("Private-to-public workspace write into {target}"),
                confidence: 1.0,
                inspector_name: "workspace_tier_crossing".to_string(),
                finding_id: Some(format!("WSXING-{}", Uuid::new_v4().simple())),
            });
        }
        Ok(results)
    }
}

#[async_trait]
impl ToolInspector for WorkspaceCrossingInspector {
    fn name(&self) -> &'static str {
        "workspace_tier_crossing"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        _biorouter_mode: BioRouterMode,
        session: &Session,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_with_pinned_capability(tool_requests, session, None)
            .await
    }

    async fn inspect_with_capability(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        _biorouter_mode: BioRouterMode,
        session: &Session,
        capability: Option<crate::privacy::CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_with_pinned_capability(tool_requests, session, capability)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(json: serde_json::Value) -> rmcp::model::JsonObject {
        json.as_object().unwrap().clone()
    }

    fn set_tools_request(id: &str, arguments: serde_json::Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                meta: None,
                name: "workspace__workspace_set_tools".into(),
                arguments: Some(args(arguments)),
                task: None,
            }),
            // `ToolRequest` has FOUR fields (`conversation/message.rs:65-76`):
            // `id`, `tool_call`, `metadata`, `tool_meta`. Omitting the last two
            // is E0063. The precedents build all four —
            // `tool_inspection.rs:352-353` and `security/sensitive_ops.rs:699+`.
            metadata: None,
            tool_meta: None,
        }
    }

    /// The inspector as the agent registers it, over `sm` and an unbound
    /// provider (which samples Public).
    fn mutation_inspector(
        sm: std::sync::Arc<crate::session::SessionManager>,
    ) -> WorkspaceMutationInspector {
        WorkspaceMutationInspector::new(std::sync::Arc::new(tokio::sync::Mutex::new(None)), sm)
    }

    /// A caller and a real target in one fresh store.
    async fn caller_and_target(
        temp: &tempfile::TempDir,
    ) -> (
        std::sync::Arc<crate::session::SessionManager>,
        Session,
        Session,
    ) {
        let sm = std::sync::Arc::new(crate::session::SessionManager::new(
            temp.path().to_path_buf(),
        ));
        let mut rows = Vec::new();
        for name in ["caller", "target"] {
            rows.push(
                sm.create_session(
                    temp.path().to_path_buf(),
                    name.into(),
                    crate::session::session_manager::SessionType::User,
                )
                .await
                .unwrap(),
            );
        }
        let target = rows.pop().unwrap();
        let caller = rows.pop().unwrap();
        (sm, caller, target)
    }

    /// **F4, the measured case: a built-in capability named for removal is
    /// refused before any approval is raised.**
    ///
    /// The QA run approved the card "This change removes 'autovisualiser',
    /// which the user configured explicitly" and was then told
    /// ``​`autovisualiser` is a built-in Biorouter capability … cannot be enabled
    /// or disabled through Extension Manager``. The change was never possible,
    /// so the card must never appear, in any mode, and the model must be handed
    /// the extension manager's own sentence instead.
    ///
    /// `workspace` is the machine-independent half of the proof: it is on the
    /// compiled security-relevant list, so the OLD inspector raised a card for
    /// it on every machine, and it is a platform capability, so the handler has
    /// always refused it after that card. `autovisualiser` is the QA's own
    /// name; whether the old code carded it depended on the machine's config,
    /// which is exactly why it cannot carry the proof alone.
    #[tokio::test]
    async fn a_built_in_capability_is_refused_before_any_approval() {
        let temp = tempfile::TempDir::new().unwrap();
        let (sm, caller, target) = caller_and_target(&temp).await;
        let inspector = mutation_inspector(sm);

        for name in ["workspace", "autovisualiser", "Auto Visualiser"] {
            let request = set_tools_request(
                "call-builtin",
                serde_json::json!({ "session_id": target.id, "remove_extensions": [name] }),
            );
            for mode in [BioRouterMode::Auto, BioRouterMode::Approve] {
                let results = inspector
                    .inspect(std::slice::from_ref(&request), &[], mode, &caller)
                    .await
                    .unwrap();
                assert!(
                    !results
                        .iter()
                        .any(|r| matches!(r.action, InspectionAction::RequireApproval(_))),
                    "{name}: a card was raised for a change that cannot be made: {results:?}"
                );
                let [refusal] = results.as_slice() else {
                    panic!("{name}: expected exactly one verdict, got {results:?}");
                };
                assert_eq!(refusal.action, InspectionAction::Deny, "{name}");
                assert_eq!(refusal.inspector_name, WORKSPACE_MUTATION_INSPECTOR_NAME);
                assert_eq!(
                    refusal.reason,
                    format!(
                        "Error: {}",
                        crate::agents::extension_manager::capability_management_error(name).message
                    ),
                    "{name}: not the extension manager's own sentence"
                );
            }
        }
    }

    // "…and a REAL extension removal still asks first" lives in
    // `workspace_extension`'s tests
    // (`a_real_extension_removal_passes_the_pre_flight_and_still_asks_first`):
    // it needs a live target agent under a process-unique session id, and only
    // that module's `seeded_target` can mint one. A temp store's second row is
    // `<day>_2` in every test, and the agent registry is process-wide.

    /// A call the handler would refuse for any other reason is not carded
    /// either — the pre-flight is the handler's whole resolve phase, not a
    /// capability special case. A made-up target, and a primary knowledge base
    /// outside the set it is meant to point into.
    #[tokio::test]
    #[serial_test::serial(workspace_services)]
    async fn any_change_the_handler_would_refuse_is_refused_before_the_card() {
        let temp = tempfile::TempDir::new().unwrap();
        let (sm, caller, target) = caller_and_target(&temp).await;
        let inspector = mutation_inspector(sm);

        // Absent target: the write gate's anti-oracle sentence, not a card for
        // a skill grant into nowhere.
        let absent = set_tools_request(
            "call-absent",
            serde_json::json!({ "session_id": "no-such-conversation", "add_skills": ["x"] }),
        );
        let results = inspector
            .inspect(
                std::slice::from_ref(&absent),
                &[],
                BioRouterMode::Auto,
                &caller,
            )
            .await
            .unwrap();
        assert!(
            matches!(results.as_slice(), [r] if r.action == InspectionAction::Deny
                && r.reason.contains(&crate::privacy::refusal::workspace_out_of_reach())),
            "{results:?}"
        );

        // A skill grant (carded on its own) riding with an impossible KB target.
        crate::workspace_services::set_for_tests(Some(std::sync::Arc::new(
            crate::workspace_services::NullServices,
        )));
        let mixed = set_tools_request(
            "call-mixed",
            serde_json::json!({
                "session_id": target.id, "add_skills": ["single-cell"],
                "set_knowledge_bases": ["kb-a"], "primary_knowledge_base": "kb-b",
            }),
        );
        let results = inspector
            .inspect(
                std::slice::from_ref(&mixed),
                &[],
                BioRouterMode::Auto,
                &caller,
            )
            .await
            .unwrap();
        crate::workspace_services::clear_test_override();
        assert!(
            matches!(results.as_slice(), [r] if r.action == InspectionAction::Deny
                && r.reason.contains("primary_knowledge_base 'kb-b'")),
            "{results:?}"
        );
    }

    #[test]
    fn adding_a_process_spawning_extension_always_confirms() {
        for name in ["developer", "computercontroller", "code_execution"] {
            let reason = set_tools_confirmation_reason(&args(serde_json::json!({
                "session_id": "s-target",
                "add_extensions": [name],
            })));
            assert!(reason.is_some(), "adding {name} must confirm");
            assert!(reason.unwrap().contains(name));
        }
    }

    #[test]
    fn removing_a_security_relevant_extension_always_confirms() {
        let reason = set_tools_confirmation_reason(&args(serde_json::json!({
            "session_id": "s-target",
            "remove_extensions": ["workspace"],
        })));
        assert!(reason.unwrap().contains("workspace"));
    }

    #[test]
    fn an_ordinary_change_does_not_confirm_through_this_inspector() {
        // `todo` is neither process-spawning nor security-relevant and is not
        // operator-persisted in a default config: the normal permission grading
        // decides, exactly as for any other non-read tool.
        assert!(set_tools_confirmation_reason(&args(serde_json::json!({
            "session_id": "s-target",
            "add_extensions": ["todo"],
        })))
        .is_none());
        // A knowledge-base swap changes no capability.
        assert!(set_tools_confirmation_reason(&args(serde_json::json!({
            "session_id": "s-target",
            "set_knowledge_bases": ["kb-a"],
        })))
        .is_none());
    }

    #[test]
    fn only_set_tools_is_inspected_and_both_name_forms_match() {
        assert!(is_set_tools_call("workspace__workspace_set_tools"));
        assert!(is_set_tools_call("workspace_set_tools"));
        assert!(!is_set_tools_call("workspace_list"));
        assert!(!is_set_tools_call("workspace__workspace_send_prompt"));
    }

    #[tokio::test]
    async fn the_inspector_requires_approval_in_every_mode() {
        use crate::config::BioRouterMode;

        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(crate::session::SessionManager::new(
            temp.path().to_path_buf(),
        ));
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "caller".into(),
                crate::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        // A REAL target: since F4 the inspector asks whether the change can be
        // made before it raises a card, and a made-up id is refused by the
        // write gate (the anti-oracle sentence) rather than confirmed.
        let target = sm
            .create_session(
                temp.path().to_path_buf(),
                "target".into(),
                crate::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        // A skill grant, not `add_extensions: ["developer"]`: every mode must
        // confirm it just the same, and unlike an extension it resolves on
        // every machine — `developer`'s entry comes from the real config, where
        // the GUI commonly writes it `enabled: false`, and the pre-flight would
        // then (correctly) refuse the pinned-off extension instead of asking.
        let request = set_tools_request(
            "call-1",
            serde_json::json!({ "session_id": target.id, "add_skills": ["ucsf-hpc"] }),
        );

        // ALL FOUR real variants (`config/biorouter_mode.rs:7-12`). Auto is the
        // one that matters most — it is where the permission inspector allows
        // everything — but Approve and SmartApprove are the modes decision 1's
        // guarantee is actually *about*, so they must be in the list, not
        // implied by an "etc".
        let inspector = mutation_inspector(std::sync::Arc::clone(&sm));
        for mode in [
            BioRouterMode::Auto,
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
            BioRouterMode::Chat,
        ] {
            let results = inspector
                .inspect(std::slice::from_ref(&request), &[], mode, &session)
                .await
                .unwrap();
            assert_eq!(results.len(), 1, "mode {mode:?} produced no result");
            assert!(matches!(
                results[0].action,
                crate::tool_inspection::InspectionAction::RequireApproval(Some(_))
            ));
        }
    }

    #[test]
    fn a_mis_cased_removal_still_confirms() {
        // `Agent::remove_extension` normalizes before removing
        // (extension_manager.rs:976-981), so "Workspace" really does strip the
        // audit-trail extension. A raw-string check would see no match and
        // return None — no confirmation, in any mode.
        for spelling in ["Workspace", "WORKSPACE", "work space"] {
            let reason = set_tools_confirmation_reason(&args(serde_json::json!({
                "session_id": "s-target",
                "remove_extensions": [spelling],
            })));
            assert!(
                reason.is_some(),
                "removal spelled {spelling:?} must confirm"
            );
        }
    }

    #[test]
    fn a_provider_switch_always_confirms() {
        // Decision b: the target's whole stored history goes to whatever
        // endpoint the new provider names, and `allows_unlisted_models` waves
        // the model check through for custom providers.
        let reason = set_tools_confirmation_reason(&args(serde_json::json!({
            "session_id": "s-target",
            "provider": "my-custom-proxy",
            "model": "anything",
        })));
        assert!(reason.unwrap().contains("my-custom-proxy"));
    }

    #[test]
    fn a_skill_grant_confirms() {
        let reason = set_tools_confirmation_reason(&args(serde_json::json!({
            "session_id": "s-target",
            "add_skills": ["ucsf-hpc"],
        })));
        assert!(reason.unwrap().contains("ucsf-hpc"));
    }

    /// The two config-derived branches — [`add_shape_risk`] and the
    /// operator-authored set — are the ones a machine's real
    /// `~/.config/biorouter/config.yaml` decides, so the tests above (which go
    /// through the public wrappers) can only ever exercise whichever branch
    /// this developer's config happens to select. On a default install that is
    /// `Opaque` + an empty operator set, i.e. neither. The `_with` seam feeds
    /// those two facts in directly, so every branch is covered on every machine
    /// and no test ever reads — let alone writes — the user's real config.
    ///
    /// A raw (un-normalized) operator set is passed on purpose: normalizing it
    /// is part of the behaviour under test.
    fn no_operator_extensions() -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }

    fn operator_extensions(names: &[&str]) -> std::collections::HashSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_structurally_process_spawning_extension_confirms_by_shape() {
        // `lab-mcp-server` is on NO name list. A `Stdio`/`InlinePython` entry
        // spawns a child process on the target's machine, and that is visible
        // in the config SHAPE alone — which is the whole reason
        // `add_shape_risk` exists. If it were reduced to the name list, an
        // arbitrary stdio MCP server could be granted to another conversation
        // silently, in Auto mode.
        let reason = set_tools_confirmation_reason_with(
            &args(serde_json::json!({
                "session_id": "s-target",
                "add_extensions": ["lab-mcp-server"],
            })),
            |_| AddShapeRisk::ProcessSpawning,
            &no_operator_extensions(),
        )
        .expect("a process-spawning shape must confirm even off the name list");
        assert!(reason.contains("process-spawning"), "{reason}");
        assert!(reason.contains("lab-mcp-server"), "{reason}");
    }

    #[test]
    fn a_network_egress_extension_confirms_by_shape() {
        // `Sse` / `StreamableHttp` carry a uri plus envs/headers: granting one
        // points another conversation's traffic at an arbitrary remote MCP
        // endpoint, with credentials. Distinct wording, because the risk is
        // exfiltration rather than execution.
        let reason = set_tools_confirmation_reason_with(
            &args(serde_json::json!({
                "session_id": "s-target",
                "add_extensions": ["remote-notes"],
            })),
            |_| AddShapeRisk::NetworkEgress,
            &no_operator_extensions(),
        )
        .expect("a network-egress shape must confirm");
        assert!(reason.contains("remote endpoint"), "{reason}");
        assert!(reason.contains("remote-notes"), "{reason}");
        assert!(
            !reason.contains("process-spawning"),
            "egress must not be reported as execution: {reason}"
        );
    }

    #[test]
    fn an_opaque_shape_falls_through_to_the_name_list() {
        // `Builtin` / `Platform` / `Frontend` and unknown names say nothing
        // about capability, so the name list is the only thing left. Both
        // outcomes are pinned: `developer` still confirms, an unknown in-process
        // extension does not (or every ordinary tool change would prompt, and
        // the confirmation would stop meaning anything).
        let opaque = |_: &str| AddShapeRisk::Opaque;
        assert!(
            set_tools_confirmation_reason_with(
                &args(serde_json::json!({
                    "session_id": "s-target",
                    "add_extensions": ["developer"],
                })),
                opaque,
                &no_operator_extensions(),
            )
            .is_some(),
            "the name list must still catch `developer` when the shape is opaque"
        );
        assert!(
            set_tools_confirmation_reason_with(
                &args(serde_json::json!({
                    "session_id": "s-target",
                    "add_extensions": ["lab-mcp-server"],
                })),
                opaque,
                &no_operator_extensions(),
            )
            .is_none(),
            "an opaque, unlisted extension must not confirm through this inspector"
        );
    }

    #[test]
    fn removing_an_operator_authored_extension_confirms() {
        // An entry the operator wrote into the config file is a human decision;
        // an agent undoing it from another conversation must surface. An
        // extension that is merely *available* is not operator-authored and
        // stays on the ordinary permission path.
        let reason = set_tools_confirmation_reason_with(
            &args(serde_json::json!({
                "session_id": "s-target",
                "remove_extensions": ["lab-mcp-server"],
            })),
            |_| AddShapeRisk::Opaque,
            &operator_extensions(&["lab-mcp-server"]),
        )
        .expect("removing an operator-authored extension must confirm");
        assert!(reason.contains("configured explicitly"), "{reason}");
        assert!(reason.contains("lab-mcp-server"), "{reason}");

        assert!(
            set_tools_confirmation_reason_with(
                &args(serde_json::json!({
                    "session_id": "s-target",
                    "remove_extensions": ["lab-mcp-server"],
                })),
                |_| AddShapeRisk::Opaque,
                &no_operator_extensions(),
            )
            .is_none(),
            "a removal of something the operator never wrote must not confirm here"
        );
    }

    #[test]
    fn a_mis_cased_operator_authored_removal_still_confirms() {
        // BOTH sides go through `norm`, because the executor does. The config
        // file holds whatever the operator typed; the model sends whatever it
        // read back. `ExtensionManager::remove_extension` normalizes before
        // removing, so these are the same extension and the check must agree —
        // otherwise the removal happens with no confirmation, in any mode.
        let reason = set_tools_confirmation_reason_with(
            &args(serde_json::json!({
                "session_id": "s-target",
                "remove_extensions": ["lab-mcp-server"],
            })),
            |_| AddShapeRisk::Opaque,
            &operator_extensions(&["Lab-MCP-Server"]),
        );
        assert!(
            reason.is_some(),
            "a differently-cased operator-authored name must still confirm"
        );
    }

    #[test]
    fn workspace_open_confirms_a_structurally_process_spawning_grant() {
        // The open path shares `add_extension_reason`, so it must inherit the
        // shape check too — otherwise the easier route (mint a session with the
        // grant baked in) is the unguarded one.
        let reason = open_confirmation_reason_with(
            &args(serde_json::json!({
                "new": { "working_dir": "/tmp", "extensions": ["lab-mcp-server"] },
            })),
            |_| AddShapeRisk::ProcessSpawning,
        )
        .expect("a process-spawning shape must confirm on the open path too");
        assert!(
            reason.starts_with("starts a new conversation that has"),
            "{reason}"
        );
        assert!(reason.contains("lab-mcp-server"), "{reason}");
    }

    #[test]
    fn workspace_open_granting_a_process_spawning_extension_confirms() {
        assert!(is_workspace_open_call("workspace__workspace_open"));
        let reason = open_confirmation_reason(&args(serde_json::json!({
            "new": { "working_dir": "/tmp", "extensions": ["developer"], "prompt": "go" },
        })));
        assert!(reason.unwrap().contains("developer"));
        // Opening an EXISTING conversation grants nothing and must not confirm.
        assert!(open_confirmation_reason(&args(serde_json::json!({
            "session_id": "s-existing",
        })))
        .is_none());
    }

    /// **The disclosure is REGISTERED.** Every assertion in this module and in
    /// `tests/workspace_crossing_disclosure.rs` builds the inspector by hand, so
    /// all of them stay green if `create_tool_inspection_manager` stops adding
    /// it — which is the failure this campaign has shipped five times under a
    /// different name: the mechanism is built, the entry point is never called,
    /// and every unit test passes because the unit is correct.
    ///
    /// A source scan, not a behavioural test, because the thing being asserted
    /// is an absence elsewhere. It reads the production half of `agent.rs` (cut
    /// at its test module, with a negative control proving the cut landed) and
    /// requires the constructor call to be there.
    #[test]
    fn the_disclosure_inspector_is_registered_in_the_agent_loop() {
        const AGENT: &str = include_str!("agent.rs");
        // ⚠ The LAST such module, not the first. `agent.rs` carries a nested
        // `#[cfg(test)] mod tests` inside another module some 3,000 lines above
        // the file-level one, and cutting at the first match put the whole
        // inspector registry on the "tests" side — while the negative control
        // below still passed, because its marker sits below both. Measured, not
        // reasoned about: the first-match version of this scan failed on a tree
        // where the registration was present and correct.
        let cut = AGENT
            .match_indices("mod tests {")
            .filter_map(|(i, _)| {
                let before = AGENT.get(..i)?.trim_end();
                let before = before
                    .strip_suffix("pub(crate)")
                    .unwrap_or(before)
                    .trim_end();
                let before = before.strip_suffix("pub").unwrap_or(before).trim_end();
                before
                    .ends_with("#[cfg(test)]")
                    .then(|| before.len() - "#[cfg(test)]".len())
            })
            .last()
            .expect("agent.rs has a `#[cfg(test)]` test module, so this scan cuts it there");
        let (production, tests) = AGENT.split_at(cut);

        // The control, FIRST — otherwise a cut that landed at the end of the
        // file would make the assertion below pass vacuously.
        // `agent_with_one_extension_for_tests` is spelled only by the file-level
        // test module.
        assert!(
            !production.contains("agent_with_one_extension_for_tests"),
            "the cut did not remove the test module, so the assertion below proves nothing"
        );
        assert!(
            tests.contains("agent_with_one_extension_for_tests"),
            "the cut removed more than the test module"
        );

        assert!(
            production.contains("WorkspaceCrossingInspector::new("),
            "the first-crossing disclosure has no production registration: the \
             inspector exists, the agent loop never adds it, and every test that \
             constructs one by hand still passes"
        );
    }

    // ---------------------------------------------------------------- the
    // first-crossing disclosure. `crossing_payload` is the pure half —
    // `WorkspaceCrossingInspector::inspect` is the two lookups around it, and
    // the ledger it consults has its own tests in `privacy::crossing`.

    #[test]
    fn a_send_prompt_payload_is_the_text_and_the_mode() {
        let (target, payload) = crossing_payload(
            "workspace_send_prompt",
            &args(serde_json::json!({
                "session_id": "s-public",
                "mode": "steer",
                "text": "drop what you are doing and summarise the cohort",
            })),
        )
        .expect("a send_prompt with text has a payload to disclose");
        assert_eq!(target, "s-public");
        assert!(payload.contains("mode steer"), "{payload}");
        // The WHOLE text, verbatim. A disclosure that showed a summary would be
        // the same as no disclosure — the user is being asked to judge exactly
        // what leaves for the public model.
        assert!(
            payload.contains("drop what you are doing and summarise the cohort"),
            "{payload}"
        );
    }

    /// **The two routes into another conversation must agree.**
    ///
    /// The measured hole: from a private chat,
    /// `workspace_open {"new":{"kind":"user","working_dir":"/tmp","prompt":"…"}}`
    /// succeeded in silence, while `workspace_send_prompt` carrying the SAME
    /// text into the session that call had just created raised the 🔒 card. Two
    /// ways to put caller-chosen text into a public conversation, one of them
    /// guarded — and the unguarded one was also the shorter one, because it does
    /// the create and the write in a single call.
    ///
    /// `crossing_payload` bailed on its first line for `workspace_open`: it
    /// requires `args["session_id"]`, and a call that MINTS its target has none.
    /// Asserted against [`crossing_disclosure`] because that is the one function
    /// all three halves — card, boundary refusal, and the record that stops the
    /// asking — now ask.
    #[test]
    fn opening_a_new_conversation_with_a_prompt_discloses_the_same_text_send_prompt_would() {
        const TEXT: &str = "summarise the cohort in /data/private and write it up";

        let (sent_target, sent) = crossing_disclosure(
            "workspace_send_prompt",
            &args(serde_json::json!({
                "session_id": "s-public", "mode": "turn", "text": TEXT,
            })),
        )
        .expect("send_prompt has always disclosed");
        assert_eq!(sent_target.as_deref(), Some("s-public"));
        assert!(sent.contains(TEXT), "{sent}");

        for name in ["workspace_open", "workspace__workspace_open"] {
            let (opened_target, opened) = crossing_disclosure(
                name,
                &args(serde_json::json!({
                    "new": { "kind": "user", "working_dir": "/tmp", "prompt": TEXT },
                })),
            )
            .unwrap_or_else(|| {
                panic!("{name} with a prompt writes into another conversation and must disclose")
            });
            // No target: the conversation does not exist yet, which is what the
            // caller must read as "treat this as a first crossing outright".
            assert!(
                opened_target.is_none(),
                "a call that mints its own target cannot name one: {opened_target:?}"
            );
            // The WHOLE text, verbatim, exactly as the send_prompt route shows
            // it — the user is judging what leaves for the public model.
            assert!(opened.contains(TEXT), "{opened}");
        }
    }

    /// The other half of the same rule: an open that writes NOTHING has nothing
    /// to disclose, and a card with an empty payload teaches the user to click
    /// through them. It must also not be recordable — that would consume the
    /// pair's one disclosure without ever having shown one.
    #[test]
    fn opening_a_conversation_without_a_prompt_discloses_nothing() {
        // A new session with no seeded turn.
        assert!(crossing_disclosure(
            "workspace_open",
            &args(serde_json::json!({ "new": { "kind": "user", "working_dir": "/tmp" } })),
        )
        .is_none());
        // A whitespace-only prompt is not a payload either.
        assert!(crossing_disclosure(
            "workspace_open",
            &args(serde_json::json!({ "new": { "prompt": "   " } })),
        )
        .is_none());
        // And opening an EXISTING conversation is a read, not a write.
        assert!(crossing_disclosure(
            "workspace_open",
            &args(serde_json::json!({ "session_id": "s-public", "placement": "tab" })),
        )
        .is_none());
    }

    #[test]
    fn the_prefixed_tool_name_is_recognised_too() {
        // Both spellings reach dispatch (extension-advertised tools are prefixed,
        // and the loop tolerates models that strip the prefix), so a disclosure
        // keyed on one of them is a disclosure with a one-word bypass.
        for name in ["workspace_send_prompt", "workspace__workspace_send_prompt"] {
            assert!(
                crossing_payload(
                    name,
                    &args(serde_json::json!({
                        "session_id": "s", "mode": "note", "text": "x"
                    })),
                )
                .is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn set_tools_discloses_what_it_would_change_and_close_discloses_nothing() {
        let (_, payload) = crossing_payload(
            "workspace_set_tools",
            &args(serde_json::json!({
                "session_id": "s-public",
                "add_skills": ["single-cell"],
                "provider": "anthropic",
            })),
        )
        .expect("a set_tools that changes something has a payload");
        assert!(payload.contains("add_skills: single-cell"), "{payload}");
        assert!(payload.contains("provider: anthropic"), "{payload}");

        // A no-op set_tools has nothing to disclose, so it must not raise a card
        // the user cannot act on.
        assert!(crossing_payload(
            "workspace_set_tools",
            &args(serde_json::json!({ "session_id": "s-public" })),
        )
        .is_none());

        // `workspace_close` carries no text at all. It is a write, and it is
        // gated by the tier like the others — but there is nothing for a
        // *disclosure* to show, and a card with an empty payload teaches the
        // user to click through them.
        assert!(crossing_payload(
            "workspace_close",
            &args(serde_json::json!({ "session_id": "s-public", "scope": "turn" })),
        )
        .is_none());
    }

    /// **The ledger must not be poisonable by a write that disclosed nothing.**
    ///
    /// `workspace_set_tools { set_knowledge_bases: [] }` is an ACCEPTED change —
    /// `set_knowledge_bases` is an `Option<Vec<_>>`, so `Some(vec![])` is a real
    /// request to clear the target's bases, and `handle_set_tools` applies it and
    /// reports success. Neither workspace inspector raises a card for it, because
    /// there is no payload to show.
    ///
    /// A record half that fired on "the write succeeded" alone would therefore
    /// let a private caller consume a public target's one disclosure with a call
    /// the user never saw, and the very next `workspace_send_prompt` into that
    /// conversation would ship its payload to the public model in silence. That
    /// is why `record_crossing_if_disclosed` asks THIS function rather than
    /// re-deriving what counts as a payload.
    #[test]
    fn a_change_with_nothing_to_disclose_has_no_payload_to_record() {
        assert!(
            crossing_payload(
                "workspace_set_tools",
                &args(serde_json::json!({
                    "session_id": "s-public", "set_knowledge_bases": []
                })),
            )
            .is_none(),
            "an empty set_knowledge_bases has no payload, so a write carrying one must \
             not be able to mark a pair as crossed"
        );
        // The control: the same tool with something to show is still covered, so
        // this is not "the predicate answers None for set_tools".
        assert!(crossing_payload(
            "workspace_set_tools",
            &args(serde_json::json!({
                "session_id": "s-public", "set_knowledge_bases": ["ms-cohort"]
            })),
        )
        .is_some());
    }

    #[test]
    fn a_call_naming_no_target_or_no_text_has_no_payload() {
        assert!(crossing_payload(
            "workspace_send_prompt",
            &args(serde_json::json!({ "mode": "note", "text": "x" })),
        )
        .is_none());
        assert!(crossing_payload(
            "workspace_send_prompt",
            &args(serde_json::json!({ "session_id": "s", "mode": "note" })),
        )
        .is_none());
    }

    /// The inspector is INERT for a public-capability caller, and that is a
    /// property rather than an optimisation: a public caller cannot write into a
    /// private conversation at all (that is `may_write` refusing), so a card
    /// here would announce a crossing that is not going to happen.
    #[tokio::test]
    async fn a_public_capability_caller_raises_no_disclosure() {
        let inspector =
            WorkspaceCrossingInspector::new(std::sync::Arc::new(tokio::sync::Mutex::new(None)));
        let request = ToolRequest {
            id: "req-1".into(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                name: "workspace_send_prompt".into(),
                arguments: Some(args(serde_json::json!({
                    "session_id": "s-public", "mode": "note", "text": "x"
                }))),
                meta: None,
                task: None,
            }),
            metadata: Default::default(),
            tool_meta: Default::default(),
        };
        // An unbound provider samples Public — the safe direction for every gate
        // that reads a capability, and what this inspector must treat as "not my
        // business" rather than as "unknown, so ask".
        let results = inspector
            .inspect(
                std::slice::from_ref(&request),
                &[],
                BioRouterMode::Auto,
                &Session::default(),
            )
            .await
            .unwrap();
        assert!(results.is_empty(), "{results:?}");
    }
}
