use super::materialize;
use crate::sandbox::provider_spec::ProviderSpec;
use std::path::PathBuf;

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "agent-desktop-provider-files-{label}-{}",
            crate::refs::new_snapshot_id()
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const DEMO: ProviderSpec = ProviderSpec {
    name: "demo",
    files: &[("provider.sh", "echo one\n")],
};

const EDITED: ProviderSpec = ProviderSpec {
    name: "demo",
    files: &[("provider.sh", "echo two\n")],
};

#[test]
fn edited_contents_get_a_new_directory() {
    let scratch = Scratch::new("edited");
    let before = materialize(&scratch.0, &DEMO).unwrap();
    let after = materialize(&scratch.0, &EDITED).unwrap();

    assert_ne!(before, after);
    assert_eq!(
        std::fs::read_to_string(after.join("provider.sh")).unwrap(),
        "echo two\n"
    );
}
