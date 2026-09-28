//! Deliberate Crew refusals that carry their own code.
//!
//! A refusal the daemon means (a public model on a Private workspace, a model outside the
//! workspace's institution, a request that assumed the other privacy mode) used to be a bare
//! `anyhow` sentence, so every HTTP surface answered it as the generic `400
//! crew_request_refused` and every client had to match its words. A [`CrewRefusal`] keeps the
//! sentence as its `Display`, so an agent, a log and an older client still read the same
//! words, and adds the stable `code`, the HTTP status and any typed fields a client needs to
//! word it itself. The routes find it anywhere in an error's chain.
//!
//! The refusal itself never changes here: each constructor replaces an `ensure!` that refused
//! the same thing before.
use serde_json::Value;
use std::fmt;

/// `crew_credential_store_unavailable`: no keyring service answers on this computer (only a
/// Secret Service can be missing) and there is no encrypted Crew vault, so a Crew key cannot be
/// saved or read (W2-DMN-1). Its sentence names `credentials init` only for a profile that
/// holds no identity yet, the only kind that command accepts.
pub const CREDENTIAL_STORE_UNAVAILABLE: &str = "crew_credential_store_unavailable";
/// `crew_credential_store_refused`: the keyring is there and did not let Biorouter use a Crew
/// key (denied at its prompt, locked, or no session to ask in); allowing access or unlocking
/// it, then trying again, helps (W2-DMN-1).
pub const CREDENTIAL_STORE_REFUSED: &str = "crew_credential_store_refused";
/// `crew_institution_mismatch`: the model, the connection or the workspace belong to different
/// institutions (W2-DMN-9).
pub const INSTITUTION_MISMATCH: &str = "crew_institution_mismatch";
/// `crew_public_model_refused`: a public model asked to read Crew context that is Private,
/// restricted, institution-owned or from a private chat (W2-DMN-9).
pub const PUBLIC_MODEL_REFUSED: &str = "crew_public_model_refused";
/// `crew_channel_not_in_workspace`: a channel ID the workspace does not list at all (W2-DMN-9).
pub const CHANNEL_NOT_IN_WORKSPACE: &str = "crew_channel_not_in_workspace";
/// `crew_mode_mismatch`: the request required one privacy mode and the connection is in the
/// other; nothing was sent (W2-DMN-9).
pub const MODE_MISMATCH: &str = "crew_mode_mismatch";
/// `crew_model_fixed`: a Crew chat keeps the model its access was granted to (W2-DMN-10).
pub const MODEL_FIXED: &str = "crew_model_fixed";
/// `crew_reconnecting`: the bridge broke and Biorouter is dialling it again (W2-DMN-6).
pub const RECONNECTING: &str = "crew_reconnecting";
/// `crew_outcome_unknown`: the bridge was lost after the request was written, so whether the
/// workspace applied it is not known (W2-DMN-7).
pub const OUTCOME_UNKNOWN: &str = "crew_outcome_unknown";
/// `crew_not_sent`: the bridge was lost before anything was written; nothing reached the
/// workspace (W2-DMN-7).
pub const NOT_SENT: &str = "crew_not_sent";

/// The sentence a Crew chat's model switch is refused with, and the grant path's refusal of a
/// different model for a chat that already has access (W2-DMN-10).
pub const MODEL_FIXED_TEXT: &str =
    "This chat's model is fixed by its Crew access. Start a new chat to use another model.";

/// A deliberate Crew refusal: a stable `code`, the HTTP status the daemon answers it with, one
/// plain sentence, and typed fields beside them.
#[derive(Clone, Debug)]
pub struct CrewRefusal {
    code: &'static str,
    status: u16,
    message: String,
    fields: Vec<(&'static str, Value)>,
}

impl CrewRefusal {
    /// A refusal answered `400`.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            status: 400,
            message: message.into(),
            fields: Vec::new(),
        }
    }

    /// The same refusal answered with `status` instead.
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// A typed field beside `code` and `error`. `code` and `error` themselves are never
    /// replaced.
    pub fn with(mut self, key: &'static str, value: Value) -> Self {
        if key != "code" && key != "error" {
            self.fields.push((key, value));
        }
        self
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn http_status(&self) -> u16 {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn fields(&self) -> &[(&'static str, Value)] {
        &self.fields
    }

    /// The [`CrewRefusal`] `error` is, carries as a context, or has anywhere in its chain.
    /// anyhow's own downcast comes first: a refusal added with `.context(…)` sits in a link
    /// the chain cannot downcast to it.
    pub fn find(error: &anyhow::Error) -> Option<&CrewRefusal> {
        error.downcast_ref::<CrewRefusal>().or_else(|| {
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<CrewRefusal>())
        })
    }

    /// [`Self::MODE_MISMATCH`]: the request required `expected` and the connection is `actual`.
    pub fn mode_mismatch(actual: super::ClusterMode, expected: super::ClusterMode) -> Self {
        Self::new(
            MODE_MISMATCH,
            format!(
                "Your connection is {}, but this request required {}. Nothing was sent.",
                mode_word(actual),
                mode_word(expected)
            ),
        )
        .with("actual_mode", serde_json::json!(actual))
        .with("expected_mode", serde_json::json!(expected))
    }

    /// [`MODEL_FIXED`], answered `409`.
    pub(super) fn model_fixed() -> Self {
        Self::new(MODEL_FIXED, MODEL_FIXED_TEXT).status(409)
    }

    /// [`PUBLIC_MODEL_REFUSED`] with `sentence`.
    pub(super) fn public_model(sentence: impl Into<String>) -> Self {
        Self::new(PUBLIC_MODEL_REFUSED, sentence)
    }
}

/// `Private` or `Public`, as the manual writes a privacy mode.
pub(super) fn mode_word(mode: super::ClusterMode) -> &'static str {
    match mode {
        super::ClusterMode::Private => "Private",
        super::ClusterMode::Public => "Public",
    }
}

impl fmt::Display for CrewRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CrewRefusal {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::ClusterMode;

    #[test]
    fn a_refusal_reads_as_its_sentence_and_is_found_under_context() {
        let refusal = CrewRefusal::mode_mismatch(ClusterMode::Private, ClusterMode::Public);
        assert_eq!(
            refusal.to_string(),
            "Your connection is Private, but this request required Public. Nothing was sent."
        );
        let wrapped = anyhow::Error::new(refusal).context("while sending");
        let found = CrewRefusal::find(&wrapped).expect("found under a context");
        assert_eq!(found.code(), MODE_MISMATCH);
        assert_eq!(found.http_status(), 400);
        assert_eq!(
            found.fields(),
            &[
                ("actual_mode", serde_json::json!("private")),
                ("expected_mode", serde_json::json!("public"))
            ]
        );
        assert!(CrewRefusal::find(&anyhow::anyhow!("plain")).is_none());
    }

    #[test]
    fn code_and_error_are_never_replaced_by_a_field() {
        let refusal = CrewRefusal::new(NOT_SENT, "Nothing was sent.")
            .with("code", serde_json::json!("x"))
            .with("error", serde_json::json!("y"));
        assert!(refusal.fields().is_empty());
        assert_eq!(CrewRefusal::model_fixed().http_status(), 409);
    }
}
