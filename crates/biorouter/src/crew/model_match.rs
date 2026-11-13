//! Whether a model may read a Crew place, decided by privacy alone (design CP-1, CP-2 in
//! `docs/crew/design/coding-agent-parity.md`).
//!
//! Every Crew door that admits a model asks [`check`]: a chat's grant at each tool call and
//! model request, a binding, a run's admission, the workspace's answer to it and the run's
//! creation. It takes a [`ModelPrivacy`] (a tier and an affiliation), never a provider, so no
//! door can ask what kind of provider a model is. Claude Code and Codex are public models like
//! any other: a public workspace admits them and a private one refuses them, with the sentence
//! any public model reads. `no_crew_door_asks_the_provider_kind` (below) fails on a Crew file
//! that asks the provider's kind or compares a tier with Public outside this module.
//!
//! ⚠ **Crew admission; a change here needs human review.** Each sentence is the one a person
//! reads when Crew refuses a model, and `docs/crew/agents-and-chat-access.md` quotes them.

use super::institution;
use super::refusal::CrewRefusal;
use super::{ClusterMode, Connection, Scope};
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::{CallCapability, ProviderTier};
use crate::providers::base::Provider;
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;

/// What a public model is told when the chat asking for Crew access has used a private one: its
/// history carries private context a public model must not continue with (W2-DMN-9, the run's
/// `origin_restricted`).
pub(super) const ORIGIN_RESTRICTED_PUBLIC: &str = "This chat has used a private model, so a public model can't continue it with Crew context. Choose a private model.";

/// Said to a public model asked to read a chat's standing grant that does not admit one.
const GRANT_PRIVATE_TO_A_PUBLIC_MODEL: &str =
    "This chat's Crew context is private, so a public model can't read it.";

/// What a Crew door knows of a model's privacy: its tier and its affiliation, read together.
///
/// For a chat this is its DR-33 privacy: the provider slot folds the chat's decision models
/// in (`DecisionBoundProvider`), so a chat that one of them made public is a public model
/// here, whatever its own model is. A tool call's capability is sampled from that same slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelPrivacy {
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

impl ModelPrivacy {
    pub fn new(tier: ProviderTier, affiliation: Option<ModelAffiliation>) -> Self {
        Self { tier, affiliation }
    }

    /// A provider's privacy as it is bound: a chat's slot answers its DR-33 tier and
    /// affiliation, a Crew task's fresh provider its own.
    pub fn of(provider: &dyn Provider) -> Self {
        Self::new(provider.tier(), provider.affiliation())
    }

    /// A tool call's privacy: the capability it was admitted on.
    pub fn of_call(capability: &CallCapability) -> Self {
        Self::new(capability.tier(), capability.affiliation())
    }

    pub fn tier(&self) -> ProviderTier {
        self.tier
    }

    pub fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }

    /// Whether this is a public model: one hosted outside the person's institution, such as
    /// Claude Code, Codex or a commercial API, or a chat a public decision model made public.
    pub fn is_public(&self) -> bool {
        self.tier == ProviderTier::Public
    }
}

/// The Crew place a model would read, as the door asking knows it.
pub(super) enum CrewPlace<'a> {
    /// A chat's standing grant. Whether it admits a public model was decided when the workspace
    /// admitted it, and recorded in it (`public_provider`, `origin_restricted`), with the
    /// institutions that own what it reads.
    Grant {
        scope: &'a Scope,
        connection: &'a Connection,
    },
    /// The connection a run is asked on, before the workspace is asked: its mode, and whether
    /// the chat the run is for (or asked from) is already private.
    Connection {
        connection: &'a Connection,
        workspace: &'a str,
        origin_restricted: bool,
    },
    /// The workspace as its snapshot answers, for a run: its mode, and whether what the run
    /// would read is restricted (a private connection or workspace, a private origin,
    /// restricted channels, or a work folder on the server).
    Workspace {
        label: &'a str,
        mode: ClusterMode,
        restricted: bool,
    },
    /// What a run reads belongs to these institutions.
    Owned {
        institution_ids: &'a BTreeSet<String>,
    },
}

/// `Ok` when `model` may read `place`, else the refusal the person reads: a public model where
/// the place is private or restricted (`crew_public_model_refused`), or a private model whose
/// affiliation does not cover the institutions that own it (`crew_institution_mismatch`).
///
/// Never the provider's kind: a door holds only the model's privacy here.
pub(super) fn check(model: &ModelPrivacy, place: CrewPlace<'_>) -> Result<()> {
    match place {
        CrewPlace::Grant { scope, connection } => {
            if model.is_public()
                && !(connection.mode == ClusterMode::Public
                    && scope.public_provider
                    && !scope.origin_restricted)
            {
                return Err(CrewRefusal::public_model(GRANT_PRIVATE_TO_A_PUBLIC_MODEL).into());
            }
            owned(model, &scope.institution_ids)
        }
        CrewPlace::Connection {
            connection,
            workspace,
            origin_restricted,
        } => {
            if !model.is_public() {
                return Ok(());
            }
            if connection.mode != ClusterMode::Public {
                return Err(CrewRefusal::public_model(format!(
                    "Your connection to {workspace} is Private, so a public model can't read \
                     it. Choose a private model."
                ))
                .with("workspace", json!(workspace))
                .into());
            }
            if origin_restricted {
                return Err(CrewRefusal::public_model(ORIGIN_RESTRICTED_PUBLIC).into());
            }
            Ok(())
        }
        CrewPlace::Workspace {
            label,
            mode,
            restricted,
        } => {
            if !model.is_public() {
                return Ok(());
            }
            if mode != ClusterMode::Public {
                return Err(CrewRefusal::public_model(format!(
                    "{label} is Private, so a public model can't read it. Choose a private model."
                ))
                .with("workspace", json!(label))
                .into());
            }
            if restricted {
                return Err(CrewRefusal::public_model(format!(
                    "Restricted channels in {label} can't be read by a public model. Choose a \
                     private model."
                ))
                .with("workspace", json!(label))
                .into());
            }
            Ok(())
        }
        CrewPlace::Owned { institution_ids } => owned(model, institution_ids),
    }
}

fn owned(model: &ModelPrivacy, institution_ids: &BTreeSet<String>) -> Result<()> {
    institution::owned_by(model, institution_ids)
}

#[cfg(test)]
#[path = "model_match_tests.rs"]
mod tests;
