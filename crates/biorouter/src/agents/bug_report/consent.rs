use sha2::{Digest, Sha256};

use crate::pending_user_action::UserActionOutcome;
use crate::permission::Permission;

use super::issue::{publication_destination, DEFAULT_REPO, LABEL};

pub(super) fn report_digest(repo: &str, title: &str, body: &str) -> String {
    let mut hash = Sha256::new();
    let destination = publication_destination(repo);
    for field in [
        "biorouter-public-bug-report-v2",
        destination.as_str(),
        title,
        body,
        LABEL,
    ] {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

struct AuditRecord {
    attempt_id: uuid::Uuid,
    repository: String,
    digest: String,
}

impl AuditRecord {
    fn record(&self, decision: &'static str) {
        tracing::info!(
            target: "biorouter::security_audit",
            event = "bug_report_publication",
            attempt_id = %self.attempt_id,
            repository = %self.repository,
            report_sha256 = %self.digest,
            occurred_at = %chrono::Utc::now().to_rfc3339(),
            decision,
            "bug report publication consent"
        );
    }
}

pub(super) struct PublicationReview {
    audit: AuditRecord,
}

impl PublicationReview {
    pub(super) fn new(
        repo: &str,
        title: &str,
        body: &str,
        private: bool,
    ) -> Result<Self, &'static str> {
        if private && repo != DEFAULT_REPO {
            return Err("A report from a private chat can only be published to the fixed Biorouter issue tracker. Nothing was filed.");
        }
        let review = Self {
            audit: AuditRecord {
                attempt_id: uuid::Uuid::new_v4(),
                repository: publication_destination(repo),
                digest: report_digest(repo, title, body),
            },
        };
        review.audit.record("requested");
        Ok(review)
    }

    // Only the session-scoped, requires_user_proof card may supply this outcome.
    pub(super) fn approve(
        self,
        outcome: &UserActionOutcome,
    ) -> Result<PublicationGrant, &'static str> {
        let decision = match outcome {
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce,
            } => {
                self.audit.record("approved_once");
                return Ok(PublicationGrant { audit: self.audit });
            }
            UserActionOutcome::Approved { .. } => "rejected_persistent_permission",
            UserActionOutcome::Denied { .. } => "denied",
            UserActionOutcome::Cancelled => "cancelled",
            UserActionOutcome::TimedOut => "timed_out",
            _ => "failed",
        };
        self.audit.record(decision);
        Err(match outcome {
            UserActionOutcome::Approved { .. } => "must be a one-time Allow once decision",
            _ => outcome.refusal_detail(),
        })
    }
}

pub(super) struct PublicationGrant {
    audit: AuditRecord,
}

impl PublicationGrant {
    pub(super) fn refuse(self, reason: &'static str) {
        self.audit.record(reason);
    }

    pub(super) fn begin(
        self,
        repo: &str,
        title: &str,
        body: &str,
    ) -> Result<PublicationAttempt, &'static str> {
        if self.audit.repository != publication_destination(repo)
            || self.audit.digest != report_digest(repo, title, body)
        {
            self.audit.record("refused_changed_report");
            return Err("The report or destination changed after approval. Nothing was filed; review a new report before publishing.");
        }
        self.audit.record("submission_attempt");
        Ok(PublicationAttempt {
            audit: self.audit,
            finished: false,
        })
    }
}

pub(super) enum PublicationResult {
    Published,
    BrowserOpened,
    BrowserUnavailable,
    ReviewedReportSaved,
    ReviewedReportReturned,
    Failed,
    OutcomeUnknown,
}

pub(super) enum PublicationFailure {
    GitHubCli,
    LocalStaging,
}

pub(super) struct PublicationAttempt {
    audit: AuditRecord,
    finished: bool,
}

impl PublicationAttempt {
    pub(super) fn finish_browser(self, opened: bool) {
        self.finish(if opened {
            PublicationResult::BrowserOpened
        } else {
            PublicationResult::BrowserUnavailable
        });
    }

    pub(super) fn failure(&self, failure: PublicationFailure) {
        self.audit.record(match failure {
            PublicationFailure::GitHubCli => "github_cli_failed",
            PublicationFailure::LocalStaging => "local_staging_failed",
        });
    }

    pub(super) fn finish(mut self, result: PublicationResult) {
        self.audit.record(match result {
            PublicationResult::Published => "published",
            PublicationResult::BrowserOpened => "compose_page_opened",
            PublicationResult::BrowserUnavailable => "compose_page_unavailable",
            PublicationResult::ReviewedReportSaved => "reviewed_report_saved",
            PublicationResult::ReviewedReportReturned => "reviewed_report_returned",
            PublicationResult::Failed => "submission_failed",
            PublicationResult::OutcomeUnknown => "submission_outcome_unknown",
        });
        self.finished = true;
    }
}

impl Drop for PublicationAttempt {
    fn drop(&mut self) {
        if !self.finished {
            self.audit.record("cancelled_during_submission");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved() -> UserActionOutcome {
        UserActionOutcome::Approved {
            permission: Permission::AllowOnce,
        }
    }

    #[test]
    fn a_private_report_requires_the_fixed_destination() {
        assert!(
            PublicationReview::new("example/other", "fixture title", "fixture body", true).is_err()
        );
        assert!(
            PublicationReview::new(DEFAULT_REPO, "fixture title", "fixture body", true).is_ok()
        );
    }

    #[test]
    fn consent_is_bound_to_the_exact_repository_title_and_body() {
        assert_eq!(
            report_digest(DEFAULT_REPO, "fixture title", "fixture body"),
            "ca32ac5a8ab4c670ea0b31f22d7c2156d37cf609feaa07a1f1066f1804b00e6d"
        );
        for (repo, title, body) in [
            ("example/other", "fixture title", "fixture body"),
            (DEFAULT_REPO, "changed title", "fixture body"),
            (DEFAULT_REPO, "fixture title", "changed body"),
        ] {
            let grant = PublicationReview::new(DEFAULT_REPO, "fixture title", "fixture body", true)
                .unwrap()
                .approve(&approved())
                .unwrap();
            assert!(grant.begin(repo, title, body).is_err());
        }
        assert_ne!(
            report_digest(DEFAULT_REPO, "ab", "c"),
            report_digest(DEFAULT_REPO, "a", "bc")
        );
        let grant = PublicationReview::new(DEFAULT_REPO, "fixture title", "fixture body", true)
            .unwrap()
            .approve(&approved())
            .unwrap();
        assert!(grant
            .begin(DEFAULT_REPO, "fixture title", "fixture body")
            .is_ok());
    }

    #[test]
    fn persistent_permission_and_nonapproval_outcomes_cannot_publish() {
        for outcome in [
            UserActionOutcome::Approved {
                permission: Permission::AlwaysAllow,
            },
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            UserActionOutcome::Cancelled,
            UserActionOutcome::TimedOut,
        ] {
            assert!(
                PublicationReview::new(DEFAULT_REPO, "fixture title", "fixture body", true)
                    .unwrap()
                    .approve(&outcome)
                    .is_err()
            );
        }
    }

    #[test]
    fn security_audit_records_only_the_digest_destination_time_and_decision() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().write(bytes)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = Buffer(Arc::clone(&bytes));
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter("warn,biorouter=debug,biorouter_server=info")
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            PublicationReview::new(
                DEFAULT_REPO,
                "unpublished fixture title",
                "private fixture body",
                true,
            )
            .unwrap()
            .approve(&approved())
            .unwrap()
            .begin(
                DEFAULT_REPO,
                "unpublished fixture title",
                "private fixture body",
            )
            .unwrap()
            .finish(PublicationResult::Published);

            let result: anyhow::Result<()> = Err(anyhow::anyhow!("sensitive fixture stderr"));
            let attempt = PublicationReview::new(DEFAULT_REPO, "fixture", "fixture", true)
                .unwrap()
                .approve(&approved())
                .unwrap()
                .begin(DEFAULT_REPO, "fixture", "fixture")
                .unwrap();
            if result.is_err() {
                attempt.failure(PublicationFailure::GitHubCli);
                attempt.finish(PublicationResult::Failed);
            }
            drop(
                PublicationReview::new(DEFAULT_REPO, "fixture", "fixture", true)
                    .unwrap()
                    .approve(&approved())
                    .unwrap()
                    .begin(DEFAULT_REPO, "fixture", "fixture")
                    .unwrap(),
            );
            PublicationReview::new(DEFAULT_REPO, "fixture", "fixture", true)
                .unwrap()
                .approve(&approved())
                .unwrap()
                .begin(DEFAULT_REPO, "fixture", "fixture")
                .unwrap()
                .finish(PublicationResult::OutcomeUnknown);
        });
        let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        for expected in [
            "security_audit",
            "report_sha256",
            DEFAULT_REPO,
            "occurred_at",
            "attempt_id",
            "approved_once",
            "submission_attempt",
            "published",
            "github_cli_failed",
            "submission_failed",
            "cancelled_during_submission",
            "submission_outcome_unknown",
        ] {
            assert!(logs.contains(expected), "missing {expected}: {logs}");
        }
        assert!(!logs.contains("unpublished fixture title"));
        assert!(!logs.contains("private fixture body"));
        assert!(!logs.contains("sensitive fixture stderr"));
    }
}
