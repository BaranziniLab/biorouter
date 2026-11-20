//! The key behind Chrome's saved cookies, for the Biorouter Browser's Chrome import
//! (`docs/desktop-ui/built-in-browser.md`, BB-23 and BB-24).
//!
//! `biorouterd browser-key --source chrome` runs [`run`]. Electron main starts it only after the
//! person chose **Import** with logins selected and confirmed main's own dialog; it reads Chrome's
//! Safe Storage password from the operating system's store, derives the AES key Chrome encrypts
//! cookie values with, and writes it, masked, as one line of hex. Main unmasks and decrypts in
//! memory.
//!
//! Why a helper of the signed daemon and not `/usr/bin/security`: on macOS the person may answer
//! the Keychain prompt with **Always Allow**, which macOS remembers for the program that asked.
//! For `biorouterd` that is Biorouter; for `security` it would be every script on the computer.
//!
//! # Only Electron main may run it
//!
//! Always Allow also means that, from then on, anything that runs this signed binary gets the key
//! without a prompt, an agent's shell included. Two layers keep the key with the app:
//!
//! * The secret guard refuses any command that names the helper (`secret_guard/resolve.rs`,
//!   `HelperSighting`), which stops a model that was told to run it.
//! * The helper itself runs only for the desktop app's main process ([`caller`]). It asks the
//!   kernel for its parent process: on macOS the parent must be the main executable of an app
//!   bundle (`<app>.app/Contents/MacOS/`) and signed by Biorouter's Developer ID team, which only
//!   the app is, and the packaged app's Electron fuses (no `ELECTRON_RUN_AS_NODE`, no
//!   `NODE_OPTIONS`, no inspector, asar integrity) mean no code but the app's own runs in it; on
//!   Linux the parent must be the executable of the helper's own system install, both owned by
//!   root and writable by nobody else. A development build (`debug_assertions`, never shipped)
//!   also accepts a development Electron under `node_modules/electron/dist/`.
//!
//! And the key never travels in the clear: main writes a fresh 16 byte mask on the helper's
//! standard input, and the helper writes the key XOR that mask. A run without main's mask is
//! refused, so the key cannot reach a terminal, a log or a file even by accident.
//!
//! The helper never prints the password, never logs and opens no network connection. It refuses
//! to run when `BIOROUTER_DISABLE_KEYRING` is set, which every automated test sets, so no test can
//! show a Keychain prompt or read a real key.

use std::io::{Read, Write};
use std::num::NonZeroU32;

use zeroize::Zeroizing;

/// Chrome's PBKDF2 salt for cookie keys on macOS and Linux.
pub const SALT: &[u8] = b"saltysalt";
/// AES-128: Chrome derives 16 bytes.
pub const KEY_LEN: usize = 16;
/// Chrome on macOS derives the key from the Keychain password with 1,003 iterations.
pub const MACOS_ITERATIONS: u32 = 1003;
/// Chrome on Linux derives both `v10` and `v11` keys with one iteration.
pub const LINUX_ITERATIONS: u32 = 1;
/// The one-time request main writes on the helper's standard input: the mask for the key.
pub const REQUEST_LEN: usize = KEY_LEN;

/// Where the password comes from. Chrome only, for now; Chromium, Brave and Edge are later
/// additions of a service name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    Chrome,
}

impl KeySource {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "chrome" => Some(Self::Chrome),
            _ => None,
        }
    }
}

/// Why no key was written. The exit code is what Electron main reads; the sentence goes to
/// standard error for a person reading a log, and never names a value.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    KeyringDisabled,
    NotFound,
    NotAllowed,
    Unsupported,
    /// Not started by the desktop app's main process, or without its one-time request.
    NotFromApp,
}

impl Refusal {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::KeyringDisabled => 3,
            Self::NotFound => 4,
            Self::NotAllowed => 5,
            Self::Unsupported => 6,
            Self::NotFromApp => 7,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::KeyringDisabled => {
                "The operating system's key store is turned off (BIOROUTER_DISABLE_KEYRING)."
            }
            Self::NotFound => "Chrome's Safe Storage key was not found.",
            Self::NotAllowed => "Access to Chrome's Safe Storage key was not allowed.",
            Self::Unsupported => "Chrome's cookie key cannot be read on this system.",
            Self::NotFromApp => {
                "Only the Biorouter app can read Chrome's cookie key, while the person imports \
                 from Chrome."
            }
        }
    }
}

/// PBKDF2-HMAC-SHA1 over `password` with Chrome's salt, 16 bytes.
pub fn derive_key(password: &[u8], iterations: u32) -> Zeroizing<[u8; KEY_LEN]> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    let rounds = NonZeroU32::new(iterations.max(1)).expect("iterations is at least one");
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA1,
        rounds,
        SALT,
        password,
        key.as_mut(),
    );
    key
}

fn hex(bytes: &[u8]) -> Zeroizing<String> {
    let mut text = Zeroizing::new(String::with_capacity(bytes.len() * 2));
    for byte in bytes {
        text.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble is a hex digit"));
        text.push(char::from_digit(u32::from(byte & 0xf), 16).expect("a nibble is a hex digit"));
    }
    text
}

/// The same rule `Config` uses: the variable being set at all turns the key store off.
pub fn keyring_disabled() -> bool {
    std::env::var_os("BIOROUTER_DISABLE_KEYRING").is_some()
}

#[cfg(target_os = "macos")]
fn read_password(source: KeySource) -> Result<Zeroizing<Vec<u8>>, Refusal> {
    let KeySource::Chrome = source;
    // The Keychain item Chrome creates: service "Chrome Safe Storage", account "Chrome". Reading
    // it shows the operating system's own prompt, which Biorouter never answers or avoids.
    let entry =
        keyring::Entry::new("Chrome Safe Storage", "Chrome").map_err(|_| Refusal::NotAllowed)?;
    match entry.get_secret() {
        Ok(secret) => Ok(Zeroizing::new(secret)),
        Err(keyring::Error::NoEntry) => Err(Refusal::NotFound),
        Err(_) => Err(Refusal::NotAllowed),
    }
}

#[cfg(target_os = "linux")]
fn read_password(source: KeySource) -> Result<Zeroizing<Vec<u8>>, Refusal> {
    use dbus_secret_service::{EncryptionType, SecretService};
    use std::collections::HashMap;

    let KeySource::Chrome = source;
    let service = SecretService::connect(EncryptionType::Plain).map_err(|_| Refusal::NotFound)?;
    // Chrome writes the v2 schema since 2018 and reads v1 for older profiles.
    for schema in [
        "chrome_libsecret_os_crypt_password_v2",
        "chrome_libsecret_os_crypt_password_v1",
    ] {
        let attributes = HashMap::from([("xdg:schema", schema), ("application", "chrome")]);
        let found = service
            .search_items(attributes)
            .map_err(|_| Refusal::NotAllowed)?;
        let item = found.unlocked.first().or_else(|| found.locked.first());
        let Some(item) = item else { continue };
        if item.is_locked().unwrap_or(true) {
            item.unlock().map_err(|_| Refusal::NotAllowed)?;
        }
        let secret = item.get_secret().map_err(|_| Refusal::NotAllowed)?;
        return Ok(Zeroizing::new(secret));
    }
    Err(Refusal::NotFound)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_password(_source: KeySource) -> Result<Zeroizing<Vec<u8>>, Refusal> {
    // Windows: Chrome 127 and later lock cookies to Chrome itself (app bound encryption, `v20`),
    // and the ways around it behave like malware, so Biorouter does not try (BB-24).
    Err(Refusal::Unsupported)
}

fn iterations() -> u32 {
    if cfg!(target_os = "macos") {
        MACOS_ITERATIONS
    } else {
        LINUX_ITERATIONS
    }
}

/// Read main's one-time mask from `input`, then write the derived key for `source`, masked, to
/// `out` as one line of hex and return 0; or return a refusal's exit code after one sentence on
/// `err`.
pub fn run(
    source: KeySource,
    input: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    run_with(
        source,
        caller::started_by_the_app(),
        keyring_disabled(),
        read_password,
        input,
        out,
        err,
    )
}

fn run_with(
    source: KeySource,
    from_app: bool,
    disabled: bool,
    read: impl FnOnce(KeySource) -> Result<Zeroizing<Vec<u8>>, Refusal>,
    input: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let refuse = |refusal: Refusal, err: &mut dyn Write| {
        let _ = writeln!(err, "{}", refusal.message());
        refusal.exit_code()
    };
    // First, before the key store or the request is touched.
    if !from_app {
        return refuse(Refusal::NotFromApp, err);
    }
    if disabled {
        return refuse(Refusal::KeyringDisabled, err);
    }
    let mut mask = Zeroizing::new([0u8; REQUEST_LEN]);
    if input.read_exact(mask.as_mut()).is_err() {
        return refuse(Refusal::NotFromApp, err);
    }
    let password = match read(source) {
        Ok(password) => password,
        Err(refusal) => return refuse(refusal, err),
    };
    let mut key = derive_key(&password, iterations());
    for (byte, mask) in key.iter_mut().zip(mask.iter()) {
        *byte ^= mask;
    }
    let line = hex(key.as_ref());
    if writeln!(out, "{}", line.as_str())
        .and_then(|_| out.flush())
        .is_err()
    {
        return 1;
    }
    0
}

/// Whether the process that started this helper is the Biorouter desktop app's main process.
pub mod caller {
    use std::path::Path;

    /// Asked once, before anything else the helper does.
    pub fn started_by_the_app() -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            // SAFETY: getppid has no preconditions and cannot fail.
            let parent = unsafe { libc::getppid() };
            if parent <= 1 {
                return false;
            }
            let Some(executable) = executable_of(parent) else {
                return false;
            };
            let admitted = admitted(&executable, parent);
            // Still our parent: had it exited during the check, this process would have been
            // given to another, and the pid could name someone else by now.
            // SAFETY: as above.
            admitted && unsafe { libc::getppid() } == parent
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            false
        }
    }

    #[cfg(target_os = "macos")]
    fn admitted(executable: &Path, pid: libc::pid_t) -> bool {
        (is_app_main_executable(executable) && crate::privacy::system_auth_peer::pid_is_ours(pid))
            || (cfg!(debug_assertions) && is_development_electron(executable))
    }

    #[cfg(target_os = "linux")]
    fn admitted(executable: &Path, _pid: libc::pid_t) -> bool {
        let installed = std::env::current_exe().is_ok_and(|helper| {
            in_own_install(executable, &helper)
                && owned_by_root_and_locked(executable)
                && executable.parent().is_some_and(owned_by_root_and_locked)
        });
        installed || (cfg!(debug_assertions) && is_development_electron(executable))
    }

    /// The executable of `<name>.app/Contents/MacOS/`, and not one of the apps nested inside a
    /// bundle (Chromium's helper apps under `Contents/Frameworks/`, Copilot's under
    /// `Contents/Resources/`), which are signed by the same team.
    pub fn is_app_main_executable(executable: &Path) -> bool {
        let Some(dir) = executable.parent() else {
            return false;
        };
        dir.file_name().is_some_and(|name| name == "MacOS")
            && dir
                .parent()
                .and_then(Path::parent)
                .and_then(Path::extension)
                .is_some_and(|ext| ext == "app")
            && dir
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "Contents")
            && executable
                .to_string_lossy()
                .matches(".app/Contents/")
                .count()
                == 1
    }

    /// The unpackaged Electron `npm run start-gui` and `just run-dev` launch.
    pub fn is_development_electron(executable: &Path) -> bool {
        let text = executable.to_string_lossy().replace('\\', "/");
        text.contains("/node_modules/electron/dist/")
            && executable
                .file_name()
                .is_some_and(|name| name == "Electron" || name == "electron")
    }

    /// Linux packages: the helper is `<install>/resources/bin/biorouterd`, and the app's
    /// executable sits in `<install>/`.
    pub fn in_own_install(executable: &Path, helper: &Path) -> bool {
        let install = helper
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent);
        install.is_some() && executable.parent() == install
    }

    #[cfg(target_os = "linux")]
    fn owned_by_root_and_locked(path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).is_ok_and(|meta| meta.uid() == 0 && meta.mode() & 0o022 == 0)
    }

    #[cfg(target_os = "macos")]
    fn executable_of(pid: libc::pid_t) -> Option<std::path::PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is owned here and its length is passed with it.
        let length =
            unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if length <= 0 {
            return None;
        }
        buffer.truncate(length as usize);
        Some(Path::new(std::ffi::OsStr::from_bytes(&buffer)).to_path_buf())
    }

    #[cfg(target_os = "linux")]
    fn executable_of(pid: libc::pid_t) -> Option<std::path::PathBuf> {
        let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        // An executable replaced on disk since it started reads " (deleted)": not the app.
        (!path.to_string_lossy().ends_with(" (deleted)")).then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn hex_of(key: &[u8]) -> String {
        hex(key).to_string()
    }

    const MASK: [u8; REQUEST_LEN] = [
        0x5a, 0x01, 0xff, 0x10, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
        0xdd,
    ];

    fn masked(key: &[u8]) -> String {
        let bytes: Vec<u8> = key.iter().zip(MASK.iter()).map(|(k, m)| k ^ m).collect();
        hex_of(&bytes)
    }

    /// Known answers computed independently (RFC 8018 PBKDF2-HMAC-SHA1, salt "saltysalt").
    /// `peanuts` with one iteration is Chrome's fixed Linux `v10` key, which every Chromium
    /// cookie tool agrees on.
    #[test]
    fn derives_chromes_keys() {
        assert_eq!(
            hex_of(derive_key(b"peanuts", 1).as_ref()),
            "fd621fe5a2b402539dfa147ca9272778"
        );
        assert_eq!(derive_key(b"peanuts", 1).len(), KEY_LEN);
        assert_ne!(
            derive_key(b"fixture-password", MACOS_ITERATIONS).as_ref(),
            derive_key(b"fixture-password", LINUX_ITERATIONS).as_ref()
        );
    }

    #[test]
    fn parses_only_known_sources() {
        assert_eq!(KeySource::parse("chrome"), Some(KeySource::Chrome));
        assert_eq!(KeySource::parse("Chrome"), None);
        assert_eq!(KeySource::parse("brave"), None);
        assert_eq!(KeySource::parse(""), None);
    }

    /// S1: anything but the desktop app's main process is refused before the key store, the
    /// request or the environment is looked at. This test binary's parent is cargo, so the real
    /// entry point refuses it, whatever `BIOROUTER_DISABLE_KEYRING` says.
    #[test]
    fn a_process_the_app_did_not_start_is_refused_first() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(KeySource::Chrome, &mut &MASK[..], &mut out, &mut err);
        assert_eq!(code, Refusal::NotFromApp.exit_code());
        assert!(out.is_empty());
        assert!(String::from_utf8(err)
            .unwrap()
            .contains("Only the Biorouter app"));

        let mut out = Vec::new();
        let code = run_with(
            KeySource::Chrome,
            false,
            false,
            |_| panic!("the key store must not be read"),
            &mut &MASK[..],
            &mut out,
            &mut Vec::new(),
        );
        assert_eq!(code, Refusal::NotFromApp.exit_code());
        assert!(out.is_empty());
    }

    #[test]
    fn refuses_while_the_key_store_is_turned_off_without_reading_it() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            KeySource::Chrome,
            true,
            true,
            |_| panic!("the key store must not be read"),
            &mut &MASK[..],
            &mut out,
            &mut err,
        );
        assert_eq!(code, Refusal::KeyringDisabled.exit_code());
        assert!(out.is_empty());
        assert!(String::from_utf8(err).unwrap().contains("turned off"));
    }

    /// Without main's one-time mask on standard input nothing is read and nothing is written.
    #[test]
    fn a_run_without_mains_request_reads_no_key() {
        for request in [&[][..], &MASK[..REQUEST_LEN - 1]] {
            let mut out = Vec::new();
            let code = run_with(
                KeySource::Chrome,
                true,
                false,
                |_| panic!("the key store must not be read"),
                &mut &request[..],
                &mut out,
                &mut Vec::new(),
            );
            assert_eq!(code, Refusal::NotFromApp.exit_code());
            assert!(out.is_empty());
        }
    }

    /// The key leaves masked with main's request, so it never travels in the clear, and the
    /// password never leaves at all.
    #[test]
    fn writes_the_masked_key_and_never_the_password_or_the_key() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            KeySource::Chrome,
            true,
            false,
            |_| Ok(Zeroizing::new(b"fixture-password".to_vec())),
            &mut &MASK[..],
            &mut out,
            &mut err,
        );
        assert_eq!(code, 0);
        let line = String::from_utf8(out).unwrap();
        let key = derive_key(b"fixture-password", iterations());
        assert_eq!(line.trim_end(), masked(key.as_ref()));
        assert_ne!(line.trim_end(), hex_of(key.as_ref()));
        assert_eq!(line.trim_end().len(), KEY_LEN * 2);
        assert!(!line.contains("fixture-password"));
        assert!(err.is_empty());
    }

    #[test]
    fn each_refusal_has_its_own_exit_code_and_writes_no_key() {
        for refusal in [Refusal::NotFound, Refusal::NotAllowed, Refusal::Unsupported] {
            let expected = refusal.exit_code();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = run_with(
                KeySource::Chrome,
                true,
                false,
                |_| Err(refusal),
                &mut &MASK[..],
                &mut out,
                &mut err,
            );
            assert_eq!(code, expected);
            assert!(out.is_empty());
            assert!(!err.is_empty());
        }
        let codes = [
            Refusal::KeyringDisabled.exit_code(),
            Refusal::NotFound.exit_code(),
            Refusal::NotAllowed.exit_code(),
            Refusal::Unsupported.exit_code(),
            Refusal::NotFromApp.exit_code(),
        ];
        let unique: std::collections::BTreeSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
        assert!(!codes.contains(&0) && !codes.contains(&1));
    }

    /// Which parents the helper accepts, by path. On macOS a path is half the rule: the parent
    /// must also be signed by Biorouter's team, which a copied shell never is.
    #[test]
    fn only_an_apps_own_main_executable_is_a_parent() {
        use caller::{in_own_install, is_app_main_executable, is_development_electron};
        assert!(is_app_main_executable(Path::new(
            "/Applications/Biorouter.app/Contents/MacOS/Biorouter"
        )));
        assert!(is_app_main_executable(Path::new(
            "/Users/x/Applications/Biorouter 2.app/Contents/MacOS/Biorouter"
        )));
        for other in [
            "/bin/zsh",
            "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd",
            "/Applications/Biorouter.app/Contents/Frameworks/Biorouter Helper.app/Contents/MacOS/Biorouter Helper",
            "/Applications/Biorouter.app/Contents/Resources/computer-use/Biorouter Copilot.app/Contents/MacOS/agent-desktop",
            "/tmp/x/MacOS/sh",
            "/tmp/x.app/MacOS/sh",
        ] {
            assert!(!is_app_main_executable(Path::new(other)), "{other}");
        }

        assert!(is_development_electron(Path::new(
            "/repo/ui/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"
        )));
        assert!(is_development_electron(Path::new(
            "/repo/ui/desktop/node_modules/electron/dist/electron"
        )));
        assert!(!is_development_electron(Path::new("/usr/bin/electron")));
        assert!(!is_development_electron(Path::new(
            "/repo/node_modules/electron/dist/sh"
        )));

        let helper = Path::new("/usr/lib/biorouter/resources/bin/biorouterd");
        assert!(in_own_install(
            Path::new("/usr/lib/biorouter/Biorouter"),
            helper
        ));
        assert!(!in_own_install(Path::new("/usr/bin/bash"), helper));
        assert!(!in_own_install(
            Path::new("/usr/lib/biorouter/resources/bin/sh"),
            helper
        ));
    }
}
