//! The key behind Chrome's saved cookies, for the Biorouter Browser's Chrome import
//! (`docs/desktop-ui/built-in-browser.md`, BB-23 and BB-24).
//!
//! `biorouterd browser-key --source chrome` runs [`run`]. Electron main starts it with a private
//! pipe only after the person chose **Import** with logins selected; it reads Chrome's Safe
//! Storage password from the operating system's store, derives the AES key Chrome encrypts cookie
//! values with, writes the key as one line of hex and exits. Main decrypts in memory.
//!
//! Why a helper of the signed daemon and not `/usr/bin/security`: on macOS the person may answer
//! the Keychain prompt with **Always Allow**, which macOS remembers for the program that asked.
//! For `biorouterd` that is Biorouter; for `security` it would be every script on the computer.
//!
//! The helper never prints the password, never logs and opens no network connection. It refuses
//! to run when `BIOROUTER_DISABLE_KEYRING` is set, which every automated test sets, so no test can
//! show a Keychain prompt or read a real key.

use std::io::Write;
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
}

impl Refusal {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::KeyringDisabled => 3,
            Self::NotFound => 4,
            Self::NotAllowed => 5,
            Self::Unsupported => 6,
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

/// Write the derived key for `source` to `out` as one line of hex and return 0, or return a
/// refusal's exit code after one sentence on `err`.
pub fn run(source: KeySource, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_with(source, keyring_disabled(), read_password, out, err)
}

fn run_with(
    source: KeySource,
    disabled: bool,
    read: impl FnOnce(KeySource) -> Result<Zeroizing<Vec<u8>>, Refusal>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let refuse = |refusal: Refusal, err: &mut dyn Write| {
        let _ = writeln!(err, "{}", refusal.message());
        refusal.exit_code()
    };
    if disabled {
        return refuse(Refusal::KeyringDisabled, err);
    }
    let password = match read(source) {
        Ok(password) => password,
        Err(refusal) => return refuse(refusal, err),
    };
    let key = derive_key(&password, iterations());
    let line = hex(key.as_ref());
    if writeln!(out, "{}", line.as_str())
        .and_then(|_| out.flush())
        .is_err()
    {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_of(key: &[u8]) -> String {
        hex(key).to_string()
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

    #[test]
    fn refuses_while_the_key_store_is_turned_off_without_reading_it() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            KeySource::Chrome,
            true,
            |_| panic!("the key store must not be read"),
            &mut out,
            &mut err,
        );
        assert_eq!(code, Refusal::KeyringDisabled.exit_code());
        assert!(out.is_empty());
        assert!(String::from_utf8(err).unwrap().contains("turned off"));
    }

    #[test]
    fn writes_the_derived_key_and_never_the_password() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            KeySource::Chrome,
            false,
            |_| Ok(Zeroizing::new(b"fixture-password".to_vec())),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 0);
        let line = String::from_utf8(out).unwrap();
        assert_eq!(
            line.trim_end(),
            hex_of(derive_key(b"fixture-password", iterations()).as_ref())
        );
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
                false,
                |_| Err(refusal),
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
        ];
        let unique: std::collections::BTreeSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
        assert!(!codes.contains(&0) && !codes.contains(&1));
    }
}
