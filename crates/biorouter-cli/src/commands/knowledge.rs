//! `biorouter knowledge` subcommands — manage personal knowledge bases from the
//! CLI with the same service the desktop GUI drives over HTTP: list bases, show
//! or set the primary base, create a base, and run the ingest / lint / query
//! macros (each backed by a bounded knowledge sub-agent).
//!
//! The CLI has no session of its own, so every command here is machine-wide by
//! default: the primary it reads and writes is the one in `.active-kb`, not a
//! chat's. The single exception is `knowledge active --session <id>`, which
//! addresses one chat's pointer on purpose — a chat can hold an explicit "no
//! primary" override that only a session-scoped gesture can lift, and the CLI
//! is the escape hatch when the GUI is not the surface in front of the user.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use biorouter::config::Config;
use biorouter::knowledge::convert::SourceInput;
use biorouter::knowledge::macros::{
    ingest as ingest_macro, lint as lint_macro, query as query_macro,
};
use biorouter::knowledge::service::{KnowledgeService, PrimaryUpdate};
use biorouter::knowledge::subagent::loop_::{Completer, SubAgentBounds};
use biorouter::knowledge::validate::{Diagnostic, Diagnostics, Severity};
use biorouter::knowledge::ProviderCompleter;
use biorouter::model::ModelConfig;
use biorouter::privacy::ProviderTier;
use console::{style, Color};

/// Brand warm tan-brown accent (xterm-256 137 ≈ #af875f), Biorouter's light cream palette
const ACCENT: Color = Color::Color256(137);

fn service() -> Result<KnowledgeService> {
    KnowledgeService::new_default().map_err(|e| anyhow!("Failed to open knowledge store: {}", e))
}

/// Resolve the base a command operates on: the explicit `--kb` flag, else the
/// primary. Returns the id and, when it was resolved rather than given, a
/// notice the caller must print *before* doing any work — a KB-less write
/// must never be silent about which base it landed in.
fn resolve_kb(
    svc: &KnowledgeService,
    explicit: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(id) = explicit {
        return Ok((id, None));
    }
    if let Some(id) = svc.primary_for_session(None)? {
        let notice = format!(
            "  {} using primary knowledge base {}",
            style("·").dim(),
            style(&id).fg(ACCENT).bold()
        );
        return Ok((id, Some(notice)));
    }
    let ids = svc.session_kb_ids(None)?;
    if ids.is_empty() {
        bail!("No knowledge bases yet. Create one with `biorouter knowledge create <id> --name <name>`.");
    }
    bail!(
        "No primary knowledge base. Pass --kb <id> (one of: {}), or set one with \
         `biorouter knowledge active --set <id>`.",
        ids.join(", ")
    )
}

/// Build an LLM completer from the configured (or overridden) provider/model,
/// mirroring how the server builds one for the HTTP macros — **and** the tier of
/// the provider that was actually constructed (issue #56).
///
/// The tier comes back from here rather than being re-derived by each handler,
/// because `providers::create` intercepts `BIOROUTER_LEAD_MODEL` *before* the
/// registry lookup: `--provider ollama` can construct a lead/worker composite
/// whose tier is `least(lead, worker)` and therefore not the requested name's.
/// `ProviderCompleter::paired` reads the tier off the same `Arc` the completer
/// wraps, so the two cannot come from different providers.
async fn build_completer(
    provider: Option<String>,
    model: Option<String>,
) -> Result<(
    Box<dyn Completer>,
    ProviderTier,
    // Issue #56 DR-26 / Task 50: the third axis, off the same `Arc`.
    Option<biorouter::privacy::affiliation::ModelAffiliation>,
)> {
    // Honour the same offline test-mode switch the server uses, so CLI knowledge
    // macros (ingest / query / ingest-conversation) can be exercised without a
    // reachable LLM provider.
    if biorouter::knowledge::test_mode::env_enabled() {
        // ⚠ The SECOND of the two named literal exemptions (the other is
        // `routes/knowledge.rs`'s twin branch). There is no provider here to
        // read a tier from, and the fail-safe direction for a *ratchet* is not
        // to privatise a base on a test path.
        return Ok((
            Box::new(biorouter::knowledge::test_mode::TestModeCompleter),
            ProviderTier::Public,
            None,
        ));
    }
    let config = Config::global();
    let provider = provider
        .or_else(|| config.get_biorouter_provider().ok())
        .ok_or_else(|| anyhow!("No provider configured. Run `biorouter configure` first."))?;
    let model = model
        .or_else(|| config.get_biorouter_model().ok())
        .ok_or_else(|| anyhow!("No model configured. Run `biorouter configure` first."))?;

    let model_config = ModelConfig::new(&model)?;
    let provider = biorouter::providers::create(&provider, model_config).await?;
    let (completer, tier, affiliation) = ProviderCompleter::paired(provider);
    Ok((Box::new(completer), tier, affiliation))
}

/// The caller's identity for a `biorouter kb lint` that will **not** write
/// (issue #56). The CLI twin of `routes::knowledge::read_only_caller_identity`,
/// and it exists for the defect that route had.
///
/// The tier comes from [`build_completer`] — the one funnel — so it is read off
/// a *constructed instance* and never re-derived from the provider NAME, which
/// is the gap [`ProviderCompleter::paired`] exists to close and which the CLI is
/// most exposed to, because a name is all `--provider` ever supplies. The
/// completer that comes back is dropped: a scan has nothing to say to a model.
///
/// ⚠ **This branch used to answer with a hardcoded `Public`**, on the reasoning
/// that a scan constructs no provider and so has no instance to read a tier
/// from. The premise was a choice, not a fact, and the conclusion refused
/// `biorouter kb lint` on **every** private base for **every** caller — telling
/// the user to re-run on a private model, the one remedy that could not work
/// while no model was being read.
///
/// ⚠ **It does not fail.** `--fix` keeps `build_completer`'s error, because a
/// fix with no model cannot run; a scan can, and always could — `kb lint` on a
/// machine with nothing configured still prints its report, and that is a real
/// capability rather than an accident of the literal. A provider that will not
/// construct therefore resolves to Public with no affiliation: the restrictive
/// reading on both axes, so an identity that cannot be read can only ever
/// refuse, never admit.
async fn read_only_caller_identity(
    provider: Option<String>,
    model: Option<String>,
) -> (
    ProviderTier,
    Option<biorouter::privacy::affiliation::ModelAffiliation>,
) {
    match build_completer(provider, model).await {
        Ok((_completer, tier, affiliation)) => (tier, affiliation),
        Err(_) => (ProviderTier::Public, None),
    }
}

/// First 10 characters of a commit sha, for compact display.
fn short_sha(sha: &str) -> String {
    sha.chars().take(10).collect()
}

fn ingest_bounds() -> SubAgentBounds {
    SubAgentBounds {
        max_steps: 60,
        max_wall: Duration::from_secs(900),
        max_tokens: 200_000,
    }
}

fn section(title: &str) {
    println!("  {} {}", style("▌").fg(ACCENT), style(title).bold());
}

// ──────────────────────────────────────────────────────────────────────────────
// list
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_list(format: &str) -> Result<()> {
    let svc = service()?;
    // `render_list` builds a block with one newline per row; `println!` supplies
    // the last one, so trim or every listing ends in a blank line.
    println!("{}", render_list(&svc, format)?.trim_end());
    Ok(())
}

/// Render the base list. Visible bases are the set the agent uses; exactly one
/// of them may be the **primary** — the base a `--kb`-less write lands in — and
/// it is marked, which `cli.rs` has promised since the focus/discovery split.
fn render_list(svc: &KnowledgeService, format: &str) -> Result<String> {
    let bases = svc.list_bases()?;
    // One locked snapshot rather than a hidden read and a primary read that can
    // disagree — and, crucially, no `unwrap_or_default()`: swallowing a failed
    // read of `.hidden-kbs` rendered every base as visible to the agent while
    // the agent's own resolver errored on the same file. This listing is the
    // answer to "what can the agent see?", so it must refuse rather than guess.
    let selection = svc.selection(None)?;
    let hidden = selection.hidden_kbs;
    let primary = selection.primary_kb;

    if format == "json" {
        return Ok(serde_json::json!({
            "primary_kb": primary,
            "bases": bases.iter().map(|b| serde_json::json!({
                "id": b.id, "name": b.name, "color": b.color,
                "hidden": hidden.contains(&b.id),
                "primary": primary.as_deref() == Some(b.id.as_str()),
            })).collect::<Vec<_>>(),
        })
        .to_string());
    }

    let mut out = String::new();
    out.push_str(&format!(
        "  {} {}\n",
        style("▌").fg(ACCENT),
        style("Knowledge bases").bold()
    ));
    if bases.is_empty() {
        out.push_str(&format!(
            "    {}",
            style("none yet. Create one with `biorouter knowledge create <id> --name <name>`")
                .dim()
        ));
        return Ok(out);
    }
    out.push_str(&format!(
        "    {}\n",
        style(
            "Visible bases are available to the agent; the primary is where a --kb-less ingest writes."
        )
        .dim()
    ));
    let width = bases.iter().map(|b| b.id.len()).max().unwrap_or(0);
    for base in &bases {
        let is_hidden = hidden.contains(&base.id);
        // A filled accent dot = visible to the agent; a dim ring = hidden.
        let marker = if is_hidden {
            style("○").dim().to_string()
        } else {
            style("●").fg(ACCENT).to_string()
        };
        let suffix = if is_hidden {
            style("  (hidden)").dim().to_string()
        } else if primary.as_deref() == Some(base.id.as_str()) {
            style("  (primary)").fg(ACCENT).to_string()
        } else {
            String::new()
        };
        out.push_str(&format!(
            "    {} {:<width$}  {}{}\n",
            marker,
            style(&base.id).bold(),
            style(&base.name).dim(),
            suffix,
            width = width
        ));
    }
    Ok(out)
}

// ──────────────────────────────────────────────────────────────────────────────
// hide / unhide — control which bases the agent can see
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_hide(id: String) -> Result<()> {
    let svc = service()?;
    println!("{}", hide_command(&svc, &id)?);
    Ok(())
}

pub async fn handle_unhide(id: String) -> Result<()> {
    let svc = service()?;
    println!("{}", unhide_command(&svc, &id)?);
    Ok(())
}

/// Naming a base that does not exist is a typo, not a no-op: report it before
/// claiming success. The locked operation below re-checks under the lock and
/// remains the authority — this pre-check exists only to say `knowledge list`.
fn require_base(svc: &KnowledgeService, id: &str) -> Result<()> {
    if !svc.list_bases()?.iter().any(|b| b.id == id) {
        bail!(
            "No knowledge base with id '{}'. Run `biorouter knowledge list` to see them.",
            id
        );
    }
    Ok(())
}

fn hide_command(svc: &KnowledgeService, id: &str) -> Result<String> {
    require_base(svc, id)?;
    svc.hide_kb(None, id, PrimaryUpdate::Unchanged)?;
    Ok(format!(
        "  {} {} is now hidden from the agent",
        style("✓").green(),
        style(id).fg(ACCENT).bold()
    ))
}

fn unhide_command(svc: &KnowledgeService, id: &str) -> Result<String> {
    require_base(svc, id)?;
    svc.include_kb(None, id, PrimaryUpdate::Unchanged)?;
    Ok(format!(
        "  {} {} is now visible to the agent",
        style("✓").green(),
        style(id).fg(ACCENT).bold()
    ))
}

// ──────────────────────────────────────────────────────────────────────────────
// active
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_active(
    set: Option<String>,
    clear: bool,
    inherit: bool,
    session: Option<String>,
) -> Result<()> {
    let svc = service()?;
    println!(
        "{}",
        active_command(&svc, session.as_deref(), set, clear, inherit)?
    );
    Ok(())
}

/// Show, set, clear or un-override the **primary** knowledge base — the base a
/// `--kb`-less ingest/query/lint writes to. Setting one validates membership: a
/// base the CLI hides from the agent can never be the primary.
///
/// `session` is the one thing the CLI addresses that is not its own: everything
/// else here is machine-wide (see the module header). A chat-level `--inherit`
/// drops that chat's override so it follows the machine choice. At machine
/// scope, `--inherit` removes the explicit preference and restores Biorouter's
/// shipped Soul default.
fn active_command(
    svc: &KnowledgeService,
    session: Option<&str>,
    set: Option<String>,
    clear: bool,
    inherit: bool,
) -> Result<String> {
    // Three incompatible outcomes; clap rejects the combination first, but the
    // check belongs with the semantics, not only with the parser.
    if [set.is_some(), clear, inherit]
        .iter()
        .filter(|asked| **asked)
        .count()
        > 1
    {
        bail!("--set, --clear and --inherit are three different gestures; pass at most one.");
    }

    let scope = match session {
        Some(id) => format!(" for chat {}", style(id).fg(ACCENT).bold()),
        None => String::new(),
    };

    if inherit {
        let selection = svc.set_selection(session, None, PrimaryUpdate::Inherit)?;
        return Ok(match selection.primary_kb {
            Some(id) => match session {
                Some(session) => format!(
                    "  {} chat {} now follows the machine-wide primary knowledge base ({})",
                    style("✓").green(),
                    style(session).fg(ACCENT).bold(),
                    style(&id).fg(ACCENT).bold()
                ),
                None => format!(
                    "  {} machine-wide primary restored to the product default ({})",
                    style("✓").green(),
                    style(&id).fg(ACCENT).bold()
                ),
            },
            None => match session {
                Some(session) => format!(
                    "  {} chat {} now follows the machine-wide primary knowledge base (none is set)",
                    style("✓").green(),
                    style(session).fg(ACCENT).bold()
                ),
                None => format!(
                    "  {} product-default knowledge base is not installed",
                    style("✓").green()
                ),
            },
        });
    }

    if clear {
        svc.set_selection(session, None, PrimaryUpdate::Clear)?;
        return Ok(format!(
            "  {} primary knowledge base cleared{}",
            style("✓").green(),
            scope
        ));
    }

    if let Some(id) = set {
        svc.set_selection(session, None, PrimaryUpdate::Set(&id))
            .map_err(|e| anyhow!("{e} Run `biorouter knowledge list` to see them."))?;
        return Ok(format!(
            "  {} primary knowledge base{} set to {}",
            style("✓").green(),
            scope,
            style(&id).fg(ACCENT).bold()
        ));
    }

    Ok(match svc.primary_for_session(session)? {
        Some(id) => format!(
            "  {} {}{}",
            style("primary:").dim(),
            style(id).fg(ACCENT).bold(),
            scope
        ),
        None => format!(
            "  {}",
            style(format!(
                "no primary knowledge base{} (use --set <id>)",
                match session {
                    Some(id) => format!(" for chat {id}"),
                    None => String::new(),
                }
            ))
            .dim()
        ),
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// create
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_create(id: String, name: Option<String>, color: Option<String>) -> Result<()> {
    let svc = service()?;
    println!("{}", create_command(&svc, id, name, color)?);
    Ok(())
}

/// Create a base — and only create it.
///
/// This used to pin the new base as the primary whenever none was set, so the
/// next `--kb`-less ingest "just worked". That is exactly the invention the
/// model forbids: the primary is where a `--kb`-less write *commits*, and a
/// pointer the user never chose sends an ingest into a base by accident, as a
/// git commit in that base's history that is easy to miss. The first base is
/// not a special case — one candidate is still a candidate, not a choice. With
/// no primary, a KB-less command fails and lists the candidates, so instead of
/// guessing we say how to choose.
fn create_command(
    svc: &KnowledgeService,
    id: String,
    name: Option<String>,
    color: Option<String>,
) -> Result<String> {
    let name = name.unwrap_or_else(|| id.clone());
    let manifest = svc
        .create_base(&id, &name, color.as_deref())
        .map_err(|e| anyhow!("Failed to create knowledge base: {}", e))?;

    let mut out = format!(
        "  {} created knowledge base {} {}",
        style("✓").green(),
        style(&manifest.id).fg(ACCENT).bold(),
        style(format!("({})", manifest.name)).dim()
    );

    if svc.primary_for_session(None)?.is_none() {
        out.push_str(&format!(
            "\n  {} {}",
            style("·").dim(),
            style(format!(
                "no primary knowledge base yet. Set one with `biorouter knowledge active --set {}`",
                manifest.id
            ))
            .dim()
        ));
    }
    Ok(out)
}

// ──────────────────────────────────────────────────────────────────────────────
// ingest
// ──────────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn handle_ingest(
    kb: Option<String>,
    url: Option<String>,
    file: Option<PathBuf>,
    text: Option<String>,
    focus: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let svc = service()?;
    let (kb_id, notice) = resolve_kb(&svc, kb)?;
    if let Some(notice) = notice {
        println!("{notice}");
    }

    let source = match (url, file, text) {
        (Some(u), None, None) => SourceInput::Url(u),
        (None, Some(p), None) => {
            if !p.exists() {
                bail!("File not found: {}", p.display());
            }
            SourceInput::Path(p)
        }
        (None, None, Some(t)) => SourceInput::Text {
            text: t,
            title: None,
        },
        (None, None, None) => {
            bail!("Provide a source: --url <url>, --file <path>, or --text <text>")
        }
        _ => bail!("Provide exactly one of --url, --file, or --text"),
    };

    let (completer, caller_capability, caller_affiliation) =
        build_completer(provider, model).await?;

    let spinner = cliclack::spinner();
    spinner.start(format!("ingesting into {}...", kb_id));

    let result = ingest_macro::ingest(
        &svc,
        ingest_macro::IngestArgs {
            kb_id: kb_id.clone(),
            // Issue #56. The tier of the provider that was CONSTRUCTED, never
            // of the `--provider` name the user typed.
            caller_is_private: caller_capability.is_private(),
            caller_affiliation: biorouter::privacy::affiliation::caller_affiliation(
                caller_affiliation,
            ),
            source,
            completer,
            focus,
            bounds: ingest_bounds(),
            event_sink: None,
            cancel: None,
        },
    )
    .await;

    spinner.stop("");

    match result {
        Ok(res) => {
            println!(
                "  {} ingested into {} {}",
                style("✓").green(),
                style(&kb_id).fg(ACCENT).bold(),
                style(format!("({} steps)", res.steps)).dim()
            );
            println!(
                "    {} {}",
                style("source:").dim(),
                style(&res.source_id).dim()
            );
            println!(
                "    {} {}",
                style("commit:").dim(),
                style(short_sha(&res.commit_sha)).dim()
            );
            print_verification(&kb_id, &res.verification);
            Ok(())
        }
        Err(e) => Err(anyhow!("Ingest failed: {}", e)),
    }
}

/// What the digest left behind, in one line.
///
/// The gap this fills is the same one the macro's tail check fills: a run that
/// printed `✓ ingested into <kb>` and nothing else could have committed fifteen
/// non-conformant pages, and the user had no reason to look. It never fails the
/// command — DR-7 makes these findings a description of the base, not a verdict
/// on the run — so it points at `knowledge lint`, which is where the findings
/// themselves live.
fn print_verification(kb_id: &str, v: &ingest_macro::Verification) {
    let label = style("verify:").dim();
    if let Some(err) = &v.scan_error {
        // Not silence, and not `clean`: the check did not run, which is a third
        // answer and the only one that should send the user to look themselves.
        println!(
            "    {label} {}",
            style(format!("could not check the base: {err}")).yellow()
        );
        return;
    }
    if v.diagnostics.total == 0 {
        println!("    {label} {}", style("clean").green());
        return;
    }
    let summary = format!("{} error(s), {} warning(s)", v.errors, v.warnings);
    println!(
        "    {label} {}",
        if v.ok {
            style(summary).yellow()
        } else {
            style(summary).red()
        }
    );
    println!(
        "      {}",
        style(format!(
            "run `biorouter knowledge lint --kb {kb_id}` to see them"
        ))
        .dim()
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// ingest-conversation
// ──────────────────────────────────────────────────────────────────────────────

/// The target base for `ingest-conversation`: `--new-kb` creates one (and says
/// so), else `--kb`, else the session's primary.
///
/// Split out of [`handle_ingest_conversation`] only to keep that function under
/// `clippy::too_many_lines`; it carries no decision of its own. ⚠ Not shared
/// with the other commands: they all resolve through [`resolve_kb`] and must
/// keep doing so — the `--new-kb` branch CREATES a base, which is a write, and
/// no read path should be able to reach it.
fn resolve_ingest_target_kb(
    svc: &KnowledgeService,
    kb: Option<String>,
    new_kb: Option<String>,
) -> Result<String> {
    let Some(name) = new_kb else {
        let (kb_id, notice) = resolve_kb(svc, kb)?;
        if let Some(notice) = notice {
            println!("{notice}");
        }
        return Ok(kb_id);
    };
    let id = biorouter::agents::knowledge_tool::slugify_kb_name(&name);
    if id.is_empty() {
        bail!("--new-kb must contain letters or numbers");
    }
    if !svc.list_bases()?.iter().any(|b| b.id == id) {
        svc.create_base(&id, name.trim(), None)?;
        println!(
            "  {} created knowledge base {}",
            style("✓").green(),
            style(&id).fg(ACCENT).bold()
        );
    }
    Ok(id)
}

/// Digest one or more chat sessions into a knowledge base.
pub async fn handle_ingest_conversation(
    kb: Option<String>,
    sessions: Vec<String>,
    new_kb: Option<String>,
    focus: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<()> {
    use biorouter::knowledge::conversation_ingest::{ingest_conversation, ConversationIngestArgs};
    use biorouter::session::session_manager::SessionManager;

    let svc = service()?;
    let kb_id = resolve_ingest_target_kb(&svc, kb, new_kb)?;

    // Resolve which sessions to ingest. Default: the most recent session.
    let manager = SessionManager::instance();
    let session_ids: Vec<String> = if sessions.is_empty() {
        let mut all = manager.list_sessions().await?;
        all.sort_by_key(|s| s.updated_at);
        match all.last() {
            Some(s) => {
                println!(
                    "  {} no --session given, using most recent session {}",
                    style("·").dim(),
                    style(&s.id).dim()
                );
                vec![s.id.clone()]
            }
            None => bail!("No sessions found to ingest."),
        }
    } else {
        sessions
    };

    let mut loaded = Vec::new();
    for sid in &session_ids {
        loaded.push(
            manager
                .get_session(sid, true)
                .await
                .map_err(|e| anyhow!("session '{sid}' not found: {e}"))?,
        );
    }

    let (completer, caller_capability, caller_affiliation) =
        build_completer(provider, model).await?;

    let spinner = cliclack::spinner();
    spinner.start(format!("digesting {} chat(s)...", loaded.len()));

    let result = ingest_conversation(
        &svc,
        ConversationIngestArgs {
            kb_id: kb_id.clone(),
            // Issue #56. The tier of the provider that was CONSTRUCTED.
            caller_capability,
            caller_affiliation,
            session_manager: std::sync::Arc::new(SessionManager::instance()),
            sessions: loaded,
            completer,
            focus,
            bounds: ingest_bounds(),
            event_sink: None,
            cancel: None,
        },
    )
    .await;

    spinner.stop("");

    match result {
        Ok(res) => {
            println!(
                "  {} {} chat(s) ingested into {} {}",
                style("✓").green(),
                session_ids.len().saturating_sub(res.refused),
                style(&kb_id).fg(ACCENT).bold(),
                style(format!("({} steps)", res.ingested.steps)).dim()
            );
            println!(
                "    {} {}",
                style("source:").dim(),
                style(&res.ingested.source_id).dim()
            );
            println!(
                "    {} {}",
                style("commit:").dim(),
                style(short_sha(&res.ingested.commit_sha)).dim()
            );
            print_verification(&kb_id, &res.ingested.verification);
            // Issue #56, Gate G. A COUNT and nothing else — a session's id,
            // title and working directory are all content (§11.4).
            if res.refused > 0 {
                println!(
                    "  {} {} private chat(s) skipped: this model is public. \
                     Re-run with a private --provider/--model to include them.",
                    style("!").yellow(),
                    res.refused
                );
            }
            Ok(())
        }
        Err(e) => Err(anyhow!("Chat ingest failed: {}", e)),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// lint
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_lint(
    kb: Option<String>,
    fix: bool,
    provider: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let svc = service()?;
    let (kb_id, notice) = resolve_kb(&svc, kb)?;
    if let Some(notice) = notice {
        println!("{notice}");
    }

    // Issue #56: only a `--fix` needs a COMPLETER, but both paths need the
    // caller's capability — see `read_only_caller_identity` for why a scan asks
    // the same question a fix does, and why it may not answer with a literal.
    let (completer, caller_capability, caller_affiliation) = if fix {
        let (c, tier, affiliation) = build_completer(provider, model).await?;
        (Some(c), tier, affiliation)
    } else {
        let (tier, affiliation) = read_only_caller_identity(provider, model).await;
        (None, tier, affiliation)
    };

    let spinner = cliclack::spinner();
    spinner.start(format!("linting {}...", kb_id));

    let result = lint_macro::lint(
        &svc,
        lint_macro::LintArgs {
            kb_id: kb_id.clone(),
            // Issue #56. The tier of the provider the autofix will run on.
            caller_is_private: caller_capability.is_private(),
            caller_affiliation: biorouter::privacy::affiliation::caller_affiliation(
                caller_affiliation,
            ),
            completer,
            autofix: fix,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .map_err(|e| anyhow!("Lint failed: {}", e))?;

    spinner.stop("");

    section(&format!("Lint report: {}", kb_id));
    print_diagnostics(&result.report.diagnostics);

    // ⚠ `result.report` is the scan the run STARTED from, so after an autofix it
    // describes a base that no longer exists — and an exit code taken from it
    // would fail a run whose whole job was to make those findings go away.
    // Re-scan instead: it is deterministic, needs no model, and is the same
    // function the report came from, so the verdict describes the base as it now
    // stands.
    let verdict = if fix {
        println!(
            "  {} {} fix(es) applied",
            style("✓").green(),
            result.fixes_applied
        );
        let kb_root = biorouter::knowledge::paths::kb_root(svc.root(), &kb_id);
        let after = lint_macro::scan(&kb_root)
            .map_err(|e| anyhow!("Re-scan after autofix failed: {}", e))?;
        println!("  {} after autofix:", style("·").dim());
        print_diagnostics(&after.diagnostics);
        after.diagnostics
    } else {
        result.report.diagnostics.clone()
    };

    // Non-zero on errors, so `biorouter knowledge lint` is usable as a gate in a
    // script or in CI. Warnings never fail: DR-7 makes them SHOULDs, and a
    // command that failed on every orphan page would be turned off within a day.
    if let Some(errors) = error_verdict(&verdict) {
        bail!(
            "{kb_id}: {errors}. Nothing was rejected and every page is still \
             readable — but a base with errors is not ready to export or share."
        );
    }
    if !fix && !verdict.is_empty() {
        println!(
            "  {}",
            style("re-run with --fix to let the sub-agent repair these").dim()
        );
    }
    Ok(())
}

/// How the failure names the errors it is failing on, or `None` when there are
/// none — the exit code and this sentence come from one place so they cannot
/// disagree about whether the run failed.
///
/// ⚠ **It has to say which population it counted.** The headline above reports
/// the PRE-CAP total and this line counted the CAPPED list, so a 130-page base
/// printed `780 finding(s)` and then `Error: 200 conformance error(s)` — two
/// numbers, two populations, and nothing on screen saying so. A reader
/// reasonably concludes 580 findings are warnings. The exit code was never
/// wrong; the arithmetic the user did from it was.
///
/// The exactness rule is a property of [`Diagnostics::new`], not a guess:
/// errors sort **first** and the cap cuts from the end, so the kept list holds
/// *every* error — unless the kept list is nothing but errors, which is the one
/// case where a capped report genuinely cannot know the total, and the only one
/// that says "at least".
fn error_verdict(d: &Diagnostics) -> Option<String> {
    let errors = d.count(Severity::Error);
    if errors == 0 {
        return None;
    }
    let exact = !d.truncated() || errors < d.items.len();
    let count = if exact {
        format!("{errors}")
    } else {
        format!("at least {errors}")
    };
    Some(format!(
        "{count} of the {} finding(s) above are conformance error(s)",
        d.total
    ))
}

/// The typed diagnostics, grouped by severity, each with its stable rule id.
///
/// **The bug this replaced was a confident false negative.** This printer knew
/// only the four legacy hygiene lists — orphans, contradictions, stale sources,
/// missing concept pages — and ignored `report.diagnostics` entirely. So a
/// BioOKF base carrying eleven `biookf.*` ERRORS printed
/// `0 orphan page(s) … 0 missing concept page(s)` and then `✓ clean`. That is
/// worse than printing nothing: a user who ran the check and was told the base
/// was fine has no reason to look again.
///
/// The four lists are not lost. Each entry re-appears here as a `kb.*`
/// diagnostic carrying the same subject (`macros::lint::scan_diagnostics`), so
/// this shows a superset of what it replaced.
fn print_diagnostics(d: &Diagnostics) {
    if d.total == 0 {
        println!("  {} clean", style("✓").green());
        return;
    }
    println!(
        "  {}",
        style(format!(
            "{} finding(s)",
            // The PRE-CAP total, always. A truncated list reporting its own
            // length is how "3 errors" gets printed for a base with four
            // hundred.
            d.total
        ))
        .yellow()
        .bold()
    );
    if d.truncated() {
        println!(
            "    {}",
            style(format!(
                "first {} shown; fix a batch and re-run to see the rest",
                d.items.len()
            ))
            .dim()
        );
    }
    for severity in [Severity::Error, Severity::Warning, Severity::Info] {
        print_severity_group(d, severity);
    }
}

/// One severity's findings, or nothing at all when it has none.
///
/// Split out of [`print_diagnostics`] so that function stays short; it carries
/// no decision of its own.
fn print_severity_group(d: &Diagnostics, severity: Severity) {
    let items: Vec<&Diagnostic> = d
        .items
        .iter()
        .filter(|item| item.severity == severity)
        .collect();
    if items.is_empty() {
        return;
    }
    let (word, colour) = match severity {
        Severity::Error => ("error(s)", Color::Red),
        Severity::Warning => ("warning(s)", Color::Yellow),
        Severity::Info => ("info", Color::Cyan),
    };
    println!(
        "    {}",
        style(format!("{} {}", items.len(), word)).fg(colour)
    );
    for item in items {
        // The rule id first and undimmed: it is the stable handle a reader
        // greps for, matches on, and looks up in the knowledge-lint skill —
        // the message is prose and will be reworded.
        println!(
            "      {} {} {}",
            style("·").dim(),
            style(&item.rule).fg(colour),
            item.subject
        );
        println!("        {}", style(&item.message).dim());
        // Only when it adds something: `kb.*` findings use the page path as
        // their subject, so printing it again would be one line of noise per
        // finding.
        match &item.path {
            Some(path) if *path != item.subject => {
                println!("        {}", style(path).dim())
            }
            _ => {}
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// query
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_query(
    question: String,
    kb: Option<String>,
    save: bool,
    provider: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let svc = service()?;
    let (kb_id, notice) = resolve_kb(&svc, kb)?;
    if let Some(notice) = notice {
        println!("{notice}");
    }
    let (completer, caller_capability, caller_affiliation) =
        build_completer(provider, model).await?;

    let spinner = cliclack::spinner();
    spinner.start(format!("querying {}...", kb_id));

    let result = query_macro::query(
        &svc,
        query_macro::QueryArgs {
            kb_id: kb_id.clone(),
            // Issue #56. `query` writes — see `QueryArgs::caller_is_private`.
            caller_is_private: caller_capability.is_private(),
            caller_affiliation: biorouter::privacy::affiliation::caller_affiliation(
                caller_affiliation,
            ),
            question,
            completer,
            file_as_page: save,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .map_err(|e| anyhow!("Query failed: {}", e))?;

    spinner.stop("");

    section(&format!("Answer: {}", kb_id));
    println!("{}", result.answer);

    if !result.cited_pages.is_empty() {
        println!();
        println!("  {}", style("cited pages").dim());
        for page in &result.cited_pages {
            println!("    {} {}", style("·").dim(), style(page).fg(ACCENT));
        }
    }
    if save {
        if let Some(sha) = &result.commit_sha {
            println!(
                "  {} saved as a page {}",
                style("✓").green(),
                style(format!("({})", short_sha(sha))).dim()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        active_command, create_command, error_verdict, hide_command, render_list, unhide_command,
    };
    use biorouter::knowledge::service::{KnowledgeService, PrimaryUpdate};
    use biorouter::knowledge::validate::{Diagnostic, Diagnostics, Severity, MAX_DIAGNOSTICS};

    fn raised(errors: usize, warnings: usize) -> Diagnostics {
        let mut all: Vec<Diagnostic> = (0..errors)
            .map(|i| Diagnostic::scan("x.err", Severity::Error, format!("p{i}.md"), "e"))
            .collect();
        all.extend(
            (0..warnings)
                .map(|i| Diagnostic::scan("x.warn", Severity::Warning, format!("q{i}.md"), "w")),
        );
        Diagnostics::new(all)
    }

    /// The failure line and the headline above it must be about the same
    /// population — the bug was two counts over two populations with nothing
    /// saying so, so every row here pins the denominator as well as the number.
    #[test]
    fn the_lint_failure_counts_errors_over_the_same_findings_the_headline_did() {
        assert_eq!(error_verdict(&raised(0, 5)), None, "warnings must not fail");
        assert_eq!(
            error_verdict(&raised(3, 40)).unwrap(),
            "3 of the 43 finding(s) above are conformance error(s)"
        );
        // Truncated, but every error survived the cut — errors sort first, so
        // the number is exact and must not be hedged.
        let mut d = raised(3, 400);
        assert!(d.truncated());
        assert_eq!(
            error_verdict(&d).unwrap(),
            "3 of the 403 finding(s) above are conformance error(s)"
        );
        // The case that printed `780 finding(s)` and then `200 conformance
        // error(s)`: the kept list is nothing but errors, so 200 is a floor.
        d = raised(600, 180);
        assert_eq!(d.count(Severity::Error), MAX_DIAGNOSTICS);
        assert_eq!(
            error_verdict(&d).unwrap(),
            "at least 200 of the 780 finding(s) above are conformance error(s)"
        );
    }

    fn svc() -> (tempfile::TempDir, KnowledgeService) {
        let tmp = tempfile::TempDir::new().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("alpha", "Alpha", None).unwrap();
        svc.create_base("beta", "Beta", None).unwrap();
        (tmp, svc)
    }

    /// First-ever CLI coverage for the knowledge commands. `--set` used to
    /// validate only that the base existed, so it would happily pin a base the
    /// CLI hides from the agent — a primary outside the set.
    #[test]
    fn active_command_shows_sets_validates_and_clears_the_primary() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();

        assert!(
            active_command(&svc, None, None, false, false)?.contains("no primary knowledge base")
        );
        assert!(
            active_command(&svc, None, Some("beta".to_string()), false, false)?.contains("beta")
        );
        assert!(active_command(&svc, None, None, false, false)?.contains("beta"));

        svc.set_hidden_persisted(&["alpha".to_string()])?;
        let err = active_command(&svc, None, Some("alpha".to_string()), false, false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("alpha") && err.contains("knowledge list"),
            "a hidden base cannot be the primary, and the error must say how to look, got: {err}"
        );

        assert!(active_command(&svc, None, None, true, false)?.contains("cleared"));
        assert_eq!(svc.primary_for_session(None)?, None);
        Ok(())
    }

    /// The explicit "no primary" override is durable by design — that is the
    /// whole point of the blank-file state — but it used to be a **one-way
    /// door**. `delete_base` installs one in every chat that had pinned the
    /// deleted base, and nothing on the CLI or the HTTP surface could take it
    /// back off, so such a chat could never follow the machine-wide default
    /// again. `--inherit` is the way back.
    #[test]
    fn inherit_reopens_a_chat_that_was_left_with_an_explicit_no_primary() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();
        svc.create_base("gamma", "Gamma", None)?;
        svc.set_selection(None, None, PrimaryUpdate::Set("alpha"))?;

        // The chat pinned a base of its own, and that base was then deleted —
        // which must not silently hand the chat the machine-wide default.
        svc.set_primary_for_session("s1", Some("gamma"))?;
        svc.delete_base("gamma")?;
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);
        // Every other gesture leaves the override in place.
        assert!(active_command(&svc, Some("s1"), None, false, false)?
            .contains("no primary knowledge base for chat s1"));

        let out = active_command(&svc, Some("s1"), None, false, true)?;
        assert!(
            out.contains("s1") && out.contains("alpha"),
            "the confirmation must name the chat and the default it now follows, got: {out}"
        );
        assert_eq!(
            svc.primary_for_session(Some("s1"))?.as_deref(),
            Some("alpha"),
            "the chat must follow the machine-wide primary again"
        );
        // And it keeps following it, rather than having copied the value once.
        svc.set_selection(None, None, PrimaryUpdate::Set("beta"))?;
        assert_eq!(
            svc.primary_for_session(Some("s1"))?.as_deref(),
            Some("beta")
        );

        // Machine-level inherit means "return to the product default". This
        // fixture intentionally has no Soul base, so the command reports that
        // precisely rather than inventing another primary.
        let out = active_command(&svc, None, None, false, true)?;
        assert!(out.contains("product-default") && out.contains("not installed"));

        // The three gestures remain mutually exclusive.
        let err = active_command(&svc, Some("s1"), Some("beta".to_string()), false, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("at most one"), "got: {err}");
        Ok(())
    }

    /// `--session` makes the machine-wide CLI able to address one chat's
    /// pointer. Scope-specific changes must not leak in either direction.
    #[test]
    fn a_session_scoped_primary_does_not_disturb_the_machine_one() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();
        svc.set_selection(None, None, PrimaryUpdate::Set("alpha"))?;

        assert!(
            active_command(&svc, Some("s1"), Some("beta".to_string()), false, false)?
                .contains("beta")
        );
        assert_eq!(
            svc.primary_for_session(Some("s1"))?.as_deref(),
            Some("beta")
        );
        assert_eq!(
            svc.primary_for_session(None)?.as_deref(),
            Some("alpha"),
            "a chat's pin must not move the machine-wide pointer"
        );

        assert!(active_command(&svc, Some("s1"), None, true, false)?.contains("for chat s1"));
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);
        assert_eq!(
            svc.primary_for_session(None)?.as_deref(),
            Some("alpha"),
            "clearing a chat's primary must not clear the machine-wide one"
        );
        Ok(())
    }

    /// `cli.rs:901` has promised "the active one is marked" since the
    /// focus/discovery split; `handle_list` only ever marked hidden-vs-visible.
    #[test]
    fn list_marks_the_primary_base() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();
        svc.set_selection(None, None, PrimaryUpdate::Set("beta"))?;

        let text = render_list(&svc, "text")?;
        assert!(
            text.contains("beta") && text.contains("primary"),
            "got: {text}"
        );

        let json: serde_json::Value = serde_json::from_str(&render_list(&svc, "json")?)?;
        assert_eq!(json["primary_kb"], serde_json::json!("beta"));
        let beta = json["bases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == serde_json::json!("beta"))
            .unwrap();
        assert_eq!(beta["primary"], serde_json::json!(true));
        Ok(())
    }

    /// A `--kb`-less ingest/query/lint resolves its target silently. It must
    /// hand back a notice so the command can say where it is about to write —
    /// an ingest commits to that base's git history and is hard to notice
    /// afterwards.
    #[test]
    fn resolve_kb_names_the_primary_and_lists_candidates_when_there_is_none() -> anyhow::Result<()>
    {
        let (_tmp, svc) = svc();

        let err = super::resolve_kb(&svc, None).unwrap_err().to_string();
        assert!(
            err.contains("alpha, beta") && err.contains("--kb"),
            "with no primary the error must list the candidates, got: {err}"
        );

        svc.set_selection(None, None, PrimaryUpdate::Set("beta"))?;
        let (id, notice) = super::resolve_kb(&svc, None)?;
        assert_eq!(id, "beta");
        assert!(
            notice
                .expect("a resolved primary must be announced")
                .contains("beta"),
            "the notice must name the base"
        );

        let (id, notice) = super::resolve_kb(&svc, Some("alpha".to_string()))?;
        assert_eq!(id, "alpha");
        assert!(notice.is_none(), "an explicit --kb needs no notice");
        Ok(())
    }

    /// The primary is an explicit choice and is **never invented** — not even
    /// for the very first base on a fresh machine, the one case where guessing
    /// looks harmless. It is not: the next `--kb`-less ingest then commits into
    /// a base the user never picked, and an ingest is a git commit in that
    /// base's history that is hard to notice afterwards. Creating a base only
    /// creates it; a KB-less write must still fail with the candidate list.
    #[test]
    fn creating_a_base_never_invents_a_primary() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());

        let out = create_command(&svc, "alpha".to_string(), None, None)?;
        assert!(out.contains("alpha"), "got: {out}");
        assert_eq!(
            svc.primary_for_session(None)?,
            None,
            "creating the first base must not silently make it the primary"
        );
        assert!(
            out.contains("knowledge active --set"),
            "the user must be told how to choose one, got: {out}"
        );

        // …and the KB-less path is the candidate-listing error, not a guess.
        let err = super::resolve_kb(&svc, None).unwrap_err().to_string();
        assert!(err.contains("alpha") && err.contains("--kb"), "got: {err}");

        // An explicit choice sticks, and a later create does not move it.
        svc.set_selection(None, None, PrimaryUpdate::Set("alpha"))?;
        let out = create_command(&svc, "beta".to_string(), None, None)?;
        assert_eq!(svc.primary_for_session(None)?.as_deref(), Some("alpha"));
        assert!(
            !out.contains("knowledge active --set"),
            "no nudge once a primary exists, got: {out}"
        );
        Ok(())
    }

    /// `hide` / `unhide` used to read the hidden list, edit it in memory and
    /// write the whole thing back. Nothing serialises those two unlocked calls,
    /// so two overlapping invocations — separate processes, or the desktop app
    /// writing the same file while a shell command runs — each write a list
    /// computed before the other's edit and one hide silently vanishes. The
    /// user sees "✓ gamma is now hidden" and gamma is still visible.
    #[test]
    fn concurrent_hides_from_separate_invocations_all_survive() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let root = tmp.path().to_path_buf();
        let svc = KnowledgeService::new(root.clone());
        let ids = ["alpha", "beta", "gamma", "delta"];
        for id in ids {
            svc.create_base(id, id, None)?;
        }

        for round in 0..8 {
            svc.clear_hidden_persisted()?;
            let gate = std::sync::Arc::new(std::sync::Barrier::new(ids.len()));
            let handles: Vec<_> = ids
                .iter()
                .map(|id| {
                    // A fresh service per thread: each stands in for its own
                    // `biorouter knowledge hide <id>` process.
                    let svc = KnowledgeService::new(root.clone());
                    let id = id.to_string();
                    let gate = gate.clone();
                    std::thread::spawn(move || -> anyhow::Result<()> {
                        gate.wait();
                        hide_command(&svc, &id)?;
                        Ok(())
                    })
                })
                .collect();
            for handle in handles {
                handle.join().expect("hide thread panicked")?;
            }

            let mut hidden = svc.get_hidden_persisted()?;
            hidden.sort();
            assert_eq!(
                hidden.len(),
                ids.len(),
                "round {round}: a concurrent hide was lost, got {hidden:?}"
            );
            assert!(
                svc.session_kb_ids(None)?.is_empty(),
                "round {round}: every base was hidden, so the set must be empty"
            );
        }
        Ok(())
    }

    /// `list` is the surface that answers "what can the agent see?", so it must
    /// not answer from a file it failed to read. It took the hidden list with
    /// `unwrap_or_default()`, which turns an unreadable or corrupt
    /// `.hidden-kbs` into "nothing is hidden" — every base rendered as
    /// available to the agent, while the agent's own resolver errors on the
    /// same file. (With a primary pinned the error surfaced by accident
    /// through `primary_for_session`; with none pinned, which is now the
    /// default state after a create, nothing caught it.)
    #[test]
    fn list_refuses_to_report_a_hidden_list_it_could_not_read() -> anyhow::Result<()> {
        let (tmp, svc) = svc();
        std::fs::write(
            biorouter::knowledge::paths::hidden_kbs_path(tmp.path()),
            b"{ not json }",
        )?;

        assert!(
            svc.session_kb_ids(None).is_err(),
            "precondition: the agent's own resolver rejects this file"
        );
        assert!(
            render_list(&svc, "text").is_err(),
            "so the listing must not claim every base is visible"
        );
        assert!(render_list(&svc, "json").is_err());
        Ok(())
    }

    /// Unhiding a base nobody has heard of used to print "✓ … is now visible"
    /// and change nothing, so a typo read as success.
    #[test]
    fn hide_and_unhide_reject_a_base_that_does_not_exist() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();

        for err in [
            hide_command(&svc, "ghost").unwrap_err().to_string(),
            unhide_command(&svc, "ghost").unwrap_err().to_string(),
        ] {
            assert!(
                err.contains("ghost") && err.contains("knowledge list"),
                "got: {err}"
            );
        }

        // The real round trip still works.
        hide_command(&svc, "alpha")?;
        assert_eq!(svc.session_kb_ids(None)?, vec!["beta".to_string()]);
        unhide_command(&svc, "alpha")?;
        assert_eq!(
            svc.session_kb_ids(None)?,
            vec!["alpha".to_string(), "beta".to_string()]
        );
        Ok(())
    }

    // ── Issue #56, Task 10B: the CLI's capability ───────────────────────────

    mod privacy_tier {
        use super::super::{build_completer, handle_ingest, handle_lint};
        use biorouter::privacy::ProviderTier;
        use serial_test::serial;

        /// The variables every row below pins, under the workspace's
        /// process-wide environment lock.
        ///
        /// `env_lock` and not a hand-rolled guard plus `#[serial]`: `#[serial]`
        /// serialises against other `#[serial]` tests only, while every other
        /// test in this binary runs concurrently, and the rest of the workspace
        /// already takes this same lock for `BIOROUTER_PATH_ROOT`. Two
        /// mechanisms in one process do not compose.
        ///
        /// `BIOROUTER_KNOWLEDGE_TEST_MODE` must be OFF: `build_completer`'s
        /// early return hands back a `TestModeCompleter` and Public before any
        /// provider exists, which would make every row Public and the whole
        /// matrix vacuous.
        ///
        /// ⚠ The private rows' loopback port is **1**, never 11434.
        /// `is_loopback_host` reads the HOST, so any port is Private, and
        /// nothing can listen on port 1 without root — while 11434 would make
        /// `the_cli_ingest_handler_…` row drive a live local model on any
        /// developer machine running Ollama.
        fn base_env(host: &str) -> Vec<(&'static str, Option<String>)> {
            vec![
                ("BIOROUTER_KNOWLEDGE_TEST_MODE", None),
                ("BIOROUTER_LEAD_MODEL", None),
                ("BIOROUTER_LEAD_PROVIDER", None),
                ("BIOROUTER_DISABLE_KEYRING", Some("1".to_string())),
                ("OLLAMA_HOST", Some(host.to_string())),
                ("OLLAMA_TIMEOUT", Some("1".to_string())),
            ]
        }

        /// Take the workspace env lock over `pairs`.
        fn lock_env(pairs: Vec<(&'static str, Option<String>)>) -> env_lock::EnvGuard<'static> {
            env_lock::lock_env(pairs)
        }

        #[tokio::test]
        #[serial]
        async fn the_cli_sources_its_capability_from_the_provider_it_constructed() {
            // ⚠ THE PROVIDER NAME IS `ollama` IN BOTH ROWS, and that is the whole
            // construction: an implementation that keys on the requested name
            // gives the same answer twice and fails one row, and so does either
            // hardcoded literal.
            //
            // What varies is `OLLAMA_HOST`: Task 5 makes a loopback Ollama
            // Private and a non-loopback one Public (`is_loopback_host`), which
            // is exactly a same-name/different-tier pair with no credential and
            // no network anywhere in it. `ModelConfig::new` accepts an unknown
            // model name, so neither row needs a real model either.
            for (host, want) in [
                ("http://127.0.0.1:1", ProviderTier::Private),
                ("http://ollama.invalid:11434", ProviderTier::Public),
            ] {
                let _env = lock_env(base_env(host));
                let (_c, tier, _a) =
                    build_completer(Some("ollama".into()), Some("qwen3.5:4b".into()))
                        .await
                        .unwrap();
                assert_eq!(tier, want, "OLLAMA_HOST={host}");
            }
        }

        #[tokio::test]
        #[serial]
        async fn the_cli_capability_follows_the_instance_not_the_name_the_user_typed() {
            // The row that fails `provider_name == "ollama"`. `providers::create`
            // intercepts BIOROUTER_LEAD_MODEL *before* the registry lookup, so
            // `--provider ollama` can construct a lead/worker composite whose
            // tier is `least(lead, worker)` — and a PUBLIC lead makes the whole
            // instance Public even though the name typed was the private one.
            let mut env = base_env("http://127.0.0.1:1");
            env.push(("BIOROUTER_LEAD_MODEL", Some("gpt-5".to_string())));
            env.push((
                "BIOROUTER_LEAD_PROVIDER",
                Some("github_copilot".to_string()),
            ));
            let _env = lock_env(env);

            let (_c, tier, _a) = build_completer(Some("ollama".into()), Some("qwen3.5:4b".into()))
                .await
                .unwrap();
            assert_eq!(
                tier,
                ProviderTier::Public,
                "the CLI keyed its capability on the name the user typed"
            );
        }

        /// `service()` is `KnowledgeService::new_default()`, whose root is
        /// `paths::knowledge_root()` → `$BIOROUTER_PATH_ROOT/config/knowledge`
        /// when the override is set. That is how this row gets a throwaway store
        /// instead of the developer's own.
        ///
        /// ⚠ The caller owns the `TempDir` and must keep it BOUND — a dropped
        /// one deletes the tree before the call runs. It also relocates
        /// `BIOROUTER_PATH_ROOT` itself, through
        /// `crate::test_sandbox::relocate_path_root_and` in a process of its own,
        /// so that every variable this module touches is set under the
        /// workspace lock and none behind its back.
        fn cli_knowledge_root_with_base(tmp: &tempfile::TempDir, id: &str) -> std::path::PathBuf {
            let root = tmp.path().join("config").join("knowledge");
            std::fs::create_dir_all(&root).unwrap();
            let svc = biorouter::knowledge::service::KnowledgeService::new(root.clone());
            svc.create_base(id, id, None).unwrap();
            root
        }

        #[tokio::test]
        #[serial]
        async fn the_cli_ingest_handler_ratchets_from_the_instance_and_the_name_is_the_same_in_both_legs(
        ) {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            // Round 3 §7: a handler can call `paired`, ignore its tier, and derive
            // the capability from the requested provider name — every structural
            // count in Step 5 still passes. Only a handler-level behavioural row
            // sees that, and only if the NAME is constant across the legs.
            for (host, want_private) in [
                ("http://127.0.0.1:1", true),
                ("http://ollama.invalid:11434", false),
            ] {
                let tmp = tempfile::TempDir::new().unwrap();
                let _env = crate::test_sandbox::relocate_path_root_and(tmp.path(), base_env(host));
                let root = cli_knowledge_root_with_base(&tmp, "k");

                // The sub-agent WILL fail — nothing answers on either host — and
                // that is the point: CP2 raises before it runs, so the ratchet is
                // observable without an LLM.
                let _ = handle_ingest(
                    Some("k".into()),
                    None,
                    None,
                    Some("n=412".into()),
                    None,
                    Some("ollama".into()),
                    Some("qwen3.5:4b".into()),
                )
                .await;

                assert_eq!(
                    biorouter::knowledge::tier::is_private(&root, "k"),
                    want_private,
                    "OLLAMA_HOST={host}: the handler did not read the constructed instance"
                );
            }
        }

        /// A base carrying conformance ERRORS makes the command exit NON-ZERO, so
        /// `biorouter knowledge lint` can gate a script or a CI job.
        ///
        /// **The printer this replaced was a confident false negative.** It knew
        /// only the four legacy hygiene lists — orphans, contradictions, stale
        /// sources, missing concept pages — and ignored `report.diagnostics`
        /// entirely, so a base whose every page fails OKF §11 rule 1 printed
        /// `0 orphan page(s) … ✓ clean` and exited 0. A user who ran the check
        /// and was told the base was fine has no reason to look again.
        ///
        /// The clean leg is what makes the dirty one mean something: an
        /// implementation that failed unconditionally would satisfy the dirty
        /// assertion alone.
        #[tokio::test]
        #[serial]
        async fn errors_fail_the_lint_command_and_a_clean_base_still_passes() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            for (page, expect_ok) in [
                // Conformant: OKF §4.1's one always-required key is present.
                ("---\ntype: Note\nidentifier: A\n---\n\nbody\n", true),
                // `okf.type.missing` — an ERROR, and invisible to the four
                // hygiene lists, which is the whole bug.
                ("---\nidentifier: A\n---\n\nbody\n", false),
            ] {
                let tmp = tempfile::TempDir::new().unwrap();
                let _env = crate::test_sandbox::relocate_path_root_and(
                    tmp.path(),
                    base_env("http://127.0.0.1:1"),
                );
                let root = cli_knowledge_root_with_base(&tmp, "k");
                biorouter::knowledge::store::write_page(
                    &root.join("k"),
                    "knowledge/note/a.md",
                    page,
                    "add a",
                    None,
                )
                .unwrap();

                let outcome = handle_lint(Some("k".into()), false, None, None).await;
                assert_eq!(
                    outcome.is_ok(),
                    expect_ok,
                    "page {page:?} should exit {}",
                    if expect_ok { "zero" } else { "non-zero" }
                );
                if let Err(e) = outcome {
                    let message = e.to_string();
                    assert!(
                        message.contains("conformance error"),
                        "the failure must say WHY the command failed: {message}"
                    );
                }
            }
        }

        /// `kb lint` with no `--fix`, BOTH directions — the path that answered
        /// with a hardcoded `Public` and so refused every caller.
        ///
        /// ⚠ The public leg alone proves nothing here: "a public model may not
        /// lint a private base" is satisfied by "nobody may", which is what the
        /// handler did. It is the PRIVATE leg that separates a barrier from an
        /// outage, and it is the leg the literal fails. Same name in both legs
        /// (`--provider ollama`), only `OLLAMA_HOST` moves, for the reason
        /// `the_cli_ingest_handler_…` states.
        ///
        /// A scan needs no LLM, so unlike the ingest row above this one asserts
        /// the handler's own `Result` rather than a side effect: nothing answers
        /// on either host, and nothing has to.
        #[tokio::test]
        #[serial]
        async fn the_cli_read_only_lint_admits_a_private_model_and_still_refuses_a_public_one() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            for (host, expect_ok) in [
                ("http://127.0.0.1:1", true),
                ("http://ollama.invalid:11434", false),
            ] {
                let tmp = tempfile::TempDir::new().unwrap();
                let _env = crate::test_sandbox::relocate_path_root_and(tmp.path(), base_env(host));
                let root = cli_knowledge_root_with_base(&tmp, "k");
                biorouter::knowledge::tier::raise_unlocked(&root, "k", true).unwrap();

                let outcome = handle_lint(
                    Some("k".into()),
                    /* fix */ false,
                    Some("ollama".into()),
                    Some("qwen3.5:4b".into()),
                )
                .await;

                if expect_ok {
                    outcome.expect("a private model was refused a read-only lint of its own base");
                } else {
                    let err = outcome
                        .expect_err("a public model linted a private base")
                        .to_string();
                    assert!(
                        err.contains("only a private model may read or write it")
                            && err.contains("switch this chat to a private model"),
                        "the refusal named neither the reason nor a usable remedy: {err}"
                    );
                }
            }
        }
    }
}
