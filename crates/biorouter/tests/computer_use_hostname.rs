//! Exercise the production consent scope without contacting a model or granting desktop access.

use biorouter::agents::types::SharedProvider;
use biorouter::privacy::ProviderTier;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::security::computer_use::ComputerUseConsent;
use std::sync::Arc;

struct Model;

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "hostname-regression-fixture"
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
        unreachable!("hostname regression must never contact a model")
    }
}

fn provider() -> SharedProvider {
    Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Model))))
}

#[tokio::test]
async fn polling_and_task_binding_keep_the_same_machine_and_challenge() {
    let provider = provider();
    let consent = Arc::new(ComputerUseConsent::default());
    let initial = consent.status("synthetic", &provider).await.unwrap();
    assert!(initial
        .target
        .ends_with(&format!(" ({})", std::env::consts::OS)));
    assert!(!initial.target.starts_with(" ("));

    let task = consent.task_guard();
    consent.bind_task("synthetic", &provider).await.unwrap();
    let bound = consent.status("synthetic", &provider).await.unwrap();
    assert_eq!(initial.target, bound.target);
    for _ in 0..64 {
        let status = consent.status("synthetic", &provider).await.unwrap();
        assert_eq!(status.target, bound.target);
        assert_eq!(status.challenge_id, bound.challenge_id);
        assert_eq!(status.state, "idle");
    }
    drop(task);
    assert_eq!(
        consent.status("synthetic", &provider).await.unwrap().target,
        initial.target
    );
}

#[cfg(windows)]
#[test]
#[ignore = "visible-console calibration child for the Windows process observer"]
fn windows_visible_console_fixture() {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    let directory = PathBuf::from(std::env::var_os("BIOROUTER_HOSTNAME_VISIBLE_CONTROL").unwrap());
    std::fs::write(directory.join("ready"), std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !directory.join("release").exists() {
        assert!(
            Instant::now() < deadline,
            "observer did not release visible control"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "consoleless child exercised by scripts/test-copilot-hostname-windows.ps1"]
async fn windows_hostname_fixture() {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::Console::{
        FreeConsole, GetConsoleProcessList, GetConsoleWindow,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    let directory = PathBuf::from(std::env::var_os("BIOROUTER_HOSTNAME_FIXTURE").unwrap());
    let role = std::env::var("BIOROUTER_HOSTNAME_FIXTURE_ROLE").unwrap();
    assert!(matches!(role.as_str(), "baseline" | "fixed"));
    let mut console_process = [0u32; 1];
    let initial_window = unsafe { GetConsoleWindow() };
    let initial_process_count = unsafe { GetConsoleProcessList(console_process.as_mut_ptr(), 1) };
    let initial_window_visible =
        !initial_window.is_null() && unsafe { IsWindowVisible(initial_window) != 0 };
    // A fallback detachment records a launcher problem without letting console
    // inheritance hide the dependency spawn. The observer requires initial zero.
    if initial_process_count > 0 {
        assert_ne!(
            unsafe { FreeConsole() },
            0,
            "cannot detach fixture console: {}",
            std::io::Error::last_os_error()
        );
    }
    assert!(unsafe { GetConsoleWindow() }.is_null());
    assert_eq!(
        unsafe { GetConsoleProcessList(console_process.as_mut_ptr(), 1) },
        0
    );
    let console = serde_json::json!({
        "initial_window_present": !initial_window.is_null(),
        "initial_window_visible": initial_window_visible,
        "initial_process_count": initial_process_count,
        "console_detached": true,
        "detach_action": if initial_process_count > 0 { "FreeConsole" } else { "already_detached" },
    });
    std::fs::write(
        directory.join("console-state.json"),
        serde_json::to_vec_pretty(&console).unwrap(),
    )
    .unwrap();
    std::fs::write(directory.join("ready"), std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !directory.join("go").exists() {
        assert!(Instant::now() < deadline, "observer did not start");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let provider = provider();
    let consent = Arc::new(ComputerUseConsent::default());
    let task = consent.task_guard();
    consent.bind_task("synthetic", &provider).await.unwrap();
    let first = consent.status("synthetic", &provider).await.unwrap();
    for _ in 0..6 {
        if role == "baseline" {
            // The positive control deliberately reproduces the dependency spawn
            // that first-party Command censuses cannot see (#461).
            #[allow(clippy::disallowed_methods)]
            let hostname = sys_info::hostname().expect("baseline hostname.exe must run");
            assert!(!hostname.is_empty());
        }
        let status = consent.status("synthetic", &provider).await.unwrap();
        assert_eq!(status.target, first.target);
        assert_eq!(status.challenge_id, first.challenge_id);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    drop(task);
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "role": role,
            "pid": std::process::id(),
            "target": first.target,
            "polls": 6,
            "interval_ms": 2000,
            "console": console,
        }))
        .unwrap(),
    )
    .unwrap();
}
