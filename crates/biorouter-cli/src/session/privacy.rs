//! Issue #56 Task 31 — the CLI's privacy surfaces (R10).
//!
//! Every repair affordance Phase 4 shipped before this is a GUI card, and the
//! CLI is not an optional surface: a user who never opens the desktop app must
//! still be able to see what tier a chat is on, understand why one refused to
//! start, and get out of it.
//!
//! Three things live here, and they are pure functions with the registry lookup
//! split off, so all of the wording is testable without a terminal:
//!
//! * [`tier_row`] — the `privacy` row of the session banner.
//! * [`terminal_refusal`] — the sentence a private chat gets when the provider
//!   resolved for it is public, whichever of the four precedence sources
//!   produced it, plus the repair.
//! * [`available_private_models`] — what the repair offers, read from the real
//!   provider registry so a fourth private provider appears with no edit here.
//!
//! ⚠ **The list is sorted, and the sort is load-bearing.** `providers()`
//! iterates a `HashMap`, so its order differs between runs; a refusal that named
//! whichever private model came out first would print a different "exact re-run
//! command" every time, which is worse than printing none.

use biorouter::privacy::{ProviderTier, SessionClassification};

/// The provider name this terminal would run a turn on, resolved the way
/// `session/builder.rs` resolves its `GlobalDefault` slot: `BIOROUTER_PROVIDER`
/// if the environment sets it, otherwise the configured default.
///
/// `None` for an install that has never been configured. That is not an error
/// here — a user who has not run `biorouter configure` has no capability, which
/// is exactly what the caller does with it.
pub fn configured_provider_name() -> Option<String> {
    biorouter::config::Config::global()
        .get_biorouter_provider()
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// **The CLI's capability**: the tier of the model this process would run.
///
/// ⚠ **The CLI had none at all, and that was the defect underneath four
/// others.** It loaded a provider when it started a chat and never resolved a
/// tier for anything else, so every request it made to the daemon was
/// indistinguishable from one made by a public-tier caller — which is why
/// `session send/watch/attach` were refused for a private chat even when the
/// terminal was configured for an institutional model. A capability is not
/// something a surface has by being a surface; it is a fact about the model
/// bound to it, and until something resolved that fact there was nothing for a
/// gate to compare.
///
/// ⚠ **The DECLARED tier, not an instance's.** `declared_provider_tier` reads
/// the registry, because building an instance would need the provider's
/// credentials just to answer a routing question, and because the daemon
/// resolves the same name through the same function on its side — two different
/// resolutions of one name is how the client and the daemon come to disagree
/// about what a request is allowed to do. The residual is inherited and
/// permissive in the same direction the rest of the feature already accepts (an
/// `ollama` re-pointed off the machine by `OLLAMA_HOST` declares Private and
/// resolves Public), and it is not the authority: every gate downstream reads
/// the instance.
///
/// An unconfigured install, an unknown provider name and a public model all give
/// [`ProviderTier::Public`] — the fail-safe side.
pub async fn caller_capability() -> ProviderTier {
    match configured_provider_name() {
        Some(name) => biorouter::workflow::privacy::declared_provider_tier(&name).await,
        None => ProviderTier::Public,
    }
}

/// A private model the user could move this chat onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateModel {
    /// The `--provider` value.
    pub name: String,
    /// What the settings grid calls it.
    pub display_name: String,
    /// Whether inference runs on this machine. Display only — it is what splits
    /// the private tier into "on this machine" and "inside the institution",
    /// and it is what [`sort_private_models`] orders on.
    pub runs_locally: bool,
}

/// Institutional first, then by name.
///
/// Both halves are deliberate. **Institutional first** because the chat being
/// repaired is usually private *because* it reached institutional data, and a
/// local model cannot pick that work back up — offering `llamacpp` as the first
/// answer to "my OMOP chat will not start" is a suggestion that does not work.
/// **Then by name**, because the first element becomes an exact command printed
/// to a user, and an unstable one is a command that is wrong half the time.
pub fn sort_private_models(models: &mut [PrivateModel]) {
    models.sort_by(|a, b| {
        a.runs_locally
            .cmp(&b.runs_locally)
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Every provider this install publishes as Private, sorted.
///
/// The registry's declared tier, not an instance's: nothing is bound here, and
/// building one per candidate would need every private provider's credentials
/// just to print a list. The consequence is stated where it matters — an
/// `ollama` re-pointed off the machine is listed, and Gate A refuses it on the
/// bind, which reads the instance.
pub async fn available_private_models() -> Vec<PrivateModel> {
    let mut models: Vec<PrivateModel> = biorouter::providers::providers()
        .await
        .into_iter()
        .filter(|(metadata, _)| metadata.tier == ProviderTier::Private)
        .map(|(metadata, _)| PrivateModel {
            name: metadata.name,
            display_name: metadata.display_name,
            runs_locally: metadata.runs_locally,
        })
        .collect();
    sort_private_models(&mut models);
    models
}

/// The `privacy` row of the session banner.
///
/// It says the tier and, for a private chat, the one consequence a user needs to
/// hold in their head: this chat will refuse a public model. The public row is
/// deliberately terse — the full DR-17 disclosure is printed by `biorouter
/// configure` when the provider is chosen, and repeating it on every session
/// start is how a disclosure becomes wallpaper.
pub fn tier_row(classification: SessionClassification) -> String {
    match classification {
        SessionClassification::Private => {
            // Same gloss fix as refusal.rs: a local model is private too.
            "Private: only a private model may run here".to_string()
        }
        SessionClassification::Public => "Public".to_string(),
    }
}

/// DR-33: a decision model, in the terminal's spelling of a name plus where it
/// runs (`joey-1 · Baseten`, `tev1:0.8b · this computer`).
pub fn decision_model_place(
    model: &str,
    host: &biorouter::privacy::decision_axes::HostLabel,
) -> String {
    use biorouter::privacy::decision_axes::HostLabel;
    let (name, place) = match host {
        HostLabel::Baseten => (
            crate::commands::decide::model_name("baseten_decision", model),
            "Baseten".to_string(),
        ),
        HostLabel::ThisComputer => (model.to_string(), "this computer".to_string()),
        HostLabel::OllamaAt(at) => (model.to_string(), format!("Ollama at {at}")),
        HostLabel::OllamaCloud => (model.to_string(), "Ollama cloud".to_string()),
    };
    crate::commands::decide::terminal_text(&format!("{name} · {place}"))
}

/// DR-33 (design §5): why a chat bound to a private model is public, when its
/// decision models made it so. The banner's second privacy line and the
/// terminal UI's header say it: `public because monitor "Done" uses joey-1 ·
/// Baseten`.
pub fn decision_reason(cause: &biorouter::privacy::decision_axes::LoweredBy) -> String {
    use biorouter::privacy::decision_axes::NameSource;
    let model = decision_model_place(&cause.model, &cause.host);
    let label = crate::commands::decide::terminal_text(&cause.label);
    match cause.source {
        NameSource::Default => format!("public because the default decision model is {model}"),
        NameSource::Monitor => format!("public because monitor \"{label}\" uses {model}"),
        NameSource::Gate => format!("public because the gate on phase \"{label}\" uses {model}"),
        NameSource::PhaseMonitor => {
            format!("public because subagent monitor \"{label}\" uses {model}")
        }
        NameSource::Parent => "public because it is part of a public chat's work".to_string(),
    }
}

/// Where the provider that is about to be refused came from.
///
/// `biorouter-cli/src/session/builder.rs` resolves it as `--provider` flag →
/// saved session provider → the workflow's `biorouter_provider` → global
/// default. Three of those four are things the user did not type just now, so
/// "why is this chat refusing to start" has four different answers and only one
/// of them is obvious.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSource {
    /// `--provider` on this very command line.
    CliFlag,
    /// The provider stored on the session row being resumed.
    SavedSession,
    /// `settings.biorouter_provider` in the workflow being run.
    Workflow,
    /// `BIOROUTER_PROVIDER` / the configured default.
    GlobalDefault,
}

impl ProviderSource {
    /// The clause that says where the model came from. Present tense, no blame,
    /// and different for each source — a refusal that says the same thing for
    /// all four is a refusal that explains none of them.
    fn because(self, provider: &str) -> String {
        match self {
            Self::CliFlag => format!("`--provider {provider}` names a public model"),
            Self::SavedSession => {
                format!("this chat is still set to `{provider}`, which is a public model")
            }
            Self::Workflow => format!("this workflow pins `{provider}`, which is a public model"),
            Self::GlobalDefault => {
                format!("your default model is `{provider}`, which is a public model")
            }
        }
    }
}

/// The whole terminal refusal: why, what is available, and the exact commands
/// that fix it.
///
/// §14.4: it names the session **id**, the provider and the tier. The id is the
/// argument to both printed commands, so it has to be here — and it is the one
/// piece of a session the user already typed. The chat's name, its working
/// directory and its contents are not, and no caller can supply them: the
/// signature has nowhere to put them.
///
/// An empty `private_models` is not an impossible state — a stripped install can
/// have none — and it must not print `--provider ` with nothing after it.
pub fn terminal_refusal(
    session_id: &str,
    provider: &str,
    source: ProviderSource,
    private_models: &[PrivateModel],
) -> String {
    format!(
        // Same gloss fix as refusal.rs and `tier_row` above: a local model is
        // private too, and the earlier pass corrected `tier_row` while leaving
        // this one, which is the longer of the two and the one that then prints
        // the repair block listing what is actually available.
        "This chat is private, so only a private model may run in it (one your institution hosts, \
         or one that runs on this machine), and {}. Nothing has been sent and the chat is \
         unchanged.\n{}",
        source.because(provider),
        repair_block(session_id, private_models)
    )
}

/// The half of [`terminal_refusal`] that says what to DO — split out so the
/// workflow module's own load-time sentence (which explains a *shared artefact*,
/// not a precedence slot, and is therefore worded there) can be followed by the
/// same repair rather than by a second copy of it.
pub fn repair_block(session_id: &str, private_models: &[PrivateModel]) -> String {
    let mut out = String::new();
    match private_models.first() {
        Some(first) => {
            let list = private_models
                .iter()
                .map(|m| format!("{} ({})", m.display_name, m.name))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("\nPrivate models on this install: {list}.\n"));
            out.push_str(&format!(
                "\nRe-run this chat on one of them:\n\n    biorouter session --resume --id \
                 {session_id} --provider {}\n",
                first.name
            ));
        }
        None => {
            out.push_str(
                "\nThis install publishes no private models. Configure one with `biorouter \
                 configure` before resuming this chat.\n",
            );
        }
    }
    out.push_str(&format!(
        "\nOr, if this chat's contents are not private after all, declassify it:\n\n    \
         biorouter session declassify {session_id}\n"
    ));
    out
}

/// Issue #56 — what the terminal says when a private chat is asked to export
/// itself from a session running a public model.
///
/// ⚠ **Not [`repair_block`], and the difference is the command.** That block's
/// repair is *re-run this chat on a private model* (`biorouter session --resume
/// --id … --provider …`), which is the right advice for a chat that will not
/// start and the wrong advice here: nobody is trying to resume anything, they
/// are trying to take a copy out. The command below is the one that actually
/// re-runs the thing they asked for.
///
/// §14.4: it names the tier and the models and nothing about the conversation —
/// the signature has nowhere to put a title, a working directory or a message.
///
/// An empty `private_models` is not an impossible state (a stripped install can
/// have none) and must not print `BIOROUTER_PROVIDER=` with nothing after it.
pub fn export_capability_refusal(session_id: &str, private_models: &[PrivateModel]) -> String {
    let mut out = String::from(
        "This chat is private, and this terminal is running a public model, so it may not \
         write a copy of the transcript to a file. Nothing was exported and the chat is \
         unchanged.\n",
    );
    match private_models.first() {
        Some(first) => {
            let list = private_models
                .iter()
                .map(|m| format!("{} ({})", m.display_name, m.name))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("\nPrivate models on this install: {list}.\n"));
            out.push_str(&format!(
                "\nRe-run the export from a terminal on one of them:\n\n    \
                 BIOROUTER_PROVIDER={} biorouter session export --id {session_id}\n",
                first.name
            ));
        }
        None => out.push_str(
            "\nThis install publishes no private models, so there is no terminal here that may \
             export this chat. Configure one with `biorouter configure`.\n",
        ),
    }
    out.push_str(&format!(
        "\nOr, if this chat's contents are not private after all, declassify it first:\n\n    \
         biorouter session declassify {session_id}\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::privacy::SessionClassification;

    #[test]
    fn a_decision_reason_names_the_cause_and_where_its_model_runs() {
        use biorouter::privacy::decision_axes::{HostLabel, LoweredBy, NameSource};
        let cause = |source, label: &str, model: &str, host| LoweredBy {
            source,
            label: label.to_string(),
            model: model.to_string(),
            host,
            others: 0,
        };
        assert_eq!(
            decision_reason(&cause(
                NameSource::Monitor,
                "Done",
                "joey-1",
                HostLabel::Baseten
            )),
            "public because monitor \"Done\" uses joey-1 · Baseten"
        );
        assert_eq!(
            decision_reason(&cause(
                NameSource::Default,
                "default",
                "joey-1",
                HostLabel::Baseten
            )),
            "public because the default decision model is joey-1 · Baseten"
        );
        assert_eq!(
            decision_reason(&cause(
                NameSource::Gate,
                "Review",
                "gpt-oss:20b",
                HostLabel::OllamaAt("10.0.0.5".into())
            )),
            "public because the gate on phase \"Review\" uses gpt-oss:20b · Ollama at 10.0.0.5"
        );
        assert_eq!(
            decision_reason(&cause(
                NameSource::Parent,
                "parent-id",
                "",
                HostLabel::Baseten
            )),
            "public because it is part of a public chat's work"
        );
    }

    fn versa() -> PrivateModel {
        PrivateModel {
            name: "versa_azure".to_string(),
            display_name: "Versa API Azure".to_string(),
            runs_locally: false,
        }
    }

    fn llamacpp() -> PrivateModel {
        PrivateModel {
            name: "llamacpp".to_string(),
            display_name: "Llama Server".to_string(),
            runs_locally: true,
        }
    }

    #[test]
    fn the_cli_prints_the_tier_and_the_exact_re_run_command() {
        // (a) the tier, printed at session start.
        assert!(tier_row(SessionClassification::Private).contains("Private"));
        assert!(tier_row(SessionClassification::Public).contains("Public"));

        // (b) the refusal names the available private models and the exact
        //     command that re-runs THIS chat on one of them.
        let refusal = terminal_refusal(
            "20260801_7",
            "anthropic",
            ProviderSource::GlobalDefault,
            &[versa(), llamacpp()],
        );
        assert!(
            refusal.contains("--provider versa_azure"),
            "the refusal must carry the exact re-run command: {refusal}"
        );
        assert!(
            refusal.contains("Versa"),
            "the refusal must list the available private models: {refusal}"
        );
        assert!(refusal.contains("Llama Server"), "{refusal}");
        assert!(
            refusal.contains("biorouter session declassify 20260801_7"),
            "the other way out is the escape hatch: {refusal}"
        );
    }

    /// The named provider is the institutional one, and it is chosen rather than
    /// happened upon: `providers()` iterates a `HashMap`, so an unsorted list
    /// would name a different model on every run and the "exact re-run command"
    /// would be a lie half the time.
    #[test]
    fn the_named_model_is_deterministic_and_institutional_first() {
        let mut a = vec![llamacpp(), versa()];
        sort_private_models(&mut a);
        let mut b = vec![versa(), llamacpp()];
        sort_private_models(&mut b);
        assert_eq!(a[0].name, "versa_azure");
        assert_eq!(b[0].name, "versa_azure");
    }

    /// §14.4: a refusal names the tier and the model and nothing about the
    /// conversation.
    #[test]
    fn the_refusal_carries_no_session_content() {
        let refusal = terminal_refusal(
            "20260801_7",
            "anthropic",
            ProviderSource::SavedSession,
            &[versa()],
        );
        for content in ["Patient MRN 4471 workup", "phi/cohort-3"] {
            assert!(!refusal.contains(content), "{refusal}");
        }
    }

    /// Each of the four precedence sources says where the public model came
    /// from, because "why is my private chat refusing to start" has four
    /// different answers and three of them are not the one the user expects.
    #[test]
    fn every_precedence_source_explains_itself() {
        let mut seen: Vec<String> = vec![];
        for source in [
            ProviderSource::CliFlag,
            ProviderSource::SavedSession,
            ProviderSource::Workflow,
            ProviderSource::GlobalDefault,
        ] {
            let text = terminal_refusal("20260801_7", "anthropic", source, &[versa()]);
            assert!(text.contains("anthropic"), "{text}");
            assert!(!seen.contains(&text), "two sources gave the same sentence");
            seen.push(text);
        }
        assert_eq!(seen.len(), 4);
    }

    /// A stripped install with no private provider at all must not print
    /// `--provider ` with nothing after it, which reads as a command to run.
    #[test]
    fn with_no_private_model_the_refusal_offers_no_empty_command() {
        // The REPAIR, not the whole refusal: `ProviderSource::CliFlag`'s own
        // clause legitimately quotes `--provider anthropic` back at the user,
        // and an assertion over the whole string cannot tell that apart from the
        // dangling command this is about.
        let repair = repair_block("20260801_7", &[]);
        assert!(!repair.contains("--provider"), "{repair}");
        assert!(repair.contains("biorouter configure"), "{repair}");
        // The escape hatch survives: it needs no private model to exist.
        assert!(
            repair.contains("biorouter session declassify 20260801_7"),
            "{repair}"
        );
        // …and with one, the command is complete.
        assert!(repair_block("20260801_7", &[versa()]).contains("--provider versa_azure"));
    }

    /// The export refusal prints the command that re-runs THE EXPORT, not the
    /// command that re-runs the chat.
    ///
    /// The distinction is the whole reason this copy is not `repair_block`: a
    /// user who was refused an export and is told to `--resume` the chat has
    /// been sent to do something they did not ask for.
    #[test]
    fn the_export_refusal_offers_the_export_command_and_the_escape_hatch() {
        let text = export_capability_refusal("20260801_7", &[versa(), llamacpp()]);
        assert!(
            text.contains(
                "BIOROUTER_PROVIDER=versa_azure biorouter session export --id 20260801_7"
            ),
            "{text}"
        );
        assert!(
            !text.contains("--resume"),
            "the export refusal sends the user to resume the chat instead: {text}"
        );
        assert!(text.contains("Versa"), "{text}");
        assert!(text.contains("Llama Server"), "{text}");
        assert!(
            text.contains("biorouter session declassify 20260801_7"),
            "{text}"
        );
        // …and it says what did NOT happen, which is what stops a user
        // believing the file was written anyway.
        assert!(text.contains("Nothing was exported"), "{text}");
    }

    /// A stripped install must not print `BIOROUTER_PROVIDER=` with nothing
    /// after it, which reads as a command to run.
    #[test]
    fn with_no_private_model_the_export_refusal_offers_no_empty_command() {
        let text = export_capability_refusal("20260801_7", &[]);
        assert!(!text.contains("BIOROUTER_PROVIDER="), "{text}");
        assert!(text.contains("biorouter configure"), "{text}");
        assert!(
            text.contains("biorouter session declassify 20260801_7"),
            "{text}"
        );
    }

    /// §14.4 again: a refusal names the tier and the model and nothing about the
    /// conversation.
    #[test]
    fn the_export_refusal_carries_no_session_content() {
        let text = export_capability_refusal("20260801_7", &[versa()]);
        for content in ["Patient MRN 4471 workup", "phi/cohort-3"] {
            assert!(!text.contains(content), "{text}");
        }
    }

    /// The list comes from the real registry, so a fourth private provider
    /// appears here with no edit in this file — and `versa_azure` really is in
    /// it, which is what keeps the fixture-driven tests above honest.
    #[tokio::test]
    async fn the_private_models_offered_come_from_the_registry() {
        let models = available_private_models().await;
        assert!(models.iter().any(|m| m.name == "versa_azure"), "{models:?}");
        assert!(models.iter().any(|m| m.name == "llamacpp"), "{models:?}");
        assert!(
            !models.iter().any(|m| m.name == "anthropic"),
            "a public model must never be offered as the repair: {models:?}"
        );
        assert_eq!(
            models[0].name, "versa_azure",
            "the registry list must arrive sorted, or the re-run command is nondeterministic"
        );
    }
}
