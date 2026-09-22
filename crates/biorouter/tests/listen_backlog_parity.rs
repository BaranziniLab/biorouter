//! **Backlog parity**: `biorouter::net`'s listen backlog is still the one tokio's
//! own listeners get.
//!
//! `biorouter::net::bind_non_inheritable` replaces `tokio::net::TcpListener::bind`
//! for every production listener, so that a Windows child process cannot inherit
//! the socket (the reasoning is on the function). It builds the socket itself,
//! and so has to pass `listen()` the backlog mio would have passed. That number
//! is a hand copy of mio's per-platform `LISTEN_BACKLOG_SIZE` table, and a hand
//! copy drifts silently: after a mio bump every listener would quietly queue a
//! different number of pending connections than tokio's would. These tests pin
//! the mio version the table was copied from and compare it, row by row, with
//! that mio's own source.
//!
//! That call sites use the helper at all is not checked here. It is a compiler
//! lint, `scripts/check-non-inheritable-sockets.sh` (clippy's
//! `disallowed_methods`, force-warned, over `--lib --bins` in release), which
//! replaced the text census this file used to hold: a scanner cannot resolve
//! Rust paths, and a reviewer walked four spellings of a tokio bind past it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/biorouter
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<crate>/ has two ancestors")
        .to_path_buf()
}

/// The mio release whose `LISTEN_BACKLOG_SIZE` table (`src/sys/mod.rs`)
/// `biorouter::net`'s `LISTEN_BACKLOG` was copied from, cfg for cfg. When
/// Cargo.lock moves to another mio, re-diff the table first, then move this.
const MIO_THE_BACKLOG_TABLE_WAS_COPIED_FROM: &str = "1.1.1";

/// The mio version Cargo.lock resolves for tokio: the mio whose listeners
/// `bind_non_inheritable` stands in for.
fn locked_mio_for_tokio() -> String {
    let lock = std::fs::read_to_string(repo_root().join("Cargo.lock"))
        .expect("Cargo.lock must be readable at the repo root");
    let packages: Vec<(&str, &str, Vec<&str>)> = lock
        .split("[[package]]")
        .skip(1)
        .map(|block| {
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|l| l.strip_prefix(key))
                    .map(|v| v.trim().trim_matches('"'))
                    .unwrap_or("")
            };
            let dependencies = block
                .split_once("dependencies = [")
                .map(|(_, rest)| rest.split(']').next().unwrap_or(""))
                .unwrap_or("")
                .split(',')
                .map(|d| d.trim().trim_matches('"'))
                .filter(|d| !d.is_empty())
                .collect();
            (field("name = "), field("version = "), dependencies)
        })
        .collect();
    let tokio: Vec<_> = packages
        .iter()
        .filter(|(name, version, _)| *name == "tokio" && version.starts_with("1."))
        .collect();
    assert_eq!(
        tokio.len(),
        1,
        "expected exactly one tokio 1.x in Cargo.lock; found {tokio:?}"
    );
    let dep = tokio[0]
        .2
        .iter()
        .find(|d| **d == "mio" || d.starts_with("mio "))
        .unwrap_or_else(|| {
            panic!(
                "tokio in Cargo.lock no longer depends on mio: {:?}",
                tokio[0]
            )
        });
    // Cargo.lock writes a bare name when only one version is locked, and
    // `name version` when there are several.
    match dep.strip_prefix("mio ") {
        Some(version) => version.to_string(),
        None => {
            let mios: Vec<_> = packages.iter().filter(|(n, _, _)| *n == "mio").collect();
            assert_eq!(
                mios.len(),
                1,
                "a bare `mio` dependency with {mios:?} locked"
            );
            mios[0].1.to_string()
        }
    }
}

/// The directory cargo unpacked `crate_dir` (`mio-1.1.1`) into, if it did.
/// Cargo puts every dependency's source under `$CARGO_HOME/registry/src/<index>/`
/// before it builds it, so after a build this needs no network.
fn registry_source(crate_dir: &str) -> Result<PathBuf, String> {
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        homes.push(PathBuf::from(home));
    }
    for var in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(var) {
            homes.push(Path::new(&home).join(".cargo"));
        }
    }
    let mut searched = Vec::new();
    for home in &homes {
        let src = home.join("registry").join("src");
        searched.push(src.display().to_string());
        let Ok(indexes) = std::fs::read_dir(&src) else {
            continue;
        };
        for index in indexes.flatten() {
            let candidate = index.path().join(crate_dir);
            if candidate.join("Cargo.toml").is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!("no {crate_dir} under any of {searched:?}"))
}

/// A `const <name>: <type> = <value>;` table, as normalized `cfg` predicate ->
/// value. Each const's predicate is the `#[cfg(...)]` attribute above it, with
/// comments and whitespace removed and a trailing `,` before `)` dropped, so
/// `mio`'s commented, multi-line spelling compares equal to a plain one.
fn backlog_table(source: &str, const_name: &str) -> BTreeMap<String, String> {
    let lines: Vec<&str> = source
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect();
    let needle = format!("const {const_name}:");
    let mut table = BTreeMap::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains(&needle) {
            continue;
        }
        let value = line
            .split_once('=')
            .and_then(|(_, v)| v.split_once(';'))
            .map(|(v, _)| v.trim().to_string())
            .unwrap_or_else(|| panic!("`{}` has no `= value;`", line.trim()));
        let cfg_start = (0..i)
            .rev()
            .find(|&k| lines[k].trim_start().starts_with("#[cfg("))
            .unwrap_or_else(|| panic!("`{}` has no #[cfg] above it", line.trim()));
        let attribute: String = lines[cfg_start..i]
            .iter()
            .flat_map(|l| l.chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        let mut depth = 0i32;
        let mut predicate = String::new();
        for c in attribute.chars().skip("#[cfg".len()) {
            predicate.push(c);
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        table.insert(predicate.replace(",)", ")"), value);
    }
    table
}

fn our_backlog_table() -> BTreeMap<String, String> {
    let net = std::fs::read_to_string(repo_root().join("crates/biorouter/src/net.rs"))
        .expect("crates/biorouter/src/net.rs must be readable");
    backlog_table(&net, "LISTEN_BACKLOG")
}

/// `LISTEN_BACKLOG` is a hand copy, and nothing else notices when a mio bump
/// changes the table it was copied from: every listener would quietly queue a
/// different number of connections than tokio's would.
#[test]
fn the_backlog_table_was_copied_from_the_locked_mio() {
    let locked = locked_mio_for_tokio();
    assert_eq!(
        locked, MIO_THE_BACKLOG_TABLE_WAS_COPIED_FROM,
        "Cargo.lock resolves mio {locked} for tokio, but `LISTEN_BACKLOG` in \
         crates/biorouter/src/net.rs was copied from mio \
         {MIO_THE_BACKLOG_TABLE_WAS_COPIED_FROM}'s `LISTEN_BACKLOG_SIZE` \
         (src/sys/mod.rs). Re-diff that table against mio {locked}'s \
         src/sys/mod.rs, make `LISTEN_BACKLOG` match it cfg for cfg, and only \
         then set MIO_THE_BACKLOG_TABLE_WAS_COPIED_FROM to \"{locked}\" in this file."
    );
}

/// The same, row by row, against the locked mio's own source in the cargo
/// registry, which is on disk after any build (no network).
#[test]
fn the_backlog_table_matches_the_locked_mio_source() {
    let locked = locked_mio_for_tokio();
    let dir = registry_source(&format!("mio-{locked}")).unwrap_or_else(|e| {
        panic!(
            "cannot compare `LISTEN_BACKLOG` with mio {locked}'s source: {e}. Cargo \
             unpacks it there before building it, so set CARGO_HOME to the cargo home \
             this tree was built with."
        )
    });
    let theirs_path = dir.join("src/sys/mod.rs");
    let theirs = std::fs::read_to_string(&theirs_path)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", theirs_path.display()));
    let theirs = backlog_table(&theirs, "LISTEN_BACKLOG_SIZE");
    let ours = our_backlog_table();
    assert_eq!(
        theirs.len(),
        4,
        "mio {locked}'s LISTEN_BACKLOG_SIZE table no longer has the four rows it was \
         copied with; read {}: {theirs:#?}",
        theirs_path.display()
    );
    assert_eq!(
        ours,
        theirs,
        "`LISTEN_BACKLOG` in crates/biorouter/src/net.rs (left) is not mio {locked}'s \
         `LISTEN_BACKLOG_SIZE` (right, {}). A listener bound through \
         `bind_non_inheritable` would queue a different number of pending connections \
         than one bound by tokio. Copy the table again, cfg for cfg.",
        theirs_path.display()
    );
}

/// The comparison is not vacuous: a table that differs in one value, or in one
/// platform, is told apart from ours.
#[test]
fn the_backlog_comparison_notices_one_changed_row() {
    let ours = our_backlog_table();
    assert_eq!(ours.len(), 4, "{ours:#?}");
    let net = std::fs::read_to_string(repo_root().join("crates/biorouter/src/net.rs")).unwrap();
    for (from, to) in [
        (
            "const LISTEN_BACKLOG: c_int = 1024;",
            "const LISTEN_BACKLOG: c_int = 4096;",
        ),
        (
            "    target_os = \"openbsd\",\n    target_vendor",
            "    target_vendor",
        ),
    ] {
        assert_eq!(
            net.matches(from).count(),
            1,
            "`{from}` must occur once in net.rs"
        );
        let changed = backlog_table(&net.replacen(from, to, 1), "LISTEN_BACKLOG");
        assert_ne!(changed, ours, "changing `{from}` must change the table");
    }
}
