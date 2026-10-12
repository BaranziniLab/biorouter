#[cfg(test)]
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

use crate::agents::extension::{ExtensionClassification, ExtensionInfo};
use crate::context_budget::{
    fit_context_blocks, injection_budget_tokens, BudgetReport, ContextBlock,
};
use crate::hints::load_hints::{load_hint_files, AGENTS_MD_FILENAME, BIOROUTER_HINTS_FILENAME};
use crate::{
    config::{BioRouterMode, Config},
    prompt_template,
    utils::sanitize_unicode_tags,
};
use std::path::Path;

/// How finely the system prompt states the clock.
///
/// Local (not UTC) either way, matching the MOIM `<info-msg>` clock so the
/// model never sees two contradictory timezones. Computed fresh at each
/// `build()` (not frozen at construction) so a long-lived agent's clock never
/// goes stale. The per-request MOIM carries the minute-precise time, so the
/// coarse clock here costs the model nothing it cannot read elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptClock {
    /// `2026-07-12 14:00`: byte-identical within the hour, so a hosted
    /// provider's multi-session prompt cache still hits.
    Hour,
    /// `2026-07-12`, for a local engine (Llama Server, Ollama). The clock sits
    /// near the TOP of the prompt (character ~605), so a change there throws
    /// away everything after it in the server's prompt cache. Measured on
    /// qwen3.8:27b (MLX, 2026-10-10): the first request after an hour boundary
    /// reused 8,191 of 30,745 tokens and spent 107 s re-reading the rest — a
    /// stall a hosted model never shows, once an hour, in every long chat.
    Day,
}

fn render_prompt_clock(now: chrono::DateTime<chrono::Local>, clock: PromptClock) -> String {
    match clock {
        PromptClock::Hour => now.format("%Y-%m-%d %H:00").to_string(),
        PromptClock::Day => now.format("%Y-%m-%d").to_string(),
    }
}

/// Is inference for this provider a local engine, whose prompt cache a long
/// prefill must refill? One of the two local providers by name (a LAN Ollama
/// prefills on the same kind of hardware), or any provider whose instance runs
/// on this machine (`providers::utils::runs_on_this_machine`).
pub fn is_local_engine(provider_name: &str, runs_on_this_machine: bool) -> bool {
    runs_on_this_machine
        || matches!(
            provider_name.to_ascii_lowercase().as_str(),
            "llamacpp" | "ollama"
        )
}

/// BR-3: which system-prompt variant to render for the active model.
///
/// One fixed `system.md` served 43+ providers of wildly varying capability:
/// strong models paid for scaffolding they don't need, and small/local models
/// got too little. Rather than the "one prompt file per model" sprawl the
/// review warned against, variants are kept intentionally minimal — a shared
/// base (`system.md`, the strong-model default) plus at most one small overlay.
/// The `Default` variant renders the base byte-identically, so strong models
/// (and their multi-session prompt cache key) are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptVariant {
    /// Strong commercial / institution-hosted models. Base `system.md` only.
    Default,
    /// Small, weak local models (Llama Server, Ollama). Base + a compact
    /// scaffolding overlay (`system_small_local.md`).
    SmallLocal,
}

impl PromptVariant {
    /// Choose a variant for `(provider_name, model_name)`.
    ///
    /// `runs_on_this_machine` is the provider INSTANCE's own answer — the
    /// caller passes `providers::utils::runs_on_this_machine(provider)`,
    /// which is true exactly while the resolved endpoint is loopback. It is
    /// what lets an `ollama_compatible` provider registered under any name
    /// (declarative providers register by `config.name`, and their registry
    /// metadata deliberately claims `runs_locally: false`) be prompted the
    /// same as the built-in `ollama` for the same engine and model (OLL-20).
    ///
    /// `BIOROUTER_SYSTEM_PROMPT_VARIANT` (`default` | `small_local`) pins the
    /// choice for testing / power users; otherwise the provider/model-keyed
    /// table decides, defaulting to [`PromptVariant::Default`].
    pub fn select(
        provider_name: &str,
        model_name: &str,
        runs_on_this_machine: bool,
    ) -> PromptVariant {
        if let Ok(pinned) = Config::global().get_param::<String>("BIOROUTER_SYSTEM_PROMPT_VARIANT")
        {
            match pinned.trim().to_ascii_lowercase().as_str() {
                "default" | "strong" => return PromptVariant::Default,
                "small_local" | "small" | "local" => return PromptVariant::SmallLocal,
                other => tracing::warn!(
                    variant = other,
                    "unknown BIOROUTER_SYSTEM_PROMPT_VARIANT; using the model-derived variant"
                ),
            }
        }
        select_variant_from_table(provider_name, model_name, runs_on_this_machine)
    }
}

/// The smallest parameter count, in billions, that gets the strong-model
/// prompt on a local engine. The two shipped local models are 27B (Qwen3.8)
/// and 31B (Gemma 4); the overlay's "single tool call per turn" and the rest
/// of its hand-holding under-use them, and every common size from here up
/// (27, 30, 31, 32, 34, 35, 70, 72 …) is a capable agent model.
const LARGE_LOCAL_MIN_BILLIONS: f64 = 24.0;

/// The parameter count a model name states, in billions: `27b`, `0.8b`,
/// `8x7b` (an MoE's total, 56). The LARGEST stated size wins, so an MoE tag
/// such as `30b-a3b` reads as its 30B total.
///
/// A size must stand alone between separators. That is what keeps `e4b`
/// (Gemma's "effective 4B") and `a3b` (an MoE's ACTIVE parameters) from being
/// read as sizes, and keeps a version number (`qwen3.8`) out of it.
fn model_parameter_billions(model_name: &str) -> Option<f64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?:^|[^a-z0-9.])(\d+(?:\.\d+)?)(?:x(\d+(?:\.\d+)?))?b(?:$|[^a-z0-9])")
            .expect("static regex")
    });
    let model = model_name.to_ascii_lowercase();
    re.captures_iter(&model)
        .filter_map(|captures| {
            let size: f64 = captures.get(1)?.as_str().parse().ok()?;
            match captures.get(2) {
                Some(experts) => Some(size * experts.as_str().parse::<f64>().ok()?),
                None => Some(size),
            }
        })
        .reduce(f64::max)
}

/// Provider/model → variant rules, first match wins, `Default` as the fallback.
/// Kept deliberately tiny (BR-3: "keep variants minimal / avoid sprawl").
///
/// A local engine gets the extra scaffolding unless its model name states a
/// LARGE model (`LARGE_LOCAL_MIN_BILLIONS`) or says `large`. A local engine is
/// one of the two local providers by name, or any provider whose instance
/// runs on this machine.
///
/// ⚠ This used to be a list of whole markers (`70b`, `72b`, `65b`, `34b`), so
/// both shipped local models — `qwen3.8:27b` and `gemma4:31b` — got the
/// small-model overlay, including its advice to use Developer `shell`
/// directly in a chat where that tool was only reachable through Code
/// Execution.
fn select_variant_from_table(
    provider_name: &str,
    model_name: &str,
    runs_on_this_machine: bool,
) -> PromptVariant {
    let model = model_name.to_ascii_lowercase();

    if is_local_engine(provider_name, runs_on_this_machine) {
        let large = model.contains("large")
            || model_parameter_billions(&model).is_some_and(|b| b >= LARGE_LOCAL_MIN_BILLIONS);
        return if large {
            PromptVariant::Default
        } else {
            PromptVariant::SmallLocal
        };
    }

    PromptVariant::Default
}

pub struct PromptManager {
    system_prompt_override: Option<String>,
    system_prompt_extras: Vec<String>,
    named_system_prompt_extras: BTreeMap<String, String>,
    /// When `Some`, pins the rendered clock (deterministic tests). When `None`,
    /// the clock is computed live at `build()` time.
    fixed_timestamp: Option<String>,
}

impl Default for PromptManager {
    fn default() -> Self {
        PromptManager::new()
    }
}

#[derive(Serialize)]
struct SystemPromptContext {
    capabilities: Vec<ExtensionInfo>,
    extensions: Vec<ExtensionInfo>,
    installed_extension_discovery_available: bool,
    marketplace_extension_search_available: bool,
    extension_state_change_available: bool,
    extension_package_install_available: bool,
    extension_package_delete_available: bool,
    extension_removal_available: bool,
    extension_resource_tools_available: bool,
    extension_resource_tools_directly_callable: bool,
    skill_load_available: bool,
    knowledge_search_available: bool,
    developer_shell_available: bool,
    developer_text_editor_available: bool,
    /// The tool is on the model's OWN tool list this turn, as opposed to
    /// reachable at all. Code Execution mode keeps `available` true while it
    /// moves Developer behind `code_execution__execute_code`; the small-local
    /// overlay used to read `available` alone and told a model to "use Developer
    /// `shell`" that it had never been offered — the reporter's
    /// `{"name": "shell", …}` text calls (repro r9).
    developer_shell_directly_callable: bool,
    developer_text_editor_directly_callable: bool,
    /// Reachable, but only from a `code_execution__execute_code` script: Code
    /// Execution mode is on, `execute_code` is on the list, and the Developer
    /// tool is not.
    developer_shell_via_code_execution: bool,
    developer_text_editor_via_code_execution: bool,
    code_execute_available: bool,
    current_date_time: String,
    biorouter_mode: BioRouterMode,
    is_autonomous: bool,
    enable_subagents: bool,
    code_execution_mode: bool,
    /// The planning gate runs for this conversation (`agents::planning_gate`):
    /// the "Working on Tasks" section then states exactly what it enforces.
    checklist_enforcement: bool,
}

pub struct SystemPromptBuilder<'a, M> {
    manager: &'a M,

    extensions_info: Vec<ExtensionInfo>,
    frontend_instructions: Option<String>,
    subagents_enabled: bool,
    hints: Option<String>,
    code_execution_mode: bool,
    checklist_enforcement: bool,
    variant: PromptVariant,
    local_engine: bool,
}

impl<'a> SystemPromptBuilder<'a, PromptManager> {
    /// BR-3: select the per-model prompt variant (default: strong-model base).
    pub fn with_prompt_variant(mut self, variant: PromptVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Whether the bound provider is a local engine ([`is_local_engine`]). The
    /// clock is then rendered by the day, not the hour, so the engine's prompt
    /// cache survives an hour boundary (see [`PromptClock::Day`]). Separate
    /// from the variant on purpose: both shipped local models are large enough
    /// for the `Default` variant, and they are the ones that pay the prefill.
    pub fn with_local_engine(mut self, local_engine: bool) -> Self {
        self.local_engine = local_engine;
        self
    }

    pub fn with_extension(mut self, extension: ExtensionInfo) -> Self {
        self.extensions_info.push(extension);
        self
    }

    pub fn with_extensions(mut self, extensions: impl Iterator<Item = ExtensionInfo>) -> Self {
        for extension in extensions {
            self.extensions_info.push(extension);
        }
        self
    }

    pub fn with_frontend_instructions(mut self, frontend_instructions: Option<String>) -> Self {
        self.frontend_instructions = frontend_instructions;
        self
    }

    pub fn with_code_execution_mode(mut self, enabled: bool) -> Self {
        self.code_execution_mode = enabled;
        self
    }

    /// Whether the planning gate enforces the checklist for this turn. Pass
    /// `planning_gate::enforcement_applies`, never a value of your own: the
    /// clause this renders is a description of that gate.
    pub fn with_checklist_enforcement(mut self, enabled: bool) -> Self {
        self.checklist_enforcement = enabled;
        self
    }

    pub fn with_hints(mut self, working_dir: &Path) -> Self {
        let config = Config::global();
        let hints_filenames = config
            .get_param::<Vec<String>>("CONTEXT_FILE_NAMES")
            .unwrap_or_else(|_| {
                vec![
                    BIOROUTER_HINTS_FILENAME.to_string(),
                    AGENTS_MD_FILENAME.to_string(),
                ]
            });
        let ignore_patterns = {
            let builder = ignore::gitignore::GitignoreBuilder::new(working_dir);
            builder.build().unwrap_or_else(|_| {
                ignore::gitignore::GitignoreBuilder::new(working_dir)
                    .build()
                    .expect("Failed to build default gitignore")
            })
        };

        let hints = load_hint_files(working_dir, &hints_filenames, &ignore_patterns);

        if !hints.is_empty() {
            self.hints = Some(hints);
        }
        self
    }

    pub fn with_enable_subagents(mut self, subagents_enabled: bool) -> Self {
        self.subagents_enabled = subagents_enabled;
        self
    }

    pub fn build(self) -> String {
        self.build_at(chrono::Local::now())
    }

    /// [`Self::build`] with the clock read as `now` — the seam that lets a test
    /// build twice "an hour apart".
    fn build_at(self, now: chrono::DateTime<chrono::Local>) -> String {
        let Self {
            manager,
            extensions_info,
            frontend_instructions,
            subagents_enabled,
            hints,
            code_execution_mode,
            checklist_enforcement,
            variant,
            local_engine,
        } = self;
        let (extensions_info, hints) =
            prepare_injected_context(extensions_info, frontend_instructions, hints);
        let config = Config::global();
        let biorouter_mode = config.get_biorouter_mode().unwrap_or(BioRouterMode::Auto);
        let clock = if local_engine {
            PromptClock::Day
        } else {
            PromptClock::Hour
        };
        let mut context = build_system_prompt_context(
            manager,
            extensions_info,
            biorouter_mode,
            subagents_enabled,
            code_execution_mode,
            render_prompt_clock(now, clock),
        );
        context.checklist_enforcement = checklist_enforcement;
        let base_prompt = render_base_prompt(manager, variant, &context);
        append_system_prompt_extras(manager, base_prompt, hints, biorouter_mode)
    }
}

fn prepare_injected_context(
    mut extensions_info: Vec<ExtensionInfo>,
    frontend_instructions: Option<String>,
    mut hints: Option<String>,
) -> (Vec<ExtensionInfo>, Option<String>) {
    if let Some(frontend_instructions) = frontend_instructions {
        extensions_info.push(ExtensionInfo::new(
            "frontend",
            &frontend_instructions,
            false,
        ));
    }
    extensions_info.sort_by(|a, b| a.name.cmp(&b.name));

    let mut extensions_info: Vec<ExtensionInfo> = extensions_info
        .into_iter()
        .map(|mut ext_info| {
            ext_info.instructions = sanitize_unicode_tags(&ext_info.instructions);
            ext_info
        })
        .collect();
    let report =
        apply_injection_budget(&mut extensions_info, &mut hints, injection_budget_tokens());
    if !report.is_empty() {
        tracing::warn!(
            dropped = ?report.dropped,
            truncated = ?report.truncated,
            "context budget: trimmed injected system-prompt blocks to fit CONTEXT_INJECTION_BUDGET_TOKENS"
        );
    }
    (extensions_info, hints)
}

fn capability_has_tool(capabilities: &[ExtensionInfo], capability: &str, tool: &str) -> bool {
    capabilities.iter().any(|info| {
        prompt_name_key(&info.name) == capability
            && (!info.tool_roster_known || info.available_tools.iter().any(|name| name == tool))
    })
}

fn capability_has_direct_tool(
    capabilities: &[ExtensionInfo],
    capability: &str,
    tool: &str,
) -> bool {
    capabilities.iter().any(|info| {
        prompt_name_key(&info.name) == capability
            && (!info.tool_roster_known
                || info.directly_callable_tools.iter().any(|name| name == tool))
    })
}

fn build_system_prompt_context(
    manager: &PromptManager,
    extensions_info: Vec<ExtensionInfo>,
    biorouter_mode: BioRouterMode,
    subagents_enabled: bool,
    code_execution_mode: bool,
    live_clock: String,
) -> SystemPromptContext {
    let (capabilities, extensions): (Vec<_>, Vec<_>) = extensions_info
        .into_iter()
        .partition(|info| info.classification == ExtensionClassification::Capability);
    let installed_extension_discovery_available = capability_has_tool(
        &capabilities,
        "extensionmanager",
        "search_available_extensions",
    );
    // ⚠ Browsing is `search_marketplace_extensions` with no query — the two
    // tools merged. A separate `browse` boolean would now be permanently false
    // and would silently delete the clause it gates.
    let marketplace_extension_search_available = capability_has_tool(
        &capabilities,
        "extensionmanager",
        "search_marketplace_extensions",
    );
    let extension_state_change_available =
        capability_has_tool(&capabilities, "extensionmanager", "manage_extensions");
    let extension_package_install_available =
        capability_has_tool(&capabilities, "extensionmanager", "install_extension");
    let extension_package_delete_available = capability_has_tool(
        &capabilities,
        "extensionmanager",
        "delete_extension_package",
    );
    // ⚠ Its own clause, gated on its own tool, exactly like the four above: the
    // two uninstall tools accept different identifiers and one can be offered
    // without the other, so a shared clause would tell a model it can remove a
    // sideloaded extension when only the marketplace door is on the roster.
    let extension_removal_available =
        capability_has_tool(&capabilities, "extensionmanager", "remove_extension");
    let extension_resource_tools_available =
        capability_has_tool(&capabilities, "extensionmanager", "list_resources")
            && capability_has_tool(&capabilities, "extensionmanager", "read_resource");
    let extension_resource_tools_directly_callable =
        capability_has_direct_tool(&capabilities, "extensionmanager", "list_resources")
            && capability_has_direct_tool(&capabilities, "extensionmanager", "read_resource");
    let skill_load_available = capability_has_tool(&capabilities, "skills", "loadSkill");
    let knowledge_search_available = capability_has_tool(&capabilities, "knowledge", "kb_search");

    let developer_shell_available = capability_has_tool(&capabilities, "developer", "shell");
    let developer_text_editor_available =
        capability_has_tool(&capabilities, "developer", "text_editor");
    let developer_shell_directly_callable =
        capability_has_direct_tool(&capabilities, "developer", "shell");
    let developer_text_editor_directly_callable =
        capability_has_direct_tool(&capabilities, "developer", "text_editor");
    let code_execute_directly_callable = code_execution_mode
        && capability_has_direct_tool(&capabilities, "codeexecution", "execute_code");

    SystemPromptContext {
        developer_shell_available,
        developer_text_editor_available,
        developer_shell_directly_callable,
        developer_text_editor_directly_callable,
        developer_shell_via_code_execution: code_execute_directly_callable
            && developer_shell_available
            && !developer_shell_directly_callable,
        developer_text_editor_via_code_execution: code_execute_directly_callable
            && developer_text_editor_available
            && !developer_text_editor_directly_callable,
        code_execute_available: capability_has_tool(&capabilities, "codeexecution", "execute_code"),
        capabilities,
        extensions,
        installed_extension_discovery_available,
        marketplace_extension_search_available,
        extension_state_change_available,
        extension_package_install_available,
        extension_package_delete_available,
        extension_removal_available,
        extension_resource_tools_available,
        extension_resource_tools_directly_callable,
        skill_load_available,
        knowledge_search_available,
        current_date_time: manager.fixed_timestamp.clone().unwrap_or(live_clock),
        biorouter_mode,
        is_autonomous: biorouter_mode == BioRouterMode::Auto,
        enable_subagents: subagents_enabled,
        code_execution_mode,
        checklist_enforcement: false,
    }
}

fn render_base_prompt(
    manager: &PromptManager,
    variant: PromptVariant,
    context: &SystemPromptContext,
) -> String {
    let mut base_prompt = if let Some(override_prompt) = &manager.system_prompt_override {
        let sanitized_override_prompt = sanitize_unicode_tags(override_prompt);
        prompt_template::render_inline_once(&sanitized_override_prompt, context)
    } else {
        prompt_template::render_global_file("system.md", context)
    }
    .unwrap_or_else(|_| {
        "You are Biorouter, a general-purpose AI agent and integrated research environment for biomedical discovery, created by Wanjun Gu and the Baranzini Lab at UCSF".to_string()
    });

    if variant == PromptVariant::SmallLocal && manager.system_prompt_override.is_none() {
        match prompt_template::render_global_file("system_small_local.md", context) {
            Ok(overlay) if !overlay.trim().is_empty() => {
                base_prompt.push_str("\n\n");
                base_prompt.push_str(&overlay);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("failed to render small-local prompt overlay: {e}")
            }
        }
    }
    base_prompt
}

fn append_system_prompt_extras(
    manager: &PromptManager,
    base_prompt: String,
    hints: Option<String>,
    biorouter_mode: BioRouterMode,
) -> String {
    let mut extras = manager.system_prompt_extras.clone();
    extras.extend(manager.named_system_prompt_extras.values().cloned());
    if let Some(hints) = hints {
        extras.push(hints);
    }
    if biorouter_mode == BioRouterMode::Chat {
        extras.push(
            "Right now you are in the chat only mode, no access to any tool use and system."
                .to_string(),
        );
    }
    let extras: Vec<String> = extras
        .into_iter()
        .map(|extra| sanitize_unicode_tags(&extra))
        .collect();

    if extras.is_empty() {
        base_prompt
    } else {
        format!(
            "{}\n\n# Additional Instructions:\n\n{}",
            base_prompt,
            extras.join("\n\n")
        )
    }
}

fn prompt_name_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl PromptManager {
    pub fn new() -> Self {
        PromptManager {
            system_prompt_override: None,
            system_prompt_extras: Vec::new(),
            named_system_prompt_extras: BTreeMap::new(),
            // Left unset: the clock is computed live per `build()` (hour or,
            // for a local engine, day granularity) so it stays cache-stable
            // yet never freezes at agent-construction time.
            fixed_timestamp: None,
        }
    }

    #[cfg(test)]
    pub fn with_timestamp(dt: DateTime<Utc>) -> Self {
        PromptManager {
            system_prompt_override: None,
            system_prompt_extras: Vec::new(),
            named_system_prompt_extras: BTreeMap::new(),
            fixed_timestamp: Some(dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        }
    }

    /// Add an additional instruction to the system prompt
    pub fn add_system_prompt_extra(&mut self, instruction: String) {
        self.system_prompt_extras.push(instruction);
    }

    /// Replace one live session-scoped prompt block without accumulating stale
    /// copies when the session is refreshed. An empty value removes the block.
    pub fn set_named_system_prompt_extra(&mut self, name: &str, instruction: Option<String>) {
        match instruction.filter(|value| !value.trim().is_empty()) {
            Some(instruction) => {
                self.named_system_prompt_extras
                    .insert(name.to_string(), instruction);
            }
            None => {
                self.named_system_prompt_extras.remove(name);
            }
        }
    }

    /// Override the system prompt with custom text
    pub fn set_system_prompt_override(&mut self, template: String) {
        self.system_prompt_override = Some(template);
    }

    pub fn builder<'a>(&'a self) -> SystemPromptBuilder<'a, Self> {
        SystemPromptBuilder {
            manager: self,

            extensions_info: vec![],
            frontend_instructions: None,
            subagents_enabled: false,
            hints: None,
            code_execution_mode: false,
            checklist_enforcement: false,
            variant: PromptVariant::Default,
            local_engine: false,
        }
    }

    pub async fn get_workflow_prompt(&self) -> String {
        let context: HashMap<&str, Value> = HashMap::new();
        prompt_template::render_global_file("workflow.md", &context)
            .unwrap_or_else(|_| "The workflow prompt is busted. Tell the user.".to_string())
    }
}

/// Apply the BR-2 injection budget across the injected system-prompt blocks —
/// the attached MCP instructions and the hint files — mutating each in place
/// (truncating or emptying). Tool instructions rank above hints: teaching the
/// model how to call a tool matters more than project hints, so hints are
/// trimmed first and instructions only if the servers alone exceed the budget.
/// `budget_tokens == 0` disables the cap. Returns what was trimmed for logging.
fn apply_injection_budget(
    extensions: &mut [ExtensionInfo],
    hints: &mut Option<String>,
    budget_tokens: usize,
) -> BudgetReport {
    if budget_tokens == 0 {
        return BudgetReport::default();
    }

    let has_hints = hints.is_some();
    let mut blocks: Vec<ContextBlock> = extensions
        .iter()
        .map(|ext| ContextBlock {
            label: format!(
                "{}:{}",
                match ext.classification {
                    ExtensionClassification::Capability => "capability",
                    ExtensionClassification::Extension => "extension",
                },
                ext.name
            ),
            content: ext.instructions.clone(),
            priority: 100,
        })
        .collect();
    if let Some(h) = hints.as_ref() {
        blocks.push(ContextBlock {
            label: "hints".to_string(),
            content: h.clone(),
            priority: 50,
        });
    }

    let (fitted, report) = fit_context_blocks(blocks, budget_tokens);
    if report.is_empty() {
        // Fast path: nothing changed, avoid rewriting every field.
        return report;
    }

    // `fitted` preserves input order: the first `extensions.len()` entries are
    // the extension blocks, followed by the optional hints block.
    for (ext, fitted_block) in extensions.iter_mut().zip(fitted.iter()) {
        ext.instructions_degraded = ext.instructions != fitted_block.content;
        ext.instructions = fitted_block.content.clone();
    }
    if has_hints {
        let fitted_hints = &fitted[extensions.len()].content;
        *hints = if fitted_hints.is_empty() {
            None
        } else {
            Some(fitted_hints.clone())
        };
    }

    report
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    /// The appended small-local overlay, heading included.
    fn small_local_overlay(prompt: &str) -> &str {
        let start = prompt
            .find("# Running on a Smaller Model")
            .expect("the small-local overlay is appended");
        prompt
            .get(start..)
            .expect("a match starts on a char boundary")
    }

    fn capability_with_tools(name: &str, tools: &[&str]) -> ExtensionInfo {
        let mut info = ExtensionInfo::capability(name, "focused capability guidance", false);
        info.tool_roster_known = true;
        info.available_tools = tools.iter().map(|tool| (*tool).to_string()).collect();
        info.directly_callable_tools
            .clone_from(&info.available_tools);
        info
    }

    #[test]
    fn test_build_system_prompt_sanitizes_override() {
        let mut manager = PromptManager::new();
        let malicious_override = "System prompt\u{E0041}\u{E0042}\u{E0043}with hidden text";
        manager.set_system_prompt_override(malicious_override.to_string());

        let result = manager.builder().build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("System prompt"));
        assert!(result.contains("with hidden text"));
    }

    #[test]
    fn test_build_system_prompt_sanitizes_extras() {
        let mut manager = PromptManager::new();
        let malicious_extra = "Extra instruction\u{E0041}\u{E0042}\u{E0043}hidden";
        manager.add_system_prompt_extra(malicious_extra.to_string());

        let result = manager.builder().build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("Extra instruction"));
        assert!(result.contains("hidden"));
    }

    #[test]
    fn test_build_system_prompt_sanitizes_multiple_extras() {
        let mut manager = PromptManager::new();
        manager.add_system_prompt_extra("First\u{E0041}instruction".to_string());
        manager.add_system_prompt_extra("Second\u{E0042}instruction".to_string());
        manager.add_system_prompt_extra("Third\u{E0043}instruction".to_string());

        let result = manager.builder().build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("Firstinstruction"));
        assert!(result.contains("Secondinstruction"));
        assert!(result.contains("Thirdinstruction"));
    }

    /// The two mechanisms, side by side, on the same block of text.
    ///
    /// Every caller that re-runs on a reconnect or a re-apply — the app socket's
    /// `configure_agent`, the workflow apply behind
    /// `PUT /sessions/{id}/user_workflow_values` — has a choice between these
    /// two, and the difference is invisible until the prompt is measured:
    /// `add_system_prompt_extra` is `Vec::push` with no dedup, so N applications
    /// leave N full copies in the built prompt. The blocks involved are not
    /// small (a workflow block inlines each declared skill's entire `SKILL.md`;
    /// an app's prompt carries its whole `ui_*` guidance), so the accumulation
    /// eats the context window silently.
    #[test]
    fn a_reapplied_block_appears_once_in_the_named_slot_and_twice_when_appended() {
        const BLOCK: &str = "WORKFLOW BLOCK SENTINEL";

        let mut named = PromptManager::new();
        for _ in 0..3 {
            named.set_named_system_prompt_extra("session_context", Some(BLOCK.into()));
        }
        assert_eq!(
            named.builder().build().matches(BLOCK).count(),
            1,
            "the named slot is what makes re-applying idempotent"
        );

        let mut appended = PromptManager::new();
        for _ in 0..3 {
            appended.add_system_prompt_extra(BLOCK.to_string());
        }
        assert_eq!(
            appended.builder().build().matches(BLOCK).count(),
            3,
            "the appending path is not deduped — this is the behaviour a call site must not pick \
             for anything it runs more than once per agent"
        );
    }

    #[test]
    fn named_system_prompt_extras_replace_and_remove_stale_context() {
        let mut manager = PromptManager::new();
        manager.set_named_system_prompt_extra("workflow", Some("OLD WORKFLOW".into()));
        manager.set_named_system_prompt_extra("workflow", Some("CURRENT WORKFLOW".into()));

        let current = manager.builder().build();
        assert!(!current.contains("OLD WORKFLOW"));
        assert_eq!(current.matches("CURRENT WORKFLOW").count(), 1);

        manager.set_named_system_prompt_extra("workflow", None);
        let removed = manager.builder().build();
        assert!(!removed.contains("CURRENT WORKFLOW"));
    }

    #[test]
    fn test_build_system_prompt_preserves_legitimate_unicode_in_extras() {
        let mut manager = PromptManager::new();
        let legitimate_unicode = "Instruction with 世界 and 🌍 emojis";
        manager.add_system_prompt_extra(legitimate_unicode.to_string());

        let result = manager.builder().build();

        assert!(result.contains("世界"));
        assert!(result.contains("🌍"));
        assert!(result.contains("Instruction with"));
        assert!(result.contains("emojis"));
    }

    #[test]
    fn test_build_system_prompt_sanitizes_extension_instructions() {
        let manager = PromptManager::new();
        let malicious_extension_info = ExtensionInfo::new(
            "test_extension",
            "Extension help\u{E0041}\u{E0042}\u{E0043}hidden instructions",
            false,
        );

        let result = manager
            .builder()
            .with_extension(malicious_extension_info)
            .build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("Extension help"));
        assert!(result.contains("hidden instructions"));
    }

    #[test]
    fn test_basic() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let system_prompt = manager.builder().build();

        assert_snapshot!(system_prompt)
    }

    /// The live (non-test) clock is computed at `build()` time, at Local hour
    /// granularity — not frozen at construction, not UTC. Robust across an hour
    /// tick by accepting either boundary.
    #[test]
    fn test_live_clock_is_fresh_local_hour() {
        let before = chrono::Local::now().format("%Y-%m-%d %H:00").to_string();
        let prompt = PromptManager::new().builder().build();
        let after = chrono::Local::now().format("%Y-%m-%d %H:00").to_string();

        let expected_before = format!("The current date and time is {before}.");
        let expected_after = format!("The current date and time is {after}.");
        assert!(
            prompt.contains(&expected_before) || prompt.contains(&expected_after),
            "system prompt must render the live Local hour clock (minutes pinned to :00)"
        );
    }

    /// A local engine's prompt must not change at an hour boundary: the clock
    /// sits near the top of the prompt, so a change there costs the engine its
    /// whole prompt cache (measured: 107 s of prefill on qwen3.8:27b). The
    /// control half shows the same two builds DO differ for a hosted model, so
    /// the first half cannot pass merely because no clock was rendered.
    #[test]
    fn a_local_engine_prompt_is_byte_identical_across_an_hour_boundary() {
        use chrono::TimeZone;
        let before = chrono::Local
            .with_ymd_and_hms(2026, 7, 12, 10, 59, 0)
            .single()
            .expect("an unambiguous local time");
        let after = chrono::Local
            .with_ymd_and_hms(2026, 7, 12, 11, 1, 0)
            .single()
            .expect("an unambiguous local time");
        let manager = PromptManager::new();
        let build = |local_engine: bool, now| {
            manager
                .builder()
                .with_local_engine(local_engine)
                .build_at(now)
        };

        let local_before = build(true, before);
        assert_eq!(local_before, build(true, after));
        assert!(
            local_before.contains("The current date and time is 2026-07-12."),
            "a local engine still gets the date"
        );

        let hosted_before = build(false, before);
        assert!(hosted_before.contains("The current date and time is 2026-07-12 10:00."));
        assert_ne!(
            hosted_before,
            build(false, after),
            "a hosted model keeps the hour clock"
        );
    }

    /// The engines whose prompt cache the day clock protects: the two local
    /// providers by name (wherever they run) and anything running here.
    #[test]
    fn local_engines_are_the_local_providers_and_anything_on_this_machine() {
        assert!(is_local_engine("ollama", false));
        assert!(is_local_engine("LlamaCpp", false));
        assert!(is_local_engine("my_lab_ollama", true));
        assert!(!is_local_engine("my_lab_ollama", false));
        assert!(!is_local_engine("anthropic", false));
    }

    #[test]
    fn test_one_extension() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let system_prompt = manager
            .builder()
            .with_extension(ExtensionInfo::new(
                "test",
                "how to use this extension",
                true,
            ))
            .build();

        assert_snapshot!(system_prompt)
    }

    #[test]
    fn test_typical_setup() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let system_prompt = manager
            .builder()
            .with_extension(ExtensionInfo::capability(
                "developer",
                "<instructions on how to use the Developer capability>",
                true,
            ))
            .with_extension(ExtensionInfo::new(
                "extension_A",
                "<instructions on how to use extension A (no resources)>",
                false,
            ))
            .build();

        assert_snapshot!(system_prompt)
    }

    #[test]
    fn test_capabilities_and_extensions_render_in_separate_authoritative_sections() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let prompt = manager
            .builder()
            .with_extension(ExtensionInfo::capability(
                "developer",
                "developer instructions",
                false,
            ))
            .with_extension(ExtensionInfo::new(
                "custom_connector",
                "connector instructions",
                false,
            ))
            .build();

        let capabilities = prompt.find("# Enabled Capabilities").unwrap();
        let developer = prompt.find("## developer").unwrap();
        let extensions = prompt.find("# Loaded Extensions").unwrap();
        let custom = prompt.find("## custom_connector").unwrap();
        assert!(capabilities < developer && developer < extensions);
        assert!(extensions < custom);
        assert!(prompt.contains("They are not extensions."));
    }

    #[test]
    fn prompt_tracks_disabled_and_tool_restricted_capabilities_exactly() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let mut restricted = ExtensionInfo::capability(
            "developer",
            "stale instructions that must not be followed",
            false,
        );
        restricted.tool_roster_known = true;

        let prompt = manager.builder().with_extension(restricted).build();

        assert!(prompt.contains("## developer"), "{prompt}");
        assert!(
            prompt.contains("loaded but has no effective tools for this turn"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("stale instructions that must not be followed"),
            "restricted capability guidance must not survive an empty effective roster: {prompt}"
        );
        assert!(
            !prompt.contains("## knowledge") && !prompt.contains("## skills"),
            "disabled capabilities must be absent rather than described as available: {prompt}"
        );
        assert!(
            !prompt.contains("built-in **Soul**") && !prompt.contains("about-biorouter"),
            "conditional guidance must disappear with its capability: {prompt}"
        );
    }

    #[test]
    fn focused_guidance_requires_the_exact_effective_operation() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let skills_without_load = manager
            .builder()
            .with_extension(capability_with_tools("skills", &["listSkills"]))
            .build();
        assert!(!skills_without_load.contains("about-biorouter"));

        let skills_with_load = manager
            .builder()
            .with_extension(capability_with_tools("skills", &["loadSkill"]))
            .build();
        assert!(skills_with_load.contains("about-biorouter"));

        let knowledge_without_search = manager
            .builder()
            .with_extension(capability_with_tools("knowledge", &["kb_write_page"]))
            .build();
        assert!(!knowledge_without_search.contains("built-in **Soul**"));

        let knowledge_with_search = manager
            .builder()
            .with_extension(capability_with_tools("knowledge", &["kb_search"]))
            .build();
        assert!(knowledge_with_search.contains("built-in **Soul**"));
    }

    #[test]
    fn extension_manager_claims_follow_its_effective_operation_groups() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let build = |tools: &[&str]| {
            manager
                .builder()
                .with_extension(capability_with_tools("Extension Manager", tools))
                .build()
        };

        // One clause per operation, each gated on its own tool. A single
        // combined sentence would have to be true whenever ANY of them is
        // callable, which is the thing this block exists to stop: a model told
        // it "can install" because `search_available_extensions` was present.
        let discovery = build(&["search_available_extensions"]);
        assert!(discovery.contains("- discover installed extensions and their exact names"));
        assert!(!discovery.contains("- enable or disable"));
        assert!(!discovery.contains("- install an extension package"));
        assert!(!discovery.contains("- permanently delete"));

        let state_change = build(&["manage_extensions"]);
        assert!(state_change.contains("- enable or disable a named installed extension"));
        assert!(!state_change.contains("- discover installed extensions"));
        assert!(!state_change.contains("- install an extension package"));

        let install = build(&["install_extension"]);
        assert!(install.contains("- install an extension package"));
        assert!(!install.contains("- permanently delete"));

        let delete = build(&["delete_extension_package"]);
        assert!(delete.contains("- permanently delete an installed marketplace extension package"));
        assert!(!delete.contains("- install an extension package"));
        assert!(!delete.contains("- permanently remove any installed extension"));

        // #164. The two uninstall doors are separately gated, so being offered
        // the marketplace one must not read as being offered the other: the
        // identifiers differ, and a model that conflates them names a registry
        // id no catalog resolves and falls back to editing config.yaml.
        let remove = build(&["remove_extension"]);
        assert!(remove.contains("- permanently remove any installed extension"));
        assert!(remove.contains("did not come from the marketplace"));
        assert!(!remove.contains("- permanently delete an installed marketplace extension"));

        // Browse and search are ONE tool now — browsing is the same call with no
        // query — so there is one clause, gated on the one surviving name.
        let marketplace = build(&["search_marketplace_extensions"]);
        assert!(marketplace.contains("- browse or search the trusted marketplace catalog"));
        assert!(!discovery.contains("browse or search the trusted marketplace"));

        // ⚠ Every clause is a bullet under one subject line, so no rendering
        // can open the paragraph with a subject-less "It" — which is what
        // happened when the marketplace sentence led with a pronoun whose
        // antecedent lived in a clause that had not rendered.
        for rendered in [
            &discovery,
            &state_change,
            &install,
            &delete,
            &remove,
            &marketplace,
        ] {
            assert!(rendered.contains(
                "The Extension Manager capability can do only what this turn's effective roster allows:"
            ));
            assert!(!rendered.contains("\nIt can "));
        }

        let resources_only = build(&["list_resources", "read_resource"]);
        assert!(resources_only.contains("Extension Manager operations are not available"));
    }

    /// The checklist clause describes the planning gate, so it renders exactly
    /// when the gate runs (`planning_gate::enforcement_applies`) and never
    /// otherwise — a prompt promising a refusal the turn does not make is the
    /// defect this flag exists to prevent.
    #[test]
    fn the_checklist_clause_renders_only_while_the_planning_gate_runs() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let off = manager.builder().build();
        assert!(!off.contains("Biorouter enforces the checklist"), "{off}");

        let on = manager.builder().with_checklist_enforcement(true).build();
        assert!(on.contains("Biorouter enforces the checklist"), "{on}");
        assert!(on.contains("your first action is `todo__todo_write`"));
        assert!(on.contains("That refusal happens once per turn"));
        assert!(on.contains("names each unfinished item by its `#N` id"));
        // It extends the planning bullet rather than replacing it.
        assert!(on.contains("plan before acting"));
    }

    /// Contract test for the agentic-behavior clauses added to `system.md`.
    /// Each assertion guards one intentional instruction against silent
    /// removal/regression. These are the table-stakes behaviors the prompt
    /// review found missing; if any of these strings disappears, the agent
    /// quietly loses the behavior.
    #[test]
    fn test_system_prompt_has_behavior_clauses() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let p = manager.builder().build();

        // Date is actually rendered (was computed-but-unused before).
        assert!(
            p.contains("The current date and time is 1970-01-01 00:00:00"),
            "current_date_time must render so the agent can judge what is 'recent'"
        );
        // Conciseness budget.
        assert!(p.contains("Be concise."), "missing conciseness clause");
        assert!(
            p.to_lowercase().contains("preamble"),
            "missing preamble/postamble ban"
        );
        // Tool-use discipline.
        assert!(
            p.contains("run in parallel"),
            "missing parallel-tool-call guidance"
        );
        assert!(
            p.contains("don't expose internal tool names"),
            "missing never-name-tools rule"
        );
        // Working-on-tasks discipline.
        assert!(
            p.contains("Before editing a file, read the relevant parts"),
            "missing read-before-edit rule"
        );
        assert!(
            p.contains("not surprising the user"),
            "missing proactiveness/don't-surprise balance"
        );
        // Planning/todo discipline (moved here from the todo extension MOIM).
        assert!(
            p.contains("plan before acting"),
            "missing planning/todo discipline"
        );
        // Verification-before-completion discipline.
        assert!(
            p.contains("Before reporting a task complete, verify it"),
            "missing verify-before-done discipline"
        );
        // Safety posture (including the biomedical-accuracy clause).
        assert!(
            p.contains("Never expose, log, or commit secrets"),
            "missing secrets rule"
        );
        assert!(
            p.contains("biomedical and scientific claims"),
            "missing biomedical-accuracy/anti-fabrication clause"
        );
        // Output conventions.
        assert!(
            p.contains("file_path:line_number"),
            "missing code-reference citation convention"
        );
        assert!(
            p.contains("use its verified absolute path on every turn, including"),
            "missing persistent absolute-path rule for file references (#44)"
        );
        assert!(
            p.contains("the label may be short, but do not replace")
                && p.contains("the target with just a filename or guess a missing directory"),
            "missing distinction between file link labels and verified targets"
        );
        assert!(
            p.contains("[source.rs](/absolute/path/source.rs:42)"),
            "missing absolute source-line link convention"
        );
        // Tool-routing discipline. Generic guidance renders in every mode;
        // capability-specific guidance must render only while that capability
        // is effective.
        assert!(p.contains("# Tool Routing"), "missing Tool Routing section");
        assert!(
            p.contains("Prefer the simplest tool that does the job"),
            "missing prefer-the-simplest-tool rule"
        );

        let with_code_execution = manager
            .builder()
            .with_extension(ExtensionInfo::capability(
                "code_execution",
                "code execution instructions",
                false,
            ))
            .build();
        assert!(
            with_code_execution.contains("Use Code Execution when the task needs computation"),
            "missing code-execution-only-for-computation rule"
        );
        assert!(
            !p.contains("Use Code Execution when the task needs computation"),
            "disabled capabilities must not leave stale routing instructions"
        );
    }

    /// The parent half of the ambiguous-delegation fix.
    ///
    /// The child was taught to stop and ask (`prompts/subagent_system.md`) and
    /// did, three runs out of three. The parent was taught nothing:
    /// `system.md` had no match for "ambiguity", "clarify" or "ask" in this
    /// sense, so it read the returned question as a finished delegation with
    /// no edit behind it and made the ambiguous edits itself, which is worse
    /// than before the child was taught anything: a full round trip AND both
    /// files rewritten.
    ///
    /// Flattened, so re-flowing the paragraph cannot break the test.
    #[test]
    fn test_system_prompt_tells_the_parent_what_to_do_with_an_unresolvable_reference() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let p = manager
            .builder()
            .build()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        assert!(
            p.contains("# ambiguity"),
            "the parent needs a section it can find: {p}"
        );
        // Scoped to the referent case. Completely Autonomous mode not asking
        // permission is the mode working; re-adding confirmation prompts for
        // ordinary work would be fixing the wrong thing.
        assert!(
            p.contains(
                "autonomy means not asking permission for work you understand. it does not \
                        mean guessing what the work is"
            ),
            "the rule must separate 'may i act' from 'what am i acting on': {p}"
        );
        assert!(
            p.contains("ask the user which one and wait"),
            "an unresolvable referent is resolved by asking, and the parent is who asks: {p}"
        );
        // The three wrong resolutions, each named. "Act on every candidate" is
        // the measured one: both files were rewritten.
        assert!(
            p.contains("don't pick the most likely candidate"),
            "missing the don't-guess rule: {p}"
        );
        assert!(
            p.contains("don't act on every candidate to cover both"),
            "rewriting BOTH candidates is the exact measured failure: {p}"
        );
        assert!(
            !p.contains("before delegating") && !p.contains("comes back with status `blocked`"),
            "delegation guidance must be absent when no delegation tool is available: {p}"
        );

        let delegating = manager
            .builder()
            .with_enable_subagents(true)
            .build()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            delegating.contains("# ambiguity and delegation"),
            "an available delegation surface needs the full section: {delegating}"
        );
        // What a blocked subagent means and what to do with it.
        assert!(
            delegating.contains("comes back with status `blocked`"),
            "the parent must be able to recognise the status: {delegating}"
        );
        assert!(
            delegating.contains("that is the delegation working, not failing"),
            "unsaid, a model treats a no-edit run as a failed one and redoes the work: {delegating}"
        );
        assert!(
            delegating.contains("delegate the task again with the answer written out in full"),
            "the cheap path, when the parent CAN settle it, must be named: {delegating}"
        );
        assert!(
            delegating.contains("put the subagent's question to the user in your reply and wait"),
            "the question must reach the user when neither party can settle it: {delegating}"
        );
        assert!(
            delegating.contains(
                "never settle it by guessing, by delegating again with a guess, or by \
                        doing the work yourself instead"
            ),
            "doing the work itself is what the parent actually did: {delegating}"
        );
    }

    /// Pillar guidance must follow the specific capabilities that make it
    /// actionable, not the mere presence of any attached client.
    #[test]
    fn test_pillar_awareness_requires_its_capabilities() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let developer_only = manager
            .builder()
            .with_extension(capability_with_tools("developer", &["shell"]))
            .build();
        assert!(
            !developer_only.contains("about-biorouter") && !developer_only.contains("Soul"),
            "Developer alone must not advertise unavailable Skills or Knowledge tools"
        );

        let skills_only = manager
            .builder()
            .with_extension(capability_with_tools("skills", &["loadSkill"]))
            .build();
        assert!(
            skills_only.contains("about-biorouter") && !skills_only.contains("Soul"),
            "Skills enables product guidance, but not Knowledge/Soul guidance"
        );

        let knowledge_only = manager
            .builder()
            .with_extension(capability_with_tools("knowledge", &["kb_search"]))
            .build();
        assert!(
            !knowledge_only.contains("about-biorouter") && knowledge_only.contains("Soul"),
            "Knowledge enables Soul guidance independently of Skills"
        );
    }

    #[test]
    fn test_code_execution_mode_keeps_authoritative_tool_state() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let prompt = manager
            .builder()
            .with_extension(ExtensionInfo::capability(
                "code_execution",
                "code execution instructions",
                false,
            ))
            .with_extension(ExtensionInfo::new(
                "custom_connector",
                "custom connector instructions",
                false,
            ))
            .with_code_execution_mode(true)
            .build();

        assert!(prompt.contains("# Enabled Capabilities"), "{prompt}");
        assert!(prompt.contains("## code_execution"), "{prompt}");
        assert!(prompt.contains("code execution instructions"), "{prompt}");
        assert!(prompt.contains("# Loaded Extensions"), "{prompt}");
        assert!(prompt.contains("## custom_connector"), "{prompt}");
        assert!(prompt.contains("custom connector instructions"), "{prompt}");
    }

    /// BR-2: with a generous budget, injected blocks are left byte-identical and
    /// the report is empty — ordinary sessions are unaffected.
    #[test]
    fn test_injection_budget_noop_under_budget() {
        let mut extensions = vec![
            ExtensionInfo::new("a", "short instructions", false),
            ExtensionInfo::new("b", "more instructions", false),
        ];
        let mut hints = Some("some project hints".to_string());
        let report = apply_injection_budget(&mut extensions, &mut hints, 10_000);
        assert!(report.is_empty());
        assert_eq!(extensions[0].instructions, "short instructions");
        assert_eq!(extensions[1].instructions, "more instructions");
        assert!(!extensions[0].instructions_degraded);
        assert!(!extensions[1].instructions_degraded);
        assert_eq!(hints.as_deref(), Some("some project hints"));
    }

    #[test]
    fn context_budget_degradation_is_visible_in_the_model_prompt() {
        let mut extensions = vec![ExtensionInfo::capability(
            "large_capability",
            &"operating guidance ".repeat(2_000),
            false,
        )];
        let mut hints = None;

        let report = apply_injection_budget(&mut extensions, &mut hints, 100);
        assert!(!report.is_empty());
        assert!(extensions[0].instructions_degraded);

        let prompt = PromptManager::new()
            .builder()
            .with_extension(extensions.remove(0))
            .build();
        assert!(prompt.contains("Context-budget notice"), "{prompt}");
        assert!(
            prompt.contains("do not invent missing behavior"),
            "{prompt}"
        );
    }

    /// BR-2: hints (lower priority) are dropped before extension instructions
    /// (higher priority) when the budget is tight.
    #[test]
    fn test_injection_budget_drops_hints_before_instructions() {
        let big_instructions = "i".repeat(3_000); // ~750 tokens
        let big_hints = "h".repeat(3_000);
        let mut extensions = vec![ExtensionInfo::new("dev", &big_instructions, false)];
        let mut hints = Some(big_hints.clone());

        let report = apply_injection_budget(&mut extensions, &mut hints, 800);

        assert_eq!(
            extensions[0].instructions, big_instructions,
            "extension instructions must be kept in full"
        );
        assert_eq!(hints, None, "hints must be dropped to fit the budget");
        assert!(report.dropped.iter().any(|l| l == "hints"));
    }

    /// BR-2: a budget of 0 disables the cap entirely.
    #[test]
    fn test_injection_budget_disabled_with_zero() {
        let big = "x".repeat(100_000);
        let mut extensions = vec![ExtensionInfo::new("dev", &big, false)];
        let mut hints = Some(big.clone());
        let report = apply_injection_budget(&mut extensions, &mut hints, 0);
        assert!(report.is_empty());
        assert_eq!(extensions[0].instructions, big);
        assert_eq!(hints.as_deref(), Some(big.as_str()));
    }

    /// BR-3: the provider/model table routes the local providers to the
    /// small-local variant and everything else to the strong-model default.
    #[test]
    fn test_prompt_variant_table() {
        // Local providers with a small model → small-local scaffolding.
        for (provider, model) in [
            ("llamacpp", "qwen3.5-4b"),
            ("llamacpp", "gemma4"),
            ("ollama", "gemma4:latest"),
            ("ollama", "qwen3:8b"),
            ("ollama", "gemma4:e4b"),
            // An MoE's ACTIVE size is not its size, and a 3B model is small.
            ("ollama", "llama3.2:3b"),
        ] {
            assert_eq!(
                select_variant_from_table(provider, model, false),
                PromptVariant::SmallLocal,
                "{provider}/{model}"
            );
        }
        // Strong commercial / institution-hosted → base prompt only.
        for (provider, model) in [
            ("anthropic", "claude-sonnet-4"),
            ("openai", "gpt-5.2"),
            ("versa_azure", "gpt-5.6-sol"),
        ] {
            assert_eq!(
                select_variant_from_table(provider, model, false),
                PromptVariant::Default,
                "{provider}/{model}"
            );
        }
        // A *large* model run locally does not need the extra hand-holding —
        // including both shipped local models, under either engine's spelling.
        for (provider, model) in [
            ("ollama", "llama3.3:70b"),
            ("ollama", "qwen3.8:27b"),
            ("ollama", "qwen3.8:27b-q4_K_M"),
            ("ollama", "gemma4:31b"),
            ("ollama", "gemma4:31b-it-q4_K_M"),
            ("llamacpp", "qwen3.8-27b"),
            ("llamacpp", "gemma4-31b"),
            ("llamacpp", "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M"),
            ("llamacpp", "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0"),
            ("ollama", "qwen3:30b-a3b"),
            ("ollama", "qwen2.5-coder:32b"),
            ("ollama", "mixtral:8x7b"),
            ("ollama", "mistral-large"),
        ] {
            assert_eq!(
                select_variant_from_table(provider, model, false),
                PromptVariant::Default,
                "{provider}/{model}"
            );
        }
    }

    /// OLL-20: a provider whose INSTANCE runs on this machine is prompted like
    /// the built-in local providers whatever it is registered as — a
    /// declarative `ollama_compatible` provider registers under its own name.
    #[test]
    fn a_provider_running_on_this_machine_is_prompted_like_a_local_one() {
        assert_eq!(
            select_variant_from_table("my_lab_ollama", "qwen3:8b", true),
            PromptVariant::SmallLocal
        );
        assert_eq!(
            select_variant_from_table("my_lab_ollama", "qwen3:8b", false),
            PromptVariant::Default,
            "the same name off this machine is not assumed local"
        );
        assert_eq!(
            select_variant_from_table("my_lab_ollama", "qwen3.8:27b", true),
            PromptVariant::Default
        );
    }

    /// Sizes are read only where they stand alone, so Gemma's `e4b`, an MoE's
    /// `a3b`, and a version number are never mistaken for a parameter count.
    #[test]
    fn model_parameter_sizes_are_read_from_standalone_markers_only() {
        for (model, expected) in [
            ("qwen3.8:27b", Some(27.0)),
            ("qwen3.8-27b", Some(27.0)),
            ("ggml-org/qwen3.8-27b-gguf:q4_k_m", Some(27.0)),
            ("gemma4:31b-it-q4_k_m", Some(31.0)),
            ("qwen3.5:0.8b", Some(0.8)),
            ("qwen3:30b-a3b", Some(30.0)),
            ("mixtral:8x7b", Some(56.0)),
            ("gemma4:e4b", None),
            ("gemma4", None),
            ("qwen3.8", None),
            ("gpt-5.6-sol", None),
        ] {
            assert_eq!(model_parameter_billions(model), expected, "{model}");
        }
    }

    /// BR-3: the small-local variant appends the scaffolding overlay, while the
    /// default variant renders the base prompt byte-identically (no overlay).
    #[test]
    fn test_small_local_variant_appends_overlay() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let default_prompt = manager
            .builder()
            .with_prompt_variant(PromptVariant::Default)
            .build();
        assert!(
            !default_prompt.contains("Running on a Smaller Model"),
            "the default (strong-model) variant must not append the overlay"
        );

        let small_prompt = manager
            .builder()
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        assert!(
            small_prompt.starts_with(&default_prompt),
            "the small-local variant is the shared base plus an appended overlay"
        );
        assert!(
            small_prompt.len() > default_prompt.len(),
            "the small-local variant must add scaffolding on top of the base"
        );
    }

    /// BR-3 contract test: guard the small-local overlay's intentional clauses
    /// against silent removal (mirrors `test_system_prompt_has_behavior_clauses`
    /// for the base prompt). If any of these disappears, the weak-model
    /// scaffolding is quietly lost.
    #[test]
    fn test_small_local_overlay_has_scaffolding_clauses() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let p = manager
            .builder()
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();

        assert!(
            p.contains("You are running on a smaller local model"),
            "missing small-model self-identification"
        );
        assert!(
            p.contains("single tool call per turn"),
            "missing one-step-at-a-time discipline"
        );
        assert!(
            p.contains("emit only valid JSON for tool arguments"),
            "missing valid-tool-JSON discipline"
        );
        assert!(
            p.contains("ask a short clarifying question"),
            "missing ask-vs-act discipline"
        );
        assert!(
            p.contains("Before saying a task is done"),
            "missing verify-before-done discipline"
        );
        assert!(
            p.contains("Prefer the simplest effective tool for the job"),
            "missing small-local tool-routing discipline"
        );
        // L3 / R8 (protocol matrix, Ollama 0.40.2): "after each tool result,
        // briefly note what you learned and the next step" made llama3.1 write
        // a sentence before its JSON call — which then leaked as text instead
        // of being parsed — or narrate the next step and stop mid-task.
        assert!(
            p.contains("act on it directly") && p.contains("Do not narrate a plan"),
            "missing act-without-narrating discipline"
        );
        assert!(
            !p.contains("briefly note what you learned"),
            "the overlay must not ask the model to narrate between tool calls"
        );
        assert!(
            !p.contains("Developer `shell`")
                && !p.contains("Developer `text_editor`")
                && !p.contains("Code Execution capability"),
            "the empty capability roster must not leave named-tool guidance: {p}"
        );

        let shell_only = manager
            .builder()
            .with_extension(capability_with_tools("developer", &["shell"]))
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        assert!(shell_only.contains("When Developer `shell` is available"));
        assert!(!shell_only.contains("When Developer `text_editor` is available"));
        assert!(!shell_only.contains("Prefer `text_editor` for file contents"));
        assert!(!shell_only.contains("Use the Code Execution capability only"));

        let editor_only = manager
            .builder()
            .with_extension(capability_with_tools("developer", &["text_editor"]))
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        assert!(!editor_only.contains("When Developer `shell` is available"));
        assert!(editor_only.contains("When Developer `text_editor` is available"));
        assert!(!editor_only.contains("Prefer `shell` for commands"));

        let code_execution_only = manager
            .builder()
            .with_extension(capability_with_tools("code_execution", &["execute_code"]))
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        assert!(code_execution_only.contains("Use the Code Execution capability only"));
        assert!(!code_execution_only.contains("When Developer `shell` is available"));
    }

    /// Repro r9: with Code Execution mode on, Developer is reachable only from
    /// an `execute_code` script and is NOT on the model's tool list. The
    /// overlay used to say "When Developer `shell` is available, use it" there
    /// anyway, and a small model wrote `{"name": "shell", …}` as text. It must
    /// now name the one route that exists, and never contradict it by telling
    /// the model to avoid Code Execution for simple work.
    #[test]
    fn small_local_overlay_routes_developer_through_code_execution_when_hidden() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let mut developer = capability_with_tools("developer", &["shell", "text_editor"]);
        developer.directly_callable_tools.clear();
        let code_execution = capability_with_tools(
            "code_execution",
            &["execute_code", "read_module", "search_modules"],
        );

        let hidden = manager
            .builder()
            .with_extension(developer.clone())
            .with_extension(code_execution.clone())
            .with_code_execution_mode(true)
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        let overlay = small_local_overlay(&hidden);
        assert!(
            !overlay.contains("When Developer `shell` is available")
                && !overlay.contains("When Developer `text_editor` is available"),
            "{overlay}"
        );
        assert!(
            overlay.contains("Developer `shell` is not a tool you can call by name")
                && overlay.contains(r#"import { shell } from "developer";"#)
                && overlay.contains("Developer `text_editor` is not a tool you can call by name")
                && overlay.contains(r#"import { text_editor } from "developer";"#),
            "{overlay}"
        );
        assert!(
            !overlay.contains("Use the Code Execution capability only when"),
            "must not steer the model away from the only route to Developer: {overlay}"
        );
        assert!(overlay.contains("for file and command work through its `developer` module"));

        // The same roster with Developer directly callable (Code Execution off):
        // the direct advice, and the "only when" clause for Code Execution.
        let direct = manager
            .builder()
            .with_extension(capability_with_tools(
                "developer",
                &["shell", "text_editor"],
            ))
            .with_extension(code_execution)
            .with_code_execution_mode(false)
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        let overlay = small_local_overlay(&direct);
        assert!(overlay.contains("When Developer `shell` is available"));
        assert!(overlay.contains("When Developer `text_editor` is available"));
        assert!(overlay.contains("Use the Code Execution capability only when"));
        assert!(!overlay.contains("is not a tool you can call by name"));

        // Hidden, and no `execute_code` to reach it with: name no route at all.
        let unreachable = manager
            .builder()
            .with_extension(developer)
            .with_extension(capability_with_tools("code_execution", &["read_module"]))
            .with_code_execution_mode(true)
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        let overlay = small_local_overlay(&unreachable);
        assert!(!overlay.contains("Developer `shell`"), "{overlay}");
        assert!(!overlay.contains("Developer `text_editor`"), "{overlay}");
    }

    /// The whole small-local overlay as a small model sees it in the
    /// reporter's setup — Code Execution mode on, Developer reachable only
    /// through `execute_code`. Snapshotted alone, so a base-prompt edit does
    /// not churn it.
    #[test]
    fn small_local_overlay_under_code_execution() {
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let mut developer = capability_with_tools("developer", &["shell", "text_editor"]);
        developer.directly_callable_tools.clear();
        let prompt = manager
            .builder()
            .with_extension(developer)
            .with_extension(capability_with_tools(
                "code_execution",
                &["execute_code", "read_module", "search_modules"],
            ))
            .with_code_execution_mode(true)
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();
        let overlay = small_local_overlay(&prompt);
        assert_snapshot!(overlay)
    }

    /// BR-3: a full custom prompt override is already complete, so the overlay
    /// is not appended even for the small-local variant.
    #[test]
    fn test_small_local_variant_skips_overlay_under_override() {
        let mut manager = PromptManager::new();
        manager.set_system_prompt_override("Custom prompt for a workflow.".to_string());

        let p = manager
            .builder()
            .with_prompt_variant(PromptVariant::SmallLocal)
            .build();

        assert!(p.contains("Custom prompt for a workflow."));
        assert!(
            !p.contains("Running on a Smaller Model"),
            "the overlay must not be appended to a custom prompt override"
        );
    }
}
