#![cfg(unix)]

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use biorouter_server::crew::local_files::{
    self, open_directory, protected_directory, select, validate_file_acl, Direction, Selection,
    SelectionRefusal,
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
    assert_eq!(
        refusal(select(&existing, Direction::Download, false)),
        SelectionRefusal::Exists {
            name: "existing.bin".into()
        }
    );
    assert!(select(&existing, Direction::Download, true).is_ok());

    let existing_link = directory.join("existing-link.bin");
    symlink(&existing, &existing_link).unwrap();
    assert!(select(&existing_link, Direction::Download, true).is_err());

    // FILES-F9: a folder is refused as a folder, with or without approval to replace, and
    // however it is spelled.
    let existing_directory = directory.join("existing-directory");
    fs::create_dir(&existing_directory).unwrap();
    for overwrite in [false, true] {
        assert!(matches!(
            refusal(select(&existing_directory, Direction::Download, overwrite)),
            SelectionRefusal::Folder { .. }
        ));
    }
    let slashed = format!("{}/", existing_directory.display());
    assert!(matches!(
        refusal(select(Path::new(&slashed), Direction::Download, true)),
        SelectionRefusal::Folder { .. }
    ));

    // FILES-F3: a folder other accounts can change says so.
    let shared = root.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(
        refusal(select(
            &shared.join("counts.csv"),
            Direction::Download,
            false
        )),
        SelectionRefusal::SharedFolder
    );
}

fn refusal(result: anyhow::Result<Selection>) -> SelectionRefusal {
    let error = match result {
        Ok(selection) => panic!("expected a refusal for {}", selection.name()),
        Err(error) => error,
    };
    error
        .downcast_ref::<SelectionRefusal>()
        .cloned()
        .unwrap_or_else(|| panic!("expected a selection refusal, got: {error:#}"))
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

/// The value of extended attribute `name` on `path`, when it has one.
#[cfg(target_os = "macos")]
fn extended_attribute(path: &Path, name: &str) -> Option<String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let name = CString::new(name).unwrap();
    let mut buffer = vec![0u8; 1024];
    // SAFETY: both strings are NUL-terminated and live for the call, and the buffer's length
    // is the one passed.
    let read = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    (read >= 0).then(|| String::from_utf8_lossy(&buffer[..read as usize]).into_owned())
}

/// FILES-F8: a file received through Crew is written by the daemon, not a browser's download
/// manager, and carried no quarantine mark, so Gatekeeper never checked an app inside a shared
/// archive. A published download is marked as a browser would mark it; the daemon's own
/// receipts are not.
#[cfg(target_os = "macos")]
#[test]
fn a_published_download_is_marked_as_downloaded_and_a_receipt_is_not() {
    use std::io::Write;

    let root = private_root();
    let output = private_directory(root.path(), "output");
    let directory =
        local_files::ProtectedDirectory::new(open_directory(&output, false).unwrap()).unwrap();
    let partial = |id: &str, payload: &[u8]| {
        let part = local_files::part_name(id);
        let mut file = directory
            .open_with(
                &part,
                local_files::nofollow_options()
                    .read(true)
                    .write(true)
                    .create_new(true),
            )
            .unwrap()
            .into_std();
        file.write_all(payload).unwrap();
        (part.to_str().unwrap().to_owned(), file)
    };

    let (part, file) = partial("0123456789abcdef0123456789abcdef", b"PK\x03\x04archive");
    directory
        .publish_selected(
            &file,
            &part,
            "shared-tool.zip",
            false,
            &local_files::TargetApproval::Absent,
        )
        .unwrap();
    let mark = extended_attribute(&output.join("shared-tool.zip"), "com.apple.quarantine")
        .expect("a received file carries the quarantine mark");
    let fields: Vec<&str> = mark.split(';').collect();
    assert_eq!(fields.len(), 4, "{mark}");
    assert_eq!(fields[0], "0081", "{mark}");
    assert!(
        u64::from_str_radix(fields[1], 16).is_ok_and(|seconds| seconds > 0),
        "{mark}"
    );
    assert_eq!(fields[2], "Biorouter", "{mark}");
    assert!(uuid_shaped(fields[3]), "{mark}");
    // The partial was renamed, not copied: nothing is left under its name.
    assert!(!output.join(&part).exists());

    let (part, file) = partial("fedcba9876543210fedcba9876543210", b"{}");
    directory
        .publish_file(&file, &part, "receipts.json", true)
        .unwrap();
    assert_eq!(
        extended_attribute(&output.join("receipts.json"), "com.apple.quarantine"),
        None,
        "the daemon's own receipts are not downloads"
    );
}

#[cfg(target_os = "macos")]
fn uuid_shaped(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.iter().map(|group| group.len()).collect::<Vec<_>>() == [8, 4, 4, 4, 12]
        && groups
            .iter()
            .all(|group| group.chars().all(|c| c.is_ascii_hexdigit()))
}
