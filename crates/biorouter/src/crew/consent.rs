//! A chat's consent to read a workspace as the person (account ownership AO-12).
//!
//! The chat tools `crew__read` and `crew__change` act as the person. The first read in a chat
//! that returns a connection's workspace content asks the person once ("Let this chat read
//! baranzini-lab as you"); until they allow it, reads answer `crew_read_consent_required`.
//! The consent is kept here, per chat and connection, in the saved registry and written through
//! [`CrewManager::update_registry`] like every other registry change, listed beside the chat's
//! grants and revoked from there.
//!
//! Each consent is bound to the chat it was given in, by the incarnation of the session row
//! that held the id then (SCOPE-BIND), so a later chat under the same id never inherits it. A
//! consent whose chat cannot be confirmed, whose connection was removed, or that the saved
//! registry cannot vouch for is not held: every check fails closed.
//!
//! ⚠ **Person authority; needs human review.** A consent lets a chat read workspace content
//! as the person. It is never authority to change anything: every change still parks its own
//! approval.

use super::{CrewManager, GRANT_UNCONFIRMED, UNSAVED_CHAT};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One chat's consent to read one connection's workspace as the person.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct PersonRead {
    pub connection_id: String,
    /// The incarnation of the chat the consent was given in (SCOPE-BIND).
    pub session_incarnation: i64,
    /// When the person gave it, in seconds since the Unix epoch.
    pub granted_at: u64,
}

/// A consent as the grants list shows it: the chat it was given in, and the consent.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct PersonReadRow {
    pub session_id: String,
    #[serde(flatten)]
    pub consent: PersonRead,
}

impl CrewManager {
    /// Record that the person let the chat `session` read the workspace of connection
    /// `connection_id` as them (AO-12). The caller has the person's proof: the chat tools park
    /// a proof-backed approval first. Bound to the chat holding `session` now; refused for a
    /// chat that is not saved on this device, one whose identity cannot be read, and a
    /// connection that is not saved. Giving it again only renews it.
    pub async fn grant_person_read(&self, session: &str, connection_id: &str) -> Result<()> {
        let incarnation = match self.chat_incarnation(session).await {
            Ok(Some(incarnation)) => incarnation,
            Ok(None) => return Err(anyhow::anyhow!(UNSAVED_CHAT)),
            Err(error) => return Err(error.context(GRANT_UNCONFIRMED)),
        };
        let granted_at = super::revocation::unix_now();
        self.update_registry(|r| {
            r.connections
                .iter()
                .any(|c| c.id == connection_id)
                .then_some(())
                .context("Unknown Crew connection")?;
            let consents = r.person_reads.entry(session.to_owned()).or_default();
            // One consent per chat and connection: a chat that was another chat under the same
            // id holds none here any more.
            consents.retain(|consent| {
                consent.connection_id != connection_id && consent.session_incarnation == incarnation
            });
            consents.push(PersonRead {
                connection_id: connection_id.to_owned(),
                session_incarnation: incarnation,
                granted_at,
            });
            Ok(())
        })
        .await
    }

    /// Whether the person let the chat `session` read the workspace of connection
    /// `connection_id` as them (AO-12): a consent given in the chat holding `session` now, for
    /// a connection still saved. `false` whenever that cannot be confirmed: the saved registry
    /// cannot be read, the chat's identity cannot be read, or the consent was given in another
    /// chat under the same id.
    pub async fn has_person_read(&self, session: &str, connection_id: &str) -> bool {
        self.person_reads(session)
            .await
            .iter()
            .any(|consent| consent.connection_id == connection_id)
    }

    /// Every consent the chat `session` holds (AO-12), for `grants_list` and
    /// `biorouter crew grants list`: those given in the chat holding `session` now, for
    /// connections still saved. Empty whenever that cannot be confirmed, as
    /// [`Self::has_person_read`] reads it.
    pub async fn person_reads(&self, session: &str) -> Vec<PersonRead> {
        self.refresh_registry().await;
        if self.is_unreadable() {
            return Vec::new();
        }
        let recorded: Vec<PersonRead> = {
            let registry = self.registry.lock().await;
            registry
                .person_reads
                .get(session)
                .map(|consents| {
                    consents
                        .iter()
                        .filter(|consent| {
                            registry
                                .connections
                                .iter()
                                .any(|c| c.id == consent.connection_id)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        if recorded.is_empty() {
            return recorded;
        }
        match self.chat_incarnation(session).await {
            Ok(Some(current)) => recorded
                .into_iter()
                .filter(|consent| consent.session_incarnation == current)
                .collect(),
            Ok(None) => Vec::new(),
            Err(error) => {
                tracing::debug!(session, %error, "Couldn't confirm which chat holds a Crew read consent; treating it as not given");
                Vec::new()
            }
        }
    }

    /// Every chat's recorded consents, for the CLI's and the desktop's grants list, which
    /// filter them to the chats the caller may list (`lists_session`). The chat each was given
    /// in is not confirmed here; [`Self::has_person_read`] is what decides a read.
    pub async fn person_read_rows(&self) -> Vec<PersonReadRow> {
        self.refresh_registry().await;
        if self.is_unreadable() {
            return Vec::new();
        }
        let registry = self.registry.lock().await;
        let mut rows: Vec<PersonReadRow> = registry
            .person_reads
            .iter()
            .flat_map(|(session, consents)| {
                consents.iter().map(|consent| PersonReadRow {
                    session_id: session.clone(),
                    consent: consent.clone(),
                })
            })
            .filter(|row| {
                registry
                    .connections
                    .iter()
                    .any(|c| c.id == row.consent.connection_id)
            })
            .collect();
        rows.sort_by(|a, b| {
            (&a.session_id, &a.consent.connection_id)
                .cmp(&(&b.session_id, &b.consent.connection_id))
        });
        rows
    }

    /// Take back the chat `session`'s consent to read connection `connection_id`'s workspace
    /// as the person (AO-12). Answers whether there was one. Like every stop, it holds in this
    /// process even when the saved registry cannot be written; the save's error is returned
    /// then, so the person is told to try again.
    pub async fn revoke_person_read(&self, session: &str, connection_id: &str) -> Result<bool> {
        self.update_registry_keeping(|r| {
            let Some(consents) = r.person_reads.get_mut(session) else {
                return Ok(false);
            };
            let before = consents.len();
            consents.retain(|consent| consent.connection_id != connection_id);
            let revoked = consents.len() != before;
            if consents.is_empty() {
                r.person_reads.remove(session);
            }
            Ok(revoked)
        })
        .await?
    }
}
