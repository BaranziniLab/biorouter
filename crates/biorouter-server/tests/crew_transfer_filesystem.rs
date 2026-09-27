#![cfg(unix)]

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use biorouter_server::crew::local_files::{
    self, open_directory, protected_directory, select, validate_file_acl, Direction, Selection,
};
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::Path;

fn private_directory(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn private_root() -> tempfile::TempDir {
    let base = fs::canonicalize(std::env::temp_dir()).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
fn directory_selection_rejects_symlink_components_and_unsafe_output_modes() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let opened = open_directory(&output, false).unwrap();
    protected_directory(&opened).unwrap();

    fs::set_permissions(&output, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(protected_directory(&open_directory(&output, false).unwrap()).is_err());
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();

    let target = private_directory(root.path(), "target");
    let link = root.path().join("linked-output");
    symlink(&target, &link).unwrap();
    assert!(open_directory(&link, false).is_err());
}

#[test]
fn source_and_destination_selection_refuse_symlinks_and_nonregular_targets() {
    let root = private_root();
    let directory = private_directory(root.path(), "files");
    let source = directory.join("source.csv");
    fs::write(&source, b"a,b\n1,2\n").unwrap();

    let selected = select(&source, Direction::Upload, false).unwrap();
    assert!(matches!(selected, Selection::Source { .. }));
    assert_eq!(selected.direction(), Direction::Upload);
    assert_eq!(selected.name(), "source.csv");
    assert_eq!(selected.size(), Some(8));

    let source_link = directory.join("source-link.csv");
    symlink(&source, &source_link).unwrap();
    assert!(select(&source_link, Direction::Upload, false).is_err());

    let destination = directory.join("new.bin");
    let selected = select(&destination, Direction::Download, false).unwrap();
    assert!(matches!(selected, Selection::Destination { .. }));

    let existing = directory.join("existing.bin");
    fs::write(&existing, b"old").unwrap();
    assert!(select(&existing, Direction::Download, false).is_err());
    assert!(select(&existing, Direction::Download, true).is_ok());

    let existing_link = directory.join("existing-link.bin");
    symlink(&existing, &existing_link).unwrap();
    assert!(select(&existing_link, Direction::Download, true).is_err());

    let existing_directory = directory.join("existing-directory");
    fs::create_dir(&existing_directory).unwrap();
    assert!(select(&existing_directory, Direction::Download, true).is_err());
}

#[test]
fn publication_is_atomic_and_leaves_one_private_destination_link() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let directory = open_directory(&output, false).unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let part = output.join(local_files::part_name(id));
    fs::write(&part, b"first payload").unwrap();

    local_files::publish(&directory, id, "published.bin", false).unwrap();
    assert!(!part.exists());
    assert_eq!(
        fs::read(output.join("published.bin")).unwrap(),
        b"first payload"
    );
    assert_eq!(
        fs::metadata(output.join("published.bin")).unwrap().nlink(),
        1
    );

    fs::write(&part, b"replacement").unwrap();
    local_files::publish(&directory, id, "published.bin", true).unwrap();
    assert!(!part.exists());
    assert_eq!(
        fs::read(output.join("published.bin")).unwrap(),
        b"replacement"
    );
}

#[test]
fn publication_refuses_a_symlink_destination_and_identity_is_directory_bound() {
    let root = private_root();
    let first = private_directory(root.path(), "first");
    let second = private_directory(root.path(), "second");
    let first_dir = open_directory(&first, false).unwrap();
    let second_dir = open_directory(&second, false).unwrap();

    fs::write(first.join(".biorouter-crew-id.part"), b"safe").unwrap();
    symlink(first.join("outside"), first.join("published.bin")).unwrap();
    assert!(local_files::publish(&first_dir, "id", "published.bin", true).is_err());
    assert_ne!(
        local_files::destination_identity(&first_dir, "published.bin").unwrap(),
        local_files::destination_identity(&second_dir, "published.bin").unwrap()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn private_file_with_acl_allow_is_rejected_even_when_mode_is_0600() {
    use std::process::Command;

    let root = private_root();
    let file = root.path().join("partial");
    fs::write(&file, b"partial").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone allow read", file.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(validate_file_acl(&fs::File::open(&file).unwrap()).is_err());
}
