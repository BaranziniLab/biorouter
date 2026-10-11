//! Guards the Agent Drafter templates against bytes that make the files
//! unreviewable.
//!
//! A single NUL byte anywhere in a text file makes `grep`/`ripgrep` classify it
//! as binary (they print "Binary file … matches" instead of the matching lines,
//! so a `grep … | head` pipeline appears to return *nothing*) and makes
//! `git diff` render "Binary files differ" instead of the patch. `sdk.ts` — the
//! most-reviewed file in the feature — carried one for the whole of SDK v2, and
//! it caused a review pass to wrongly conclude that real findings cited
//! nonexistent code. Never again.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use std::path::{Path, PathBuf};

fn templates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent_drafter/templates")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir)
        .expect("read templates dir")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn templates_contain_no_nul_bytes() {
    let mut files = Vec::new();
    walk(&templates_dir(), &mut files);
    assert!(!files.is_empty(), "no templates found");

    let mut offenders = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).expect("read template");
        if let Some(offset) = bytes.iter().position(|b| *b == 0) {
            let line = 1 + bytes[..offset].iter().filter(|b| **b == b'\n').count();
            offenders.push(format!("{}:{line} (byte offset {offset})", path.display()));
        }
    }

    assert!(
        offenders.is_empty(),
        "NUL byte(s) in template text files: grep will treat these as binary and \
         git diff will not show changes to them. Use an escape sequence (\\u0000) \
         instead of a literal NUL:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn templates_are_valid_utf8() {
    let mut files = Vec::new();
    walk(&templates_dir(), &mut files);

    for path in &files {
        let bytes = std::fs::read(path).expect("read template");
        assert!(
            std::str::from_utf8(&bytes).is_ok(),
            "{} is not valid UTF-8",
            path.display()
        );
    }
}
