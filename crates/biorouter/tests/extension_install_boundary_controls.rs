#![cfg(unix)]

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::fs;
use std::io::Write;
use std::path::Path;

use biorouter::extension_install::brxt::BrxtBundle;

fn bundle(root: &Path) -> BrxtBundle {
    let path = root.join("boundary.brxt");
    let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::FileOptions::default();
    let manifest = serde_json::json!({
        "name": "boundary-controls",
        "display_name": "Boundary controls",
        "description": "Owned archive extraction controls",
        "version": "1.0.0",
        "entry_point": "src/main.py",
        "repository": "https://example.invalid/boundary-controls",
        "env_vars": [],
    });
    for (name, body) in [
        ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
        ("README.md", b"# Synthetic boundary controls\n".to_vec()),
        (
            "pyproject.toml",
            b"[project]\nname = 'boundary-controls'\n".to_vec(),
        ),
        ("src/main.py", b"# synthetic archive payload\n".to_vec()),
    ] {
        archive.start_file(name, options).unwrap();
        archive.write_all(&body).unwrap();
    }
    archive.finish().unwrap();
    BrxtBundle::open(&path).unwrap()
}

#[test]
fn regular_nested_output_still_extracts() {
    let root = tempfile::tempdir().unwrap();
    let install = root.path().join("install");
    bundle(root.path()).extract_to(&install).unwrap();
    assert_eq!(
        fs::read(install.join("src/main.py")).unwrap(),
        b"# synthetic archive payload\n"
    );
}

#[test]
fn existing_internal_file_link_still_extracts() {
    let root = tempfile::tempdir().unwrap();
    let install = root.path().join("install");
    fs::create_dir_all(install.join("src")).unwrap();
    let target = install.join("internal.py");
    fs::write(&target, b"# original internal file\n").unwrap();
    let link = install.join("src/main.py");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    bundle(root.path()).extract_to(&install).unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"# synthetic archive payload\n");
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

#[test]
fn existing_internal_ancestor_link_still_extracts_new_children() {
    let root = tempfile::tempdir().unwrap();
    let install = root.path().join("install");
    let target = install.join("internal-source");
    fs::create_dir_all(&target).unwrap();
    let link = install.join("src");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    bundle(root.path()).extract_to(&install).unwrap();

    assert_eq!(
        fs::read(target.join("main.py")).unwrap(),
        b"# synthetic archive payload\n"
    );
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

#[test]
fn dangling_existing_links_do_not_create_targets_outside_the_package() {
    for ancestor in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("install");
        fs::create_dir_all(install.join("src")).unwrap();
        let target = root.path().join("outside-must-stay-missing");
        let link = if ancestor {
            fs::remove_dir(install.join("src")).unwrap();
            install.join("src")
        } else {
            install.join("src/main.py")
        };
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(bundle(root.path()).extract_to(&install).is_err());
        assert!(!target.exists());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
}
