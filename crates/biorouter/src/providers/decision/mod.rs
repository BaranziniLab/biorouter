//! Decision-model adapters: the backends a [`crate::decision::DecisionService`]
//! sends questions to.
//!
//! Albert Lee's prototype in PR #469 (`agents/baseten_decisions.rs`,
//! `agents/exit_criteria.rs`) was the first working decision-model integration
//! in BioRouter, and the System One question builders, the answer-type check,
//! the slug catalog and the refusal to echo error bodies carry over from it.
//!
//! In order of preference, recorded on every decision:
//! 1. [`system_one`]: a decision model's own distribution, from Baseten or a
//!    local Ollama (0.35 and later).
//! 2. [`logprob`]: a chat model's first-token log-probabilities, over Ollama's
//!    `/api/chat` or an OpenAI-format endpoint.
//! 3. [`verbalized`]: a number a chat model writes, never calibrated.
//!
//! [`chat`] is the decision model bound as a chat's main model: the provider
//! the factory registers for `baseten_decision` and `ollama_decision`, and the
//! reply a decision chat gives instead of an LLM loop.
//!
//! [`install`] downloads a decision model into the Ollama on this computer
//! when a person asks (the install route and `biorouter models decision
//! pull`), so a private chat has one it may use. Nothing installs on its own.
//!
//! These live under `providers/` on purpose: the destination-key scan in
//! `providers/destination_keys.rs` reads every config key this tree reads, so
//! a new host key cannot be added here without deciding whether changing it
//! over HTTP needs the user-action proof.

pub mod catalog;
pub mod chat;
pub mod install;
pub mod logprob;
pub mod system_one;
pub mod verbalized;

pub use catalog::{ConfiguredDecisionModels, BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER};
pub use chat::{
    is_decision_chat_provider, BasetenDecisionChatProvider, DecisionChatProvider,
    OllamaDecisionChatProvider,
};
pub use install::{
    installer, DecisionInstallSnapshot, DecisionInstaller, InstallJob, InstallOffer,
    InstallRefusal, InstallRefusalKind, InstallState, InstallableDecisionModel, OllamaPlace,
    OllamaStatus, PullProgress, PullStage, DEFAULT_INSTALL_MODEL, INSTALLABLE_DECISION_MODELS,
};
pub use logprob::LogprobAdapter;
pub use system_one::SystemOneAdapter;
pub use verbalized::VerbalizedAdapter;

/// A request that never reached the model, as a person reads it: "joey-1 on
/// Baseten could not be reached because its server's certificate did not
/// match". No full stop, so it reads inside the sentences that quote it (a
/// monitor's notice, a gate's last error). `name` is the model as a person
/// knows it. The transport error itself names the host and the failure in a
/// library's words (a TLS error quotes the certificate's names), so it goes to
/// the log only, never into the sentence a surface shows.
pub(crate) fn unreachable_sentence(name: &str, error: &anyhow::Error) -> String {
    tracing::warn!(
        model = name,
        error = %error.root_cause(),
        "decision model could not be reached"
    );
    format!(
        "{name} could not be reached because {}",
        unreachable_reason(error)
    )
}

/// What kept a request from reaching its server, in a few plain words.
fn unreachable_reason(error: &anyhow::Error) -> &'static str {
    let timed_out = error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)
    });
    let text = error
        .chain()
        .map(|cause| cause.to_string().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    if text.contains("not valid for name") || text.contains("notvalidforname") {
        "its server's certificate did not match"
    } else if text.contains("certificate") {
        "its server's certificate could not be verified"
    } else if timed_out || text.contains("timed out") {
        "it did not answer in time"
    } else if text.contains("connection refused") {
        "nothing answered at its address"
    } else if text.contains("dns error")
        || text.contains("failed to lookup address")
        || text.contains("nodename nor servname")
        || text.contains("name or service not known")
    {
        "its address could not be found"
    } else {
        "the connection failed"
    }
}

#[cfg(test)]
mod unreachable_tests {
    use super::*;

    /// The sentence names the model and why in plain words, and never carries
    /// the library's text or a backtick.
    #[test]
    fn a_transport_failure_reads_as_a_plain_sentence() {
        let tls = anyhow::anyhow!(
            "invalid peer certificate: certificate not valid for name \"x.example\"; \
             certificate is only valid for DnsName(\"y.example\")"
        )
        .context("error sending request");
        let sentence = unreachable_sentence("joey-1 on Baseten", &tls);
        assert_eq!(
            sentence,
            "joey-1 on Baseten could not be reached because its server's certificate did not \
             match"
        );
        let refused = anyhow::anyhow!("tcp connect error: Connection refused (os error 61)");
        assert_eq!(
            unreachable_sentence("tev1:0.8b on Ollama", &refused),
            "tev1:0.8b on Ollama could not be reached because nothing answered at its address"
        );
        for sentence in [
            unreachable_sentence("joey-1 on Baseten", &tls),
            unreachable_sentence("m", &anyhow::anyhow!("something else broke")),
        ] {
            assert!(!sentence.contains('`'), "{sentence}");
            assert!(!sentence.contains("DnsName"), "{sentence}");
        }
    }
}
