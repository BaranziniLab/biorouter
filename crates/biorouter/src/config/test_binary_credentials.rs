//! The credential store every Biorouter **test binary** uses in place of the OS one.
//!
//! A cargo test binary that builds a `keyring::Entry` reaches the real macOS Keychain, Windows
//! Credential Manager or Linux Secret Service, exactly as a shipped binary does. On macOS that
//! is a password dialog: a test binary is ad-hoc signed, its code hash changes on every rebuild,
//! so the Keychain asks again on every run and "Always Allow" never sticks. On 2026-11-12 the
//! owner's machine showed 12 such dialogs in one day, and the system log named a
//! `target/debug/deps/` test binary (`biorouter-…`, `biorouter_cli-…`, `biorouter_server-…`)
//! behind every one, each reading the real `biorouter` item. A dialog nobody answers blocks the
//! test forever.
//!
//! So every test binary that links this crate replaces the store before `main`:
//! `keep_this_test_binary_off_the_os_credential_store!` (exported at the crate root) installs a process-memory store as `keyring`'s default credential builder, from a `#[ctor]`.
//! Every `keyring::Entry::new` in the process, from any crate (`Config`'s secrets blob, Crew's
//! device keys, a `Config::new(path, service)` a test builds), then reads and writes a map in
//! this process. Nothing in the test process reaches the OS store, whatever
//! `BIOROUTER_DISABLE_KEYRING` says. A production binary a test starts (`biorouter`,
//! `biorouterd`) is another process, which no ctor here reaches: the three test sandboxes
//! export `BIOROUTER_DISABLE_KEYRING=true` for their children, and any other test that starts
//! one passes it on the child's `Command` itself.
//!
//! ⚠ There is no environment variable and no Cargo feature behind it, deliberately: either
//! could switch off the Keychain in a shipped binary, or in the `target/debug/biorouterd` a
//! developer's desktop app runs. The only way in is the macro, and
//! `tests::only_test_code_keeps_a_binary_off_the_os_credential_store` fails if anything under a
//! crate's `src/` invokes it outside a `#[cfg(test)]` item. It is public, and compiled into
//! every build, only because the CLI, server and ACP test binaries link this crate built
//! without `cfg(test)`, the same reason as `Config::global_if_initialized`.
//!
//! Where each test binary invokes it, and the census that keeps a new one from forgetting:
//! `tests::every_test_binary_that_links_biorouter_keeps_off_the_os_credential_store`. User-facing
//! account: `docs/security/secret-storage.md`, "Tests never use the credential store".

use std::any::Any;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi, CredentialPersistence};

/// `(target, service, user)`, the three names a `keyring::Entry` is built from.
type Key = (Option<String>, String, String);

/// Every secret a test binary stored, for the life of the process. Shared by every entry, as
/// the OS store is: an entry built for `("biorouter", "secrets")` reads what another entry for
/// the same names wrote. (`keyring`'s own `mock` store keeps one value per `Entry` value, so a
/// write through one entry is invisible to the next, which is not how any caller here uses it.)
static STORE: LazyLock<Mutex<HashMap<Key, Vec<u8>>>> = LazyLock::new(Mutex::default);

fn store() -> MutexGuard<'static, HashMap<Key, Vec<u8>>> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One entry in [`STORE`].
#[derive(Debug)]
struct ProcessMemoryCredential {
    key: Key,
}

impl CredentialApi for ProcessMemoryCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        store().insert(self.key.clone(), secret.to_vec());
        Ok(())
    }

    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        store()
            .get(&self.key)
            .cloned()
            .ok_or(keyring::Error::NoEntry)
    }

    fn delete_credential(&self) -> keyring::Result<()> {
        store()
            .remove(&self.key)
            .map(drop)
            .ok_or(keyring::Error::NoEntry)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct ProcessMemoryCredentialBuilder;

impl CredentialBuilderApi for ProcessMemoryCredentialBuilder {
    fn build(
        &self,
        target: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(ProcessMemoryCredential {
            key: (
                target.map(str::to_owned),
                service.to_owned(),
                user.to_owned(),
            ),
        }))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn persistence(&self) -> CredentialPersistence {
        CredentialPersistence::ProcessOnly
    }
}

/// Make every `keyring::Entry` this process builds from now on use [`STORE`].
///
/// Called by the `#[ctor]` that `keep_this_test_binary_off_the_os_credential_store!` expands
/// to, and by nothing else: call the macro, not this.
#[doc(hidden)]
pub fn keep_off_the_os_store() {
    keyring::set_default_credential_builder(Box::new(ProcessMemoryCredentialBuilder));
}

/// Whether a `keyring::Entry` built now would use [`STORE`] rather than the OS store.
///
/// Building an entry opens nothing: on every platform `keyring` supports, `Entry::new` only
/// records the names, and the store is reached by a read or a write. So this is safe to ask in
/// a binary whose store was never replaced, which is the case it exists to catch.
#[doc(hidden)]
pub fn os_store_is_kept_off() -> bool {
    keyring::Entry::new("biorouter", "test-binary-credential-store-probe")
        .is_ok_and(|entry| entry.get_credential().is::<ProcessMemoryCredential>())
}

/// Keep this test binary off the OS credential store: invoke once, at module level, in every
/// test binary that links `biorouter`.
///
/// It expands to a `#[ctor]` that installs the process-memory store before `main` (see
/// `biorouter::config::test_binary_credentials`), and a test
/// that fails if this binary would still build an entry on the OS store. The caller needs
/// `ctor` among its dependencies; every crate that invokes it has it as a dev-dependency.
///
/// Where it goes: the `biorouter`, `biorouter-cli` and `biorouter-server` libraries invoke it
/// in their `test_sandbox.rs`, which each of their test binaries declares; `biorouter-acp`'s
/// library invokes it in `lib.rs` under `#[cfg(test)]`; every `tests/*.rs` of `biorouter` and
/// `biorouter-acp` invokes it directly. Never in production code.
#[doc(hidden)]
#[macro_export]
macro_rules! keep_this_test_binary_off_the_os_credential_store {
    () => {
        #[ctor::ctor]
        fn keep_this_test_binary_off_the_os_credential_store() {
            $crate::config::test_binary_credentials::keep_off_the_os_store();
        }

        #[test]
        fn this_test_binary_never_reaches_the_os_credential_store() {
            assert!(
                $crate::config::test_binary_credentials::os_store_is_kept_off(),
                "this test binary builds keyring entries on the OS credential store (the macOS \
                 Keychain, Windows Credential Manager or Secret Service), so a test that reads \
                 a secret raises a password dialog nobody may be there to answer"
            );
        }
    };
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// The store answers like the OS one: a value written through one entry is read through
    /// another built for the same names, names that differ in any part are separate, and a
    /// delete leaves "no entry".
    #[test]
    fn entries_built_for_the_same_names_share_one_value() {
        let service = "biorouter-test-binary-credentials";
        let write = keyring::Entry::new(service, "alice").unwrap();
        write.set_password("first").unwrap();
        let read = keyring::Entry::new(service, "alice").unwrap();
        assert_eq!(read.get_password().unwrap(), "first");

        let other_user = keyring::Entry::new(service, "bob").unwrap();
        assert!(matches!(
            other_user.get_password(),
            Err(keyring::Error::NoEntry)
        ));
        let other_service = keyring::Entry::new("another-service", "alice").unwrap();
        assert!(matches!(
            other_service.get_password(),
            Err(keyring::Error::NoEntry)
        ));
        let other_target = keyring::Entry::new_with_target("elsewhere", service, "alice").unwrap();
        assert!(matches!(
            other_target.get_password(),
            Err(keyring::Error::NoEntry)
        ));

        write.set_password("second").unwrap();
        assert_eq!(read.get_password().unwrap(), "second");
        read.delete_credential().unwrap();
        assert!(matches!(write.get_password(), Err(keyring::Error::NoEntry)));
        assert!(matches!(
            write.delete_credential(),
            Err(keyring::Error::NoEntry)
        ));
    }

    /// The path a test binary really takes: a keyring-backed `Config`, the shape
    /// `Config::default()` builds when `BIOROUTER_DISABLE_KEYRING` is unset, writes a secret
    /// that a second `Config` on the same service then reads. Before this store, both calls
    /// reached the developer's real Keychain item.
    #[test]
    fn a_keyring_backed_config_keeps_its_secrets_in_process_memory() {
        let dir = tempfile::tempdir().unwrap();
        let service = "biorouter-test-binary-credentials-config";
        let first = crate::config::Config::new(dir.path().join("config.yaml"), service).unwrap();
        first
            .set_secret("TEST_BINARY_API_KEY", &serde_json::json!("sk-in-memory"))
            .unwrap();

        let second = crate::config::Config::new(dir.path().join("config.yaml"), service).unwrap();
        let read: String = second.get_secret("TEST_BINARY_API_KEY").unwrap();
        assert_eq!(read, "sk-in-memory");
        assert!(
            !dir.path().join("secrets.yaml").exists(),
            "the keyring path wrote no file"
        );
    }

    /// The workspace's `crates/` directory, canonical, so paths built under it compare equal.
    fn crates_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .canonicalize()
            .expect("crates/ resolves")
    }

    /// Every workspace crate whose test binaries link `biorouter` (itself included).
    fn crates_that_link_biorouter(crates_dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(crates_dir).expect("crates/ is readable") {
            let dir = entry.expect("a directory entry").path();
            let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
                continue;
            };
            let links = manifest.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("name = \"biorouter\"")
                    || (line.starts_with("biorouter ") || line.starts_with("biorouter="))
                        && line.contains("../biorouter\"")
            });
            if links {
                found.push(dir);
            }
        }
        found.sort();
        found
    }

    fn rust_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                rust_files_under(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    const MACRO: &str = "keep_this_test_binary_off_the_os_credential_store!";

    /// `source`'s lines, trimmed, without its comment lines: a doc comment that names the
    /// macro or `mod test_sandbox;` must never count as the code doing it. (The sandboxes' own
    /// docs name the macro, so a census that matched comments passed with the invocation gone.)
    fn code_of(source: &str) -> Vec<&str> {
        source
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//"))
            .collect()
    }

    /// Whether the attribute lines directly above `code[index]` include `#[cfg(test)]`.
    fn under_cfg_test(code: &[&str], index: usize) -> bool {
        code[..index]
            .iter()
            .rev()
            .take_while(|line| line.starts_with("#["))
            .any(|line| *line == "#[cfg(test)]")
    }

    /// Whether `line` declares the `test_sandbox` module, at any visibility, inline or not.
    fn declares_the_sandbox_module(line: &str) -> bool {
        let Some((before, after)) = line.split_once("mod test_sandbox") else {
            return false;
        };
        (before.is_empty() || before.starts_with("pub"))
            && (after.starts_with(';') || after.starts_with(' ') || after.starts_with('{'))
    }

    /// `code` declares `mod test_sandbox` as a `#[cfg(test)]` item.
    fn declares_test_sandbox(code: &[&str]) -> bool {
        (0..code.len()).any(|i| declares_the_sandbox_module(code[i]) && under_cfg_test(code, i))
    }

    /// `code` invokes the macro as a `#[cfg(test)]` item.
    fn invokes_under_cfg_test(code: &[&str]) -> bool {
        (0..code.len()).any(|i| code[i].contains(MACRO) && under_cfg_test(code, i))
    }

    /// `path`, relative to `crates/`, for a failure message.
    fn shown(path: &Path, crates_dir: &Path) -> String {
        path.strip_prefix(crates_dir)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    /// The roots of the crate's library and binary test binaries that can hold a test: the
    /// library when anything under `src/` has a `#[cfg(test)]`, and each binary target whose
    /// root declares modules or tests (`biorouterd`'s `main.rs` re-declares the server's module
    /// tree, tests included).
    fn library_and_binary_roots(dir: &Path) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if dir.join("src/lib.rs").exists() {
            let mut src = Vec::new();
            rust_files_under(&dir.join("src"), &mut src);
            if src.iter().any(|file| {
                std::fs::read_to_string(file).is_ok_and(|source| source.contains("#[cfg(test)]"))
            }) {
                roots.push(dir.join("src/lib.rs"));
            }
        }
        let mut bins = vec![dir.join("src/main.rs")];
        if let Ok(entries) = std::fs::read_dir(dir.join("src/bin")) {
            bins.extend(entries.filter_map(|e| e.ok().map(|e| e.path())));
        }
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        let mut in_bin = false;
        for line in manifest.lines().map(str::trim) {
            if line.starts_with('[') {
                in_bin = line == "[[bin]]";
            } else if let Some(path) = line
                .strip_prefix("path = \"")
                .and_then(|rest| rest.strip_suffix('"'))
                .filter(|_| in_bin)
            {
                bins.push(dir.join(path));
            }
        }
        bins.sort();
        bins.dedup();
        for bin in bins {
            let Ok(source) = std::fs::read_to_string(&bin) else {
                continue;
            };
            if source.contains("#[cfg(test)]")
                || source.contains("#[test]")
                || source.lines().any(|line| line.starts_with("mod "))
            {
                roots.push(bin);
            }
        }
        roots
    }

    /// Every top-level `tests/*.rs`: each is its own test binary. Subdirectories are helper
    /// modules, not binaries.
    fn integration_test_files(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir.join("tests")) else {
            return Vec::new();
        };
        entries
            .map(|entry| entry.expect("a directory entry").path())
            .filter(|path| {
                path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs")
            })
            .collect()
    }

    /// The census. Every test binary that links `biorouter` installs the process-memory store,
    /// because each is its own executable and nothing it does not declare runs in it:
    ///
    /// - each crate's library test binary, through `#[cfg(test)] mod test_sandbox;` (whose
    ///   `src/test_sandbox.rs` invokes the macro) or a `#[cfg(test)]` invocation in `lib.rs`;
    /// - each binary target whose root declares modules or tests (`biorouterd`'s `main.rs`
    ///   re-declares the server's module tree, tests included), the same way;
    /// - each top-level `tests/*.rs`, by invoking the macro, or by declaring `mod test_sandbox;`
    ///   where that crate's `src/test_sandbox.rs` invokes it.
    ///
    /// A new test file, crate or binary that forgets fails here, naming itself.
    #[test]
    fn every_test_binary_that_links_biorouter_keeps_off_the_os_credential_store() {
        let crates_dir = crates_dir();
        let crates = crates_that_link_biorouter(&crates_dir);
        let names: Vec<String> = crates
            .iter()
            .map(|dir| dir.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        for expected in [
            "biorouter",
            "biorouter-acp",
            "biorouter-cli",
            "biorouter-server",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "the census lost {expected}; it found {names:?}"
            );
        }

        let mut missing = Vec::new();
        let mut checked = 0;
        for dir in &crates {
            let sandbox_invokes = std::fs::read_to_string(dir.join("src/test_sandbox.rs"))
                .is_ok_and(|source| code_of(&source).iter().any(|line| line.contains(MACRO)));
            for root in library_and_binary_roots(dir) {
                checked += 1;
                let source = std::fs::read_to_string(&root).unwrap();
                let code = code_of(&source);
                if !((declares_test_sandbox(&code) && sandbox_invokes)
                    || invokes_under_cfg_test(&code))
                {
                    missing.push(shown(&root, &crates_dir));
                }
            }
            for file in integration_test_files(dir) {
                checked += 1;
                let source = std::fs::read_to_string(&file).unwrap();
                let code = code_of(&source);
                let invokes = code
                    .iter()
                    .any(|line| line.starts_with(&format!("biorouter::{MACRO}")));
                let declares = code.iter().any(|line| declares_the_sandbox_module(line));
                if !(invokes || (declares && sandbox_invokes)) {
                    missing.push(shown(&file, &crates_dir));
                }
            }
        }

        assert!(
            checked > 150,
            "the census checked only {checked} test binaries; it has stopped looking where they are"
        );
        missing.sort();
        assert!(
            missing.is_empty(),
            "these test binaries link biorouter but do not keep off the OS credential store, so \
             a test in them that reads a secret raises a Keychain password dialog: {missing:?}\n\n\
             In a tests/*.rs file, add near the top:\n\n    \
             biorouter::keep_this_test_binary_off_the_os_credential_store!();\n\n\
             (biorouter-cli and biorouter-server test files declare `mod test_sandbox;` instead, \
             which does it for them.)"
        );
    }

    /// ⚠ The guarantee that no production path loses the Keychain: nothing under any crate's
    /// `src/` invokes the macro, calls `keep_off_the_os_store` or replaces `keyring`'s
    /// default credential builder, except a `test_sandbox.rs` (declared `#[cfg(test)]`
    /// everywhere, which the census above and this test both check) or a `#[cfg(test)]` item.
    #[test]
    fn only_test_code_keeps_a_binary_off_the_os_credential_store() {
        let crates_dir = crates_dir();
        let this_file = crates_dir.join("biorouter/src/config/test_binary_credentials.rs");
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&crates_dir).unwrap() {
            rust_files_under(&entry.unwrap().path().join("src"), &mut files);
        }
        assert!(files.len() > 500, "found only {} source files", files.len());

        let mut offenders = Vec::new();
        for file in files {
            if file == this_file {
                continue;
            }
            let source = std::fs::read_to_string(&file).unwrap();
            let code = code_of(&source);
            let in_a_sandbox = file
                .file_name()
                .is_some_and(|name| name == "test_sandbox.rs");
            let shown = shown(&file, &crates_dir);
            for (index, line) in code.iter().enumerate() {
                if line.contains("keep_off_the_os_store")
                    || line.contains("set_default_credential_builder")
                {
                    offenders.push(format!("{shown}: replaces the credential store directly"));
                }
                // A `test_sandbox.rs` may invoke it unconditionally: every declaration of one,
                // checked just below, is `#[cfg(test)]`.
                if line.contains(MACRO) && !in_a_sandbox && !under_cfg_test(&code, index) {
                    offenders.push(format!(
                        "{shown}: invokes the macro outside a #[cfg(test)] item: {line}"
                    ));
                }
                if declares_the_sandbox_module(line) && !under_cfg_test(&code, index) {
                    offenders.push(format!(
                        "{shown}: declares test_sandbox outside #[cfg(test)]: {line}"
                    ));
                }
                // `#[path = "…/test_sandbox.rs"] mod other_name;` is the same module by
                // another name: the item under the attribute must be `#[cfg(test)]` too.
                if line.starts_with("#[path") && line.contains("test_sandbox.rs") {
                    let item = (index..code.len())
                        .find(|&i| !code[i].starts_with("#["))
                        .unwrap_or(code.len());
                    if item == code.len() || !under_cfg_test(&code, item) {
                        offenders.push(format!(
                            "{shown}: includes test_sandbox.rs outside #[cfg(test)]: {line}"
                        ));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "production code could switch a shipped binary off the OS credential store: {offenders:#?}"
        );
    }
}
