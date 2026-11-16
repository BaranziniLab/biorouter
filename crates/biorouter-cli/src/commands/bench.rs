use crate::session::build_session;
use crate::session::SessionBuilderConfig;
use crate::{logging, CliSession};
use anyhow::Context;
use async_trait::async_trait;
use biorouter::conversation::Conversation;
use biorouter::session::session_manager::Session;
use biorouter::session::{SessionManager, SessionType};
use biorouter_bench::bench_session::{BenchAgent, BenchBaseSession};
use biorouter_bench::eval_suites::ExtensionRequirements;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// allow session obj to be used in benchmarking
#[async_trait]
impl BenchBaseSession for CliSession {
    async fn headless(&mut self, message: String) -> anyhow::Result<()> {
        // A turn its supervision stopped still ran; the eval judges what it
        // produced rather than failing the run.
        crate::commands::supervision_stop::ran_to_an_end(self.headless(message).await).map(|_| ())
    }
    fn message_history(&self) -> Conversation {
        self.message_history()
    }
    fn get_total_token_usage(&self) -> anyhow::Result<Option<i32>> {
        // Since the trait requires sync but the session method is async,
        // we need to block on the async call
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_total_token_usage())
        })
    }

    async fn get_session(&self) -> anyhow::Result<Session> {
        self.get_session().await
    }
}
/// The chat a bench eval runs in, created before the session is built:
/// `build_session` opens the id it is given and creates nothing, so an id
/// invented by the eval runner named no chat and every run panicked "Session
/// not found" as it opened it. Hidden, so bench runs stay out of the chat
/// list; named after the eval and opened in the eval's directory, where its
/// files are. The eval copies the finished chat into that directory as
/// `session.json`.
async fn create_bench_session(
    manager: &SessionManager,
    working_dir: PathBuf,
    name: String,
) -> anyhow::Result<String> {
    let session = manager
        .create_session(working_dir, name, SessionType::Hidden)
        .await?;
    Ok(session.id)
}

pub async fn agent_generator(
    requirements: ExtensionRequirements,
    session_name: String,
) -> anyhow::Result<BenchAgent> {
    let working_dir =
        std::env::current_dir().context("Could not read the eval's working directory")?;
    let session_id = create_bench_session(&SessionManager::instance(), working_dir, session_name)
        .await
        .context("Could not create the chat the eval runs in")?;
    let base_session = build_session(SessionBuilderConfig {
        session_id: Some(session_id),
        resume: false,
        no_session: false,
        extensions: requirements.external,
        streamable_http_extensions: requirements.streamable_http,
        builtins: requirements.builtin,
        workflow: None,
        additional_system_prompt: None,
        provider: None,
        model: None,
        debug: false,
        max_tool_repetitions: None,
        interactive: false, // Benchmarking is non-interactive
        scheduled_job_id: None,
        max_turns: None,
        quiet: false,
        output_format: "text".to_string(),
        decisions: Default::default(),
    })
    .await;

    let bench_agent = BenchAgent::new(Box::new(base_session));

    let errors = Some(Arc::new(Mutex::new(bench_agent.get_errors().await)));
    logging::setup_logging(Some("bench"), errors).expect("Failed to initialize logging");

    Ok(bench_agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W1-06/W1-10: the eval runner used to invent an id (`core:example-<nanos>`)
    /// that no chat had, and `build_session`, which opens the id it is given and
    /// creates nothing, handed it to `CliSession::new`, which panicked "Session
    /// not found" on every bench run. The chat must exist before the session is
    /// built: a hidden one, so bench runs stay out of the chat list, named after
    /// the eval and opened in the eval's directory.
    #[tokio::test]
    async fn a_bench_agent_session_exists_before_the_session_is_built() {
        let store = tempfile::tempdir().unwrap();
        let eval_dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(store.path().to_path_buf());

        let id = create_bench_session(
            &manager,
            eval_dir.path().to_path_buf(),
            "core:example".to_string(),
        )
        .await
        .expect("the bench chat is created");

        let session = manager
            .get_session(&id, false)
            .await
            .expect("the id build_session is given names a chat that exists");
        assert_eq!(session.session_type, SessionType::Hidden);
        assert_eq!(session.name, "core:example");
        assert_eq!(session.working_dir, eval_dir.path());
    }
}
