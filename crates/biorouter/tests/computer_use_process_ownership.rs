//! Production consent boundary across real processes, without a desktop helper.
#![cfg(unix)]

use biorouter::agents::types::SharedProvider;
use biorouter::privacy::ProviderTier;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::security::computer_use::ComputerUseConsent;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

struct Model;

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }
    fn get_name(&self) -> &str {
        "process-ownership-fixture"
    }
    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("synthetic")
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }
    async fn complete_with_model(
        &self,
        _: &biorouter::model::ModelConfig,
        _: &str,
        _: &[biorouter::conversation::message::Message],
        _: &[rmcp::model::Tool],
    ) -> Result<
        (biorouter::conversation::message::Message, ProviderUsage),
        biorouter::providers::errors::ProviderError,
    > {
        unreachable!("ownership acceptance must never contact a provider")
    }
}

#[tokio::test]
#[ignore = "child entry point launched by the process ownership test"]
async fn ownership_child() {
    let root = std::path::PathBuf::from(std::env::var_os("CU_OWNERSHIP_ROOT").unwrap());
    let role = std::env::var("CU_OWNERSHIP_ROLE").unwrap();
    assert!(dirs::data_local_dir().unwrap().starts_with(&root));
    let provider: SharedProvider = Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Model))));
    let consent = Arc::new(ComputerUseConsent::default());
    let task = consent.task_guard();
    consent.bind_task(&role, &provider).await.unwrap();
    let cancel = CancellationToken::new();
    let mut pending = Box::pin(consent.permit(&role, &provider, &cancel));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    let status = consent.status(&role, &provider).await.unwrap();
    assert!(status.requested);
    assert_ne!(status.state, "active");
    let approval = consent
        .approve(&role, &status.challenge_id, &provider)
        .await;
    if role == "contender" {
        assert!(approval
            .unwrap_err()
            .to_string()
            .contains("busy in another BioRouter process"));
        std::fs::write(root.join("contender.done"), "blocked").unwrap();
        return;
    }
    approval.unwrap();
    let permit = pending.await.unwrap();
    let controller = permit.lock().await.unwrap();
    std::fs::write(root.join(format!("{role}.ready")), &permit.generation).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.join(format!("{role}.release")).exists() {
        assert!(Instant::now() < deadline, "parent did not release child");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(controller);
    drop(permit);
    drop(task);
    assert_ne!(
        consent.status(&role, &provider).await.unwrap().state,
        "active"
    );
    if role == "owner" {
        std::fs::write(root.join("owner.released"), "released").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !root.join("owner.exit").exists() {
            assert!(
                Instant::now() < deadline,
                "parent did not finish successor check"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(root: &Path, role: &str) -> Process {
    Process(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ownership_child", "--ignored", "--nocapture"])
            // The production library uses dirs::data_local_dir(), not the unit-test lock.
            .env("HOME", root)
            .env("XDG_DATA_HOME", root.join("data"))
            .env("CU_OWNERSHIP_ROOT", root)
            .env("CU_OWNERSHIP_ROLE", role)
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

fn await_file(root: &Path, name: &str, child: &mut Process) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join(name).exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before {name}"
        );
        assert!(Instant::now() < deadline, "timed out waiting for {name}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn await_success(child: &mut Process) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "child failed: {status}");
            return;
        }
        assert!(Instant::now() < deadline, "child failed to finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn controller_excludes_other_processes_and_recovers_after_release_and_death() {
    for abrupt in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut owner = spawn(root.path(), "owner");
        await_file(root.path(), "owner.ready", &mut owner);
        let mut contender = spawn(root.path(), "contender");
        await_file(root.path(), "contender.done", &mut contender);
        await_success(&mut contender);
        if abrupt {
            owner.0.kill().unwrap();
            owner.0.wait().unwrap();
        } else {
            std::fs::write(root.path().join("owner.release"), "release").unwrap();
            await_file(root.path(), "owner.released", &mut owner);
            assert!(owner.0.try_wait().unwrap().is_none());
        }
        let mut successor = spawn(root.path(), "successor");
        await_file(root.path(), "successor.ready", &mut successor);
        if !abrupt {
            assert!(owner.0.try_wait().unwrap().is_none());
        }
        assert_ne!(
            std::fs::read(root.path().join("owner.ready")).unwrap(),
            std::fs::read(root.path().join("successor.ready")).unwrap()
        );
        std::fs::write(root.path().join("successor.release"), "release").unwrap();
        await_success(&mut successor);
        if !abrupt {
            std::fs::write(root.path().join("owner.exit"), "exit").unwrap();
            await_success(&mut owner);
        }
    }
}
