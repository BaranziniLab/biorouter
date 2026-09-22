#![cfg(unix)]

use biorouter_server::crew::local_files::{
    self, open_directory, select, selection_identity, Direction, ProtectedDirectory, TargetApproval,
};
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn private_directory(root: &Path, name: &str) -> PathBuf {
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
fn source_selection_stays_anchored_to_the_original_inode_after_path_replacement() {
    let root = private_root();
    let directory = private_directory(root.path(), "files");
    let source = directory.join("source.csv");
    fs::write(&source, b"original").unwrap();
    let original_inode = fs::metadata(&source).unwrap().ino();

    let selection = select(&source, Direction::Upload, false).unwrap();
    let approved_identity = selection_identity(&selection).unwrap();

    let moved = directory.join("source-original.csv");
    fs::rename(&source, &moved).unwrap();
    fs::write(&source, b"replacement").unwrap();
    assert_ne!(original_inode, fs::metadata(&source).unwrap().ino());

    assert_eq!(selection_identity(&selection).unwrap(), approved_identity);
    assert_eq!(fs::read(moved).unwrap(), b"original");
    assert_eq!(fs::read(source).unwrap(), b"replacement");
}

#[test]
fn selected_directory_inode_replacement_keeps_publication_on_the_original_directory() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let protected = ProtectedDirectory::new(open_directory(&output, false).unwrap()).unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let part = local_files::part_name(id);

    let moved = root.path().join("output-original");
    fs::rename(&output, &moved).unwrap();
    let replacement = private_directory(root.path(), "output");
    fs::write(replacement.join("final.bin"), b"replacement sentinel").unwrap();
    fs::write(moved.join(&part), b"approved payload").unwrap();

    let file = protected
        .open_with(&part, local_files::nofollow_options().read(true))
        .unwrap()
        .into_std();
    protected
        .publish_file(&file, part.to_str().unwrap(), "final.bin", false)
        .unwrap();

    assert_eq!(
        fs::read(moved.join("final.bin")).unwrap(),
        b"approved payload"
    );
    assert_eq!(
        fs::read(replacement.join("final.bin")).unwrap(),
        b"replacement sentinel"
    );
    assert!(!moved.join(&part).exists());
}

#[test]
fn raced_target_creation_refuses_implicit_overwrite_and_preserves_partial_and_sentinel() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let target = output.join("result.bin");
    let sentinel = root.path().join("sentinel.bin");
    fs::write(&sentinel, b"unrelated sentinel").unwrap();

    let selection = select(&target, Direction::Download, false).unwrap();
    let approved_identity = selection_identity(&selection).unwrap();
    let directory = open_directory(&output, false).unwrap();

    fs::hard_link(&sentinel, &target).unwrap();
    let id = "fedcba9876543210fedcba9876543210";
    fs::write(output.join(local_files::part_name(id)), b"new payload").unwrap();

    assert!(local_files::publish(&directory, id, "result.bin", false).is_err());
    assert_eq!(selection_identity(&selection).unwrap(), approved_identity);
    assert_eq!(fs::read(&target).unwrap(), b"unrelated sentinel");
    assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated sentinel");
    assert!(output.join(local_files::part_name(id)).is_file());
}

#[test]
fn symlink_substitution_is_refused_but_explicit_hardlink_replacement_preserves_sentinel() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let directory = open_directory(&output, false).unwrap();

    let symlink_target = output.join("symlink-target.bin");
    let destination = output.join("destination.bin");
    fs::write(&destination, b"approved destination").unwrap();
    let symlink_selection = select(&destination, Direction::Download, true).unwrap();
    fs::write(&symlink_target, b"protected sentinel").unwrap();
    fs::remove_file(&destination).unwrap();
    symlink(&symlink_target, &destination).unwrap();
    let symlink_id = "11111111111111111111111111111111";
    fs::write(
        output.join(local_files::part_name(symlink_id)),
        b"replacement payload",
    )
    .unwrap();

    assert!(local_files::publish(&directory, symlink_id, "destination.bin", true).is_err());
    assert_eq!(fs::read(&symlink_target).unwrap(), b"protected sentinel");
    assert!(fs::symlink_metadata(&destination)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(output.join(local_files::part_name(symlink_id)).is_file());
    assert!(selection_identity(&symlink_selection).is_ok());

    let hardlink_target = root.path().join("hardlink-sentinel.bin");
    let hardlink_destination = output.join("hardlink-destination.bin");
    fs::write(&hardlink_target, b"hardlink sentinel").unwrap();
    fs::write(&hardlink_destination, b"approved destination").unwrap();
    let hardlink_selection = select(&hardlink_destination, Direction::Download, true).unwrap();
    fs::remove_file(&hardlink_destination).unwrap();
    fs::hard_link(&hardlink_target, &hardlink_destination).unwrap();
    let hardlink_id = "22222222222222222222222222222222";
    fs::write(
        output.join(local_files::part_name(hardlink_id)),
        b"authorized replacement",
    )
    .unwrap();

    local_files::publish(&directory, hardlink_id, "hardlink-destination.bin", true).unwrap();
    assert_eq!(
        fs::read(&hardlink_destination).unwrap(),
        b"authorized replacement"
    );
    assert_eq!(fs::read(&hardlink_target).unwrap(), b"hardlink sentinel");
    assert_eq!(fs::metadata(&hardlink_target).unwrap().nlink(), 1);
    assert!(selection_identity(&hardlink_selection).is_ok());
    assert!(!output.join(local_files::part_name(hardlink_id)).exists());
}

#[test]
fn selected_target_must_still_match_for_overwrite_and_absent_target_cannot_be_created_raced() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let protected = ProtectedDirectory::new(open_directory(&output, false).unwrap()).unwrap();

    let existing = output.join("existing.bin");
    fs::write(&existing, b"approved A").unwrap();
    let existing_approval = TargetApproval::capture(&protected, "existing.bin").unwrap();
    fs::write(&existing, b"new B").unwrap();
    assert!(existing_approval
        .verify(&protected, "existing.bin")
        .is_err());

    let existing_id = "33333333333333333333333333333333";
    fs::write(
        output.join(local_files::part_name(existing_id)),
        b"must not publish",
    )
    .unwrap();
    let part = protected
        .open_with(
            local_files::part_name(existing_id),
            local_files::nofollow_options().read(true),
        )
        .unwrap()
        .into_std();
    assert!(protected
        .publish_selected(
            &part,
            &local_files::part_name(existing_id).to_string_lossy(),
            "existing.bin",
            true,
            &existing_approval,
        )
        .is_err());
    assert_eq!(fs::read(&existing).unwrap(), b"new B");
    assert!(output.join(local_files::part_name(existing_id)).is_file());

    let absent = output.join("absent.bin");
    let absent_approval = TargetApproval::capture(&protected, "absent.bin").unwrap();
    assert_eq!(absent_approval, TargetApproval::Absent);
    fs::write(&absent, b"raced target").unwrap();
    assert!(absent_approval.verify(&protected, "absent.bin").is_err());

    let absent_id = "44444444444444444444444444444444";
    fs::write(
        output.join(local_files::part_name(absent_id)),
        b"must not replace",
    )
    .unwrap();
    let part = protected
        .open_with(
            local_files::part_name(absent_id),
            local_files::nofollow_options().read(true),
        )
        .unwrap()
        .into_std();
    assert!(protected
        .publish_selected(
            &part,
            &local_files::part_name(absent_id).to_string_lossy(),
            "absent.bin",
            true,
            &absent_approval,
        )
        .is_err());
    assert_eq!(fs::read(&absent).unwrap(), b"raced target");
    assert!(output.join(local_files::part_name(absent_id)).is_file());

    let valid = output.join("valid.bin");
    fs::write(&valid, b"approved target").unwrap();
    let valid_approval = TargetApproval::capture(&protected, "valid.bin").unwrap();
    let valid_id = "77777777777777777777777777777777";
    fs::write(
        output.join(local_files::part_name(valid_id)),
        b"valid replacement",
    )
    .unwrap();
    let part = protected
        .open_with(
            local_files::part_name(valid_id),
            local_files::nofollow_options().read(true),
        )
        .unwrap()
        .into_std();
    protected
        .publish_selected(
            &part,
            &local_files::part_name(valid_id).to_string_lossy(),
            "valid.bin",
            true,
            &valid_approval,
        )
        .unwrap();
    assert_eq!(fs::read(&valid).unwrap(), b"valid replacement");
    assert!(!output.join(local_files::part_name(valid_id)).exists());
}

#[test]
fn publication_refuses_a_replaced_named_partial_even_with_the_original_fd() {
    let root = private_root();
    let output = private_directory(root.path(), "output");
    let protected = ProtectedDirectory::new(open_directory(&output, false).unwrap()).unwrap();
    let id = "88888888888888888888888888888888";
    let part = local_files::part_name(id);
    fs::write(output.join(&part), b"original partial").unwrap();
    let original = protected
        .open_with(&part, local_files::nofollow_options().read(true))
        .unwrap()
        .into_std();

    fs::remove_file(output.join(&part)).unwrap();
    fs::write(output.join(&part), b"replaced partial").unwrap();
    assert!(protected
        .publish_file(&original, part.to_str().unwrap(), "final.bin", false)
        .is_err());
    assert!(!output.join("final.bin").exists());
    assert_eq!(fs::read(output.join(&part)).unwrap(), b"replaced partial");
}
