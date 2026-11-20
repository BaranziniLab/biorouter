use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the binary crate sits inside the repository")
        .to_path_buf()
}

fn assert_suite_passes(script: &str) {
    let output = Command::new("sh")
        .arg(script)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("the provider suite at {script} runs: {error}"));
    assert!(
        output.status.success(),
        "{script} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn macos_provider_suite_passes() {
    assert_suite_passes("providers/macos/test-provider.sh");
}
