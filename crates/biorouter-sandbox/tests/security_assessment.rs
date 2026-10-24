//! Observational boundary probes for the frozen 2026-10 assessment.
use std::time::Instant;

use biorouter_sandbox::{DockerSandbox, LocalProcessSandbox, SandboxClient, SandboxSpec};
use serde_json::json;

fn record(id: &str, kind: &str, observed: serde_json::Value, protected: bool, elapsed: Instant) {
    let outcome = match (kind, protected) {
        ("control", true) => "allowed_control",
        ("control", false) => "blocked_control",
        (_, true) => "blocked",
        (_, false) => "attack_succeeded",
    };
    println!(
        "SECURITY_ASSESSMENT {}",
        json!({
            "case_id": id, "family": "C", "case_type": kind,
            "repetition": 1, "outcome": outcome, "observed": observed,
            "duration_ms": elapsed.elapsed().as_millis(),
            "configuration": "native production SandboxClient file API; fresh synthetic fixture only",
            "actor": "ordinary process user; synthetic workspace supplied to API",
            "limitation": "component boundary, not full agent dispatch or OS process containment"
        })
    );
}

#[tokio::test]
async fn synthetic_file_boundary_observations() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("work");
    let outside = fixture.path().join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("canary.txt"), b"SYNTHETIC_OUTSIDE_CANARY").unwrap();
    let local = LocalProcessSandbox::new(SandboxSpec::new(&workspace));

    let started = Instant::now();
    local
        .write("allowed.txt", b"SYNTHETIC_ALLOWED")
        .await
        .unwrap();
    let bytes = local.read("allowed.txt").await.unwrap();
    let listing = local.list(".").await.unwrap();
    let allowed = bytes == b"SYNTHETIC_ALLOWED" && listing.contains(&"allowed.txt".into());
    record(
        "TB-FS-C01",
        "control",
        json!({"read_matches":bytes == b"SYNTHETIC_ALLOWED", "listed":listing}),
        allowed,
        started,
    );

    let started = Instant::now();
    let result = local.read("../outside/canary.txt").await;
    record(
        "TB-FS-A01",
        "attack",
        json!({"rejected":result.is_err(), "outside_bytes_returned":result.as_ref().is_ok_and(|v| v == b"SYNTHETIC_OUTSIDE_CANARY")}),
        result.is_err(),
        started,
    );

    let started = Instant::now();
    let result = local
        .read(outside.join("canary.txt").to_str().unwrap())
        .await;
    record(
        "TB-FS-A02",
        "attack",
        json!({"rejected":result.is_err(), "outside_bytes_returned":result.as_ref().is_ok_and(|v| v == b"SYNTHETIC_OUTSIDE_CANARY")}),
        result.is_err(),
        started,
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(outside.join("canary.txt"), workspace.join("existing-link")).unwrap();
        let started = Instant::now();
        let result = local.read("existing-link").await;
        record(
            "TB-FS-A03",
            "attack",
            json!({"rejected":result.is_err(), "outside_bytes_returned":result.as_ref().is_ok_and(|v| v == b"SYNTHETIC_OUTSIDE_CANARY")}),
            result.is_err(),
            started,
        );

        let started = Instant::now();
        let result = local
            .write("existing-link", b"SYNTHETIC_ATTACK_WRITE")
            .await;
        let unchanged =
            std::fs::read(outside.join("canary.txt")).unwrap() == b"SYNTHETIC_OUTSIDE_CANARY";
        record(
            "TB-FS-A04",
            "attack",
            json!({"rejected":result.is_err(), "outside_unchanged":unchanged}),
            result.is_err() && unchanged,
            started,
        );

        symlink(&outside, workspace.join("directory-link")).unwrap();
        let started = Instant::now();
        let result = local
            .write("directory-link/new-file.txt", b"SYNTHETIC_ATTACK_WRITE")
            .await;
        let escaped = outside.join("new-file.txt").exists();
        record(
            "TB-FS-A05",
            "attack",
            json!({"rejected":result.is_err(), "outside_file_created":escaped}),
            result.is_err() && !escaped,
            started,
        );

        for (id, name, docker_api) in [
            ("TB-FS-A06", "local-dangling", false),
            ("TB-FS-A07", "docker-dangling", true),
        ] {
            let target = outside.join(format!("{name}.txt"));
            symlink(&target, workspace.join(name)).unwrap();
            let started = Instant::now();
            let result = if docker_api {
                DockerSandbox::new(SandboxSpec::new(&workspace))
                    .write(name, b"SYNTHETIC_ATTACK_WRITE")
                    .await
            } else {
                local.write(name, b"SYNTHETIC_ATTACK_WRITE").await
            };
            let escaped = std::fs::read(&target).is_ok_and(|v| v == b"SYNTHETIC_ATTACK_WRITE");
            record(
                id,
                "attack",
                json!({"backend":if docker_api {"DockerSandbox host-side file API"} else {"LocalProcessSandbox file API"}, "api_returned_success":result.is_ok(), "outside_canary_created":escaped}),
                !escaped,
                started,
            );
            assert!(
                result.is_err() && !escaped,
                "dangling symlink escaped the workspace jail"
            );
        }
    }
}

#[tokio::test]
#[ignore = "explicit opt-in: requires local Docker image; synthetic mounts only"]
async fn docker_runtime_boundary_observations() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    let image = std::env::var("BIOROUTER_ASSESSMENT_IMAGE")
        .expect("pin BIOROUTER_ASSESSMENT_IMAGE to the inspected local Alpine image ID");
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("work");
    let outside = fixture.path().join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::set_permissions(&workspace, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::write(outside.join("host-canary"), b"SYNTHETIC_HOST_CANARY").unwrap();
    let base = SandboxSpec::new(&workspace)
        .with_image(&image)
        .with_timeout(Duration::from_secs(8))
        .with_max_mem("64m")
        .with_cpus(0.5)
        .with_mount(biorouter_sandbox::Mount {
            host: outside.clone(),
            guest: "/readonly-fixture".into(),
            writable: false,
        });
    let sandbox = DockerSandbox::new(base.clone());
    assert!(sandbox.available().await, "Docker CLI unavailable");

    let started = Instant::now();
    let output = sandbox.exec(&["sh".into(),"-c".into(),"id -u; grep -E '^(CapEff|NoNewPrivs):' /proc/self/status; printf SYNTHETIC_ALLOWED > /work/allowed-runtime; cat /work/allowed-runtime".into()],None).await.unwrap();
    let allowed = output.success()
        && output.stdout.starts_with("65534\n")
        && output.stdout.contains("0000000000000000")
        && output.stdout.contains("NoNewPrivs:\t1")
        && std::fs::read(workspace.join("allowed-runtime"))
            .is_ok_and(|v| v == b"SYNTHETIC_ALLOWED");
    docker_record(
        "TB-DK-C01",
        "control",
        if allowed {
            "allowed_control"
        } else {
            "blocked_control"
        },
        json!({"stdout":output.stdout,"stderr":output.stderr,"exit_code":output.code,"workspace_marker_created":workspace.join("allowed-runtime").exists()}),
        started,
    );

    let started = Instant::now();
    let unmounted = fixture.path().join("unmounted-host-canary");
    std::fs::write(&unmounted, b"SYNTHETIC_UNMOUNTED_CANARY").unwrap();
    let output = sandbox
        .exec(&["cat".into(), unmounted.display().to_string()], None)
        .await
        .unwrap();
    let leaked = output.stdout.contains("SYNTHETIC_UNMOUNTED_CANARY");
    docker_record(
        "TB-DK-A01",
        "attack",
        if !leaked && output.code != 0 {
            "blocked"
        } else if leaked {
            "attack_succeeded"
        } else {
            "inconclusive"
        },
        json!({"exit_code":output.code,"stdout":output.stdout,"stderr":output.stderr,"unmounted_host_canary_returned":leaked}),
        started,
    );

    let started = Instant::now();
    let output = sandbox.exec(&["sh".into(),"-c".into(),"ls /sys/class/net; cat /proc/net/route; wget -T 1 -q -O - http://198.51.100.1:9; printf '\nCONNECT_EXIT=%s\n' \"$?\"".into()],None).await.unwrap();
    let interface_only_loopback =
        output.stdout.lines().next() == Some("lo") && !output.stdout.contains("eth0");
    docker_record(
        "TB-DK-A02",
        "attack",
        if interface_only_loopback {
            "blocked"
        } else {
            "inconclusive"
        },
        json!({"stdout":output.stdout,"stderr":output.stderr,"only_loopback_interface":interface_only_loopback,"limitation":"network namespace observation plus bounded TEST-NET connection attempt; no outside network endpoint used"}),
        started,
    );

    let started = Instant::now();
    let output = sandbox
        .exec(
            &[
                "sh".into(),
                "-c".into(),
                "printf SYNTHETIC_ATTACK_WRITE > /readonly-fixture/host-canary".into(),
            ],
            None,
        )
        .await
        .unwrap();
    let unchanged = std::fs::read(outside.join("host-canary")).unwrap() == b"SYNTHETIC_HOST_CANARY";
    docker_record(
        "TB-DK-A03",
        "attack",
        if unchanged && output.code != 0 {
            "blocked"
        } else if !unchanged {
            "attack_succeeded"
        } else {
            "inconclusive"
        },
        json!({"exit_code":output.code,"stderr":output.stderr,"readonly_host_canary_unchanged":unchanged}),
        started,
    );

    let started = Instant::now();
    let timeout = DockerSandbox::new(base.with_timeout(Duration::from_millis(700)));
    let marker = workspace.join("after-timeout");
    let output = timeout
        .exec(
            &[
                "sh".into(),
                "-c".into(),
                "sleep 2; printf SYNTHETIC_POST_TIMEOUT > /work/after-timeout; sleep 2".into(),
            ],
            None,
        )
        .await
        .unwrap();
    let listing = std::process::Command::new("docker")
        .args(["ps", "-q"])
        .output()
        .unwrap();
    let mut owned_containers = Vec::new();
    for id in String::from_utf8_lossy(&listing.stdout).lines() {
        let inspect = std::process::Command::new("docker")
            .args(["inspect", id])
            .output()
            .unwrap();
        let Ok(data) = serde_json::from_slice::<serde_json::Value>(&inspect.stdout) else {
            continue;
        };
        let owns_workspace = data[0]["Mounts"].as_array().is_some_and(|mounts| {
            mounts
                .iter()
                .any(|mount| mount["Source"].as_str() == workspace.to_str())
        });
        if owns_workspace {
            owned_containers.push(id.to_owned());
        }
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let escaped_timeout = std::fs::read(&marker).is_ok_and(|v| v == b"SYNTHETIC_POST_TIMEOUT");
    for id in &owned_containers {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", id])
            .output();
    }
    docker_record(
        "TB-DK-A04",
        "attack",
        if output.timed_out && escaped_timeout {
            "attack_succeeded"
        } else if output.timed_out {
            "blocked"
        } else {
            "inconclusive"
        },
        json!({"reported_timeout":output.timed_out,"marker_created_after_timeout":escaped_timeout,"test_owned_running_container_count_after_api_return":owned_containers.len(),"cleanup":"only containers whose bind mount source exactly matches this fresh synthetic workspace"}),
        started,
    );
}

fn docker_record(
    id: &str,
    kind: &str,
    outcome: &str,
    observed: serde_json::Value,
    started: Instant,
) {
    println!(
        "SECURITY_ASSESSMENT {}",
        json!({"case_id":id,"family":"E","case_type":kind,"repetition":1,"outcome":outcome,"observed":observed,"duration_ms":started.elapsed().as_millis(),"configuration":"actual DockerSandbox::exec; pinned local Alpine image; network none; uid65534; cap-drop ALL; no-new-privileges; memory64m; cpus0.5; synthetic mounts","actor":"non-root guest process","limitation":"leaf executor boundary, not full agent dispatch; host CLI controls test-owned containers"})
    );
}
