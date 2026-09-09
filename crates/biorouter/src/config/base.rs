use crate::config::paths::Paths;
use crate::config::BioRouterMode;
use fs2::FileExt;
use keyring::Entry;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml::Mapping;
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;

tokio::task_local! {
    /// Per-task overrides for `get_secret` / `get_param` resolution.
    ///
    /// This exists so a candidate credential (e.g. an API key being validated
    /// during provider auto-detection) can be supplied to provider constructors
    /// **without** mutating the process environment. Mutating `std::env` from a
    /// multi-threaded program is unsound (`set_var`/`remove_var` race with any
    /// other thread reading the environment) and leaks the value into every
    /// subprocess spawned during the window. A task-local override is instead
    /// scoped to the probing task only: concurrent agent turns never observe it,
    /// and nothing touches the real environment. Keys are the upper-cased config
    /// key names (the same form `get_secret`/`get_param` look up).
    static CONFIG_OVERRIDES: HashMap<String, String>;
}

/// Look up a task-local override for the given upper-cased key, if one is set in
/// the current task scope. Returns `None` when no override scope is active (the
/// common case) so callers fall through to environment / keyring resolution.
fn override_lookup(env_key: &str) -> Option<String> {
    CONFIG_OVERRIDES
        .try_with(|m| m.get(env_key).cloned())
        .ok()
        .flatten()
}

/// Run `fut` with the given config overrides active for the duration of its
/// execution (and any synchronous `get_secret`/`get_param` calls it makes).
///
/// The overrides take precedence over both environment variables and the
/// keyring, but only within this task — they are never written to the process
/// environment. Used by provider auto-detection to test a candidate key.
pub async fn with_config_overrides<F, T>(overrides: HashMap<String, String>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CONFIG_OVERRIDES.scope(overrides, fut).await
}

const KEYRING_SERVICE: &str = "biorouter";
const KEYRING_USERNAME: &str = "secrets";
pub const CONFIG_YAML_NAME: &str = "config.yaml";

// Windows Credential Manager caps a credential blob at 2560 bytes (1280 UTF-16
// units), so on Windows the secrets JSON is split across continuation entries
// named "secrets.1", "secrets.2", ... with the main entry holding a chunk-count
// header. Other platforms have no practical limit and always use one entry —
// extra entries on macOS would mean extra Keychain authorization prompts.
const KEYRING_CHUNK_MARKER: &str = "__BIOROUTER_CHUNKED__:";
#[cfg(windows)]
const KEYRING_CHUNK_UTF16_LIMIT: usize = 1000;
#[cfg(not(windows))]
const KEYRING_CHUNK_UTF16_LIMIT: usize = usize::MAX;
const KEYRING_MAX_CHUNKS: usize = 64;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Configuration value not found: {0}")]
    NotFound(String),
    #[error("Failed to deserialize value: {0}")]
    DeserializeError(String),
    #[error("Failed to read config file: {0}")]
    FileError(#[from] std::io::Error),
    #[error("Failed to create config directory: {0}")]
    DirectoryError(String),
    #[error("Failed to access keyring: {0}")]
    KeyringError(String),
    #[error("Failed to lock config file: {0}")]
    LockError(String),
    #[error("Secret stored using file-based fallback")]
    FallbackToFileStorage,
}

impl From<serde_json::Error> for ConfigError {
    fn from(err: serde_json::Error) -> Self {
        ConfigError::DeserializeError(err.to_string())
    }
}

impl From<serde_yaml::Error> for ConfigError {
    fn from(err: serde_yaml::Error) -> Self {
        ConfigError::DeserializeError(err.to_string())
    }
}

impl From<keyring::Error> for ConfigError {
    fn from(err: keyring::Error) -> Self {
        ConfigError::KeyringError(err.to_string())
    }
}

/// Configuration management for biorouter.
///
/// This module provides a flexible configuration system that supports:
/// - Dynamic configuration keys
/// - Multiple value types through serde deserialization
/// - Environment variable overrides
/// - YAML-based configuration file storage
/// - Hot reloading of configuration changes
/// - Secure secret storage in system keyring
///
/// Configuration values are loaded with the following precedence:
/// 1. Environment variables (exact key match)
/// 2. Configuration file (~/.config/biorouter/config.yaml by default)
///
/// Secrets are loaded with the following precedence:
/// 1. Environment variables (exact key match)
/// 2. System keyring (which can be disabled with BIOROUTER_DISABLE_KEYRING).
///    The keyring is read at most once per process and cached in memory, so
///    macOS shows at most one Keychain authorization prompt per run.
/// 3. If the keyring is disabled (or the platform store is unavailable,
///    e.g. headless Linux), secrets are stored in a secrets file
///    (~/.config/biorouter/secrets.yaml by default)
///
/// # Examples
///
/// ```no_run
/// use biorouter::config::Config;
/// use serde::Deserialize;
///
/// // Get a string value
/// let config = Config::global();
/// let api_key: String = config.get_param("OPENAI_API_KEY").unwrap();
///
/// // Get a complex type
/// #[derive(Deserialize)]
/// struct ServerConfig {
///     host: String,
///     port: u16,
/// }
///
/// let server_config: ServerConfig = config.get_param("server").unwrap();
/// ```
///
/// # Naming Convention
/// we recommend snake_case for keys, and will convert to UPPERCASE when
/// checking for environment overrides. e.g. openai_api_key will check for an
/// environment variable OPENAI_API_KEY
///
/// For biorouter-specific configuration, consider prefixing with "biorouter_" to avoid conflicts.
pub struct Config {
    config_path: PathBuf,
    secrets: SecretStorage,
    guard: Mutex<()>,
    // Process-lifetime cache of the secrets map. The OS credential store is
    // read at most once per process; without this, every secret lookup is a
    // separate store access — on macOS that means one Keychain authorization
    // prompt per lookup until the user clicks "Always Allow".
    secrets_cache: Mutex<Option<HashMap<String, Value>>>,
    // Single-flight gate for the COLD read above. The cache alone does not make
    // the comment's "at most once per process" true: `all_secrets` checks the
    // cache, releases that lock, then reads the store, so N concurrent callers
    // on a cold cache all miss and all read. `/config/providers` is exactly that
    // shape -- it `join_all`s ~45 providers, each calling `check_provider_configured`
    // -- so the first settings load could issue dozens of simultaneous Keychain
    // reads: a prompt storm, and, when some of those racing reads fail, a grid
    // where one provider says Configured and its neighbour says not, from the
    // same credential blob.
    secrets_read: Mutex<()>,
    // Parsed contents of `config.yaml`, against the file state that produced
    // them. Without this, EVERY `get_param` re-read and re-parsed the whole
    // file: `ModelConfig::new`, the privacy tier resolver, the extension map
    // and the agent's turn loop all sit on that path, and at start-up — before
    // the file exists — every one of those reads was also a WRITE, which is how
    // a config-layer storm came to fail `test (windows-latest)` twice (#188,
    // #197). A `stat` per lookup keeps a change made by another process (the
    // CLI, `biorouter configure`) visible; see `FileStamp`.
    values_cache: Mutex<Option<CachedConfig>>,
    // Single-flight gate for the COLD load above, for the same reason
    // `secrets_read` exists beside `secrets_cache`: the cache alone lets N
    // concurrent callers all miss and all read. Here that is not merely
    // wasteful — on a missing config file every one of those misses takes the
    // CREATE branch, which is the storm itself.
    //
    // ⚠ Lock order is `guard` → `values_read` → `values_cache`, and the cache
    // locks are innermost by construction: nothing under either of them takes
    // `guard`, and `save_values` (which runs under `guard`) touches
    // `values_cache` alone. Filling the cache from inside `load` while holding
    // a lock that `save_values` also needs would deadlock against `load`'s own
    // create branch, which writes.
    values_read: Mutex<()>,
    // The last failure to write a default config file. Reported out of band —
    // `load` stays infallible about it on purpose, because making an unwritable
    // config directory a hard failure would turn the storm path this layer
    // spent two PRs calming down into a start-up crash. See
    // `record_default_config_write_error`.
    last_write_error: Mutex<Option<String>>,
    // Test-only replacement for the OS credential store, so cache and
    // chunking behavior can be exercised without touching a real keyring
    // (which would show authorization prompts on macOS).
    #[cfg(test)]
    test_keyring_store: Option<std::sync::Arc<dyn KeyringBlobStore + Send + Sync>>,
    // Test-only injection of the Windows failure this file's retries exist for.
    // See `IoFaults`.
    #[cfg(test)]
    io_faults: IoFaults,
    // Test-only counters. See `IoProbe`.
    #[cfg(test)]
    io_probe: IoProbe,
}

enum SecretStorage {
    Keyring { service: String },
    File { path: PathBuf },
}

/// Minimal username → value store interface over the OS keyring, so the
/// chunking logic can be exercised in tests without a real credential store
/// (the keyring mock has no shared state between Entry instances).
trait KeyringBlobStore {
    fn get(&self, username: &str) -> Result<String, keyring::Error>;
    fn set(&self, username: &str, value: &str) -> Result<(), keyring::Error>;
    fn delete(&self, username: &str) -> Result<(), keyring::Error>;
}

struct OsKeyringStore<'a> {
    service: &'a str,
}

impl KeyringBlobStore for OsKeyringStore<'_> {
    fn get(&self, username: &str) -> Result<String, keyring::Error> {
        Entry::new(self.service, username)?.get_password()
    }
    fn set(&self, username: &str, value: &str) -> Result<(), keyring::Error> {
        Entry::new(self.service, username)?.set_password(value)
    }
    fn delete(&self, username: &str) -> Result<(), keyring::Error> {
        Entry::new(self.service, username)?.delete_credential()
    }
}

// Global instance
static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

impl Default for Config {
    fn default() -> Self {
        let config_dir = Paths::config_dir();

        let config_path = config_dir.join(CONFIG_YAML_NAME);

        let secrets = match env::var("BIOROUTER_DISABLE_KEYRING") {
            Ok(_) => SecretStorage::File {
                path: config_dir.join("secrets.yaml"),
            },
            Err(_) => SecretStorage::Keyring {
                service: KEYRING_SERVICE.to_string(),
            },
        };
        Config {
            config_path,
            secrets,
            guard: Mutex::new(()),
            secrets_cache: Mutex::new(None),
            secrets_read: Mutex::new(()),
            values_cache: Mutex::new(None),
            values_read: Mutex::new(()),
            last_write_error: Mutex::new(None),
            #[cfg(test)]
            test_keyring_store: None,
            #[cfg(test)]
            io_faults: IoFaults::default(),
            #[cfg(test)]
            io_probe: IoProbe::default(),
        }
    }
}

pub trait ConfigValue {
    const KEY: &'static str;
    const DEFAULT: &'static str;
}

macro_rules! config_value {
    ($key:ident, $type:ty) => {
        impl Config {
            paste::paste! {
                pub fn [<get_ $key:lower>](&self) -> Result<$type, ConfigError> {
                    self.get_param(stringify!($key))
                }
            }
            paste::paste! {
                pub fn [<set_ $key:lower>](&self, v: impl Into<$type>) -> Result<(), ConfigError> {
                    self.set_param(stringify!($key), &v.into())
                }
            }
        }
    };

    ($key:ident, $inner:ty, $default:expr) => {
        paste::paste! {
            #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
            #[serde(transparent)]
            pub struct [<$key:camel>]($inner);

            impl ConfigValue for [<$key:camel>] {
                const KEY: &'static str = stringify!($key);
                const DEFAULT: &'static str = $default;
            }

            impl Default for [<$key:camel>] {
                fn default() -> Self {
                    [<$key:camel>]($default.into())
                }
            }

            impl std::ops::Deref for [<$key:camel>] {
                type Target = $inner;

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }

            impl std::ops::DerefMut for [<$key:camel>] {
                fn deref_mut(&mut self) -> &mut Self::Target {
                    &mut self.0
                }
            }

            impl std::fmt::Display for [<$key:camel>] {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{:?}", self.0)
                }
            }

            impl From<$inner> for [<$key:camel>] {
                fn from(value: $inner) -> Self {
                    [<$key:camel>](value)
                }
            }

            impl From<[<$key:camel>]> for $inner {
                fn from(value: [<$key:camel>]) -> $inner {
                    value.0
                }
            }

            config_value!($key, [<$key:camel>]);
        }
    };
}

fn parse_yaml_content(content: &str) -> Result<Mapping, ConfigError> {
    serde_yaml::from_str(content).map_err(|e| e.into())
}

/// A cheap identity for the config file's current state, from one `stat`.
///
/// `None` (as `Option<FileStamp>`) means the file is not there, which is itself
/// a state worth caching: on an install whose config directory cannot be
/// written, "absent" is the steady state and re-deriving it per lookup is the
/// whole cost this cache exists to remove.
///
/// The same shape `crate::catalog::file_stamp` already uses to watch this file
/// for changes made by another process — deliberately, so there is one answer
/// to "has `config.yaml` moved?" rather than two that can disagree — plus the
/// permission bits, which that watcher does not need and this does: a config
/// that could not be READ caches its failure, and a `chmod` that repairs it
/// moves neither the length nor the modification time.
///
/// ⚠ Two states with the same length, mtime and mode are indistinguishable
/// here. In-process writes never rely on that (every one of them invalidates
/// the cache explicitly), so the residual is a *different process* rewriting
/// `config.yaml` to the same length within one mtime tick — sub-microsecond on
/// APFS, ext4 and NTFS, and one second on the rare filesystem that truncates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileStamp {
    len: u64,
    modified: std::time::SystemTime,
    permissions: u32,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(FileStamp {
            len: meta.len(),
            modified: meta.modified().ok()?,
            permissions: permission_bits(&meta),
        })
    }
}

#[cfg(unix)]
fn permission_bits(meta: &std::fs::Metadata) -> u32 {
    std::os::unix::fs::PermissionsExt::mode(&meta.permissions())
}

#[cfg(not(unix))]
fn permission_bits(meta: &std::fs::Metadata) -> u32 {
    u32::from(meta.permissions().readonly())
}

/// A read failure, remembered in a form that can be handed to more than one
/// caller.
///
/// [`ConfigError`] is not `Clone` — it carries a [`std::io::Error`] — so a
/// cached failure is stored as the kind and message and rebuilt on the way out.
/// The raw OS code is dropped in that round trip, which is safe **because
/// nothing decides anything on it after the fact**: [`is_transiently_unavailable`]
/// only ever inspects the live error inside [`retry_while_transiently_unavailable`],
/// and the consumers that match on a returned `ConfigError` (`validate_max_tokens`
/// and its neighbours) match on the variant.
#[derive(Clone, Debug)]
struct CachedFileError {
    kind: std::io::ErrorKind,
    message: String,
}

impl CachedFileError {
    fn rebuild(&self) -> ConfigError {
        ConfigError::FileError(std::io::Error::new(self.kind, self.message.clone()))
    }
}

/// The outcome of one full load, held against the file state that produced it.
struct CachedConfig {
    /// The stamp observed **before** the read that produced `outcome`, and
    /// only when the load left the file in that same state.
    ///
    /// Before, not after, and the asymmetry is the point: a writer landing
    /// during our read leaves the post-read stamp describing content we never
    /// saw, so caching against it would serve that content's identity with the
    /// previous content's values, indefinitely. Against the pre-read stamp the
    /// same race costs a miss on the next call and nothing else.
    ///
    /// The "same state" qualifier is the other half, and it is not a nicety:
    /// the load path WRITES, so a load that created, restored or replaced the
    /// file produced a state its own starting stamp does not describe. Those
    /// store nothing at all — see the tail of [`Config::load_shared`].
    stamp: Option<FileStamp>,
    outcome: Result<Arc<Mapping>, CachedFileError>,
}

/// Attempts a filesystem operation gets before its failure is believed.
///
/// Eight, with the 1 ms → 16 ms backoff below, is ~63 ms in the worst case —
/// three orders of magnitude more than the window it exists to outlast, and
/// small enough that a config the user really cannot read still reports so
/// promptly.
const TRANSIENT_IO_ATTEMPTS: usize = 8;
const TRANSIENT_IO_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_millis(16);

/// Whether an I/O failure is Windows saying "this name is mid-replacement",
/// rather than saying anything about this process's rights to the file.
///
/// `std::fs::rename` is a *replace* on Windows. While it supersedes the
/// destination, the destination's NAME is briefly unopenable: a `CreateFileW`
/// that lands in that window is answered `STATUS_DELETE_PENDING`, which the
/// Win32 layer reports as `ERROR_ACCESS_DENIED` (5) — `PermissionDenied`, with
/// the message "Access is denied.". The neighbouring outcomes are
/// `ERROR_SHARING_VIOLATION` (32) and `ERROR_LOCK_VIOLATION` (33), and Rust
/// maps neither to a named [`std::io::ErrorKind`], so those two are matched by
/// raw code.
///
/// Unix has no equivalent — `rename(2)` is atomic and a reader never sees the
/// seam — so there this only ever matches a real `EACCES`, which costs the
/// retry budget once and is then reported unchanged.
fn is_transiently_unavailable(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    cfg!(windows) && matches!(err.raw_os_error(), Some(32 | 33))
}

/// Run one filesystem operation, retrying while it fails the way a name that a
/// sibling is replacing fails.
///
/// The operation is a closure so the retry rule is checkable on every platform:
/// the failure it exists for is Windows-only, sub-millisecond and produced by
/// an interleaving no test can arrange, so injecting it is the only way to
/// assert anything about the response to it.
fn retry_while_transiently_unavailable<T>(
    mut attempt: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut backoff = std::time::Duration::from_millis(1);
    for _ in 1..TRANSIENT_IO_ATTEMPTS {
        match attempt() {
            Err(err) if is_transiently_unavailable(&err) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(TRANSIENT_IO_MAX_BACKOFF);
            }
            settled => return settled,
        }
    }
    attempt()
}

/// Test-only injection of the failure Windows produces while a sibling replaces
/// the config file's name.
///
/// Every field is a count of *attempts* to fail, applied per call, so an armed
/// read or rename fails that many times and then behaves normally — exactly the
/// shape [`retry_while_transiently_unavailable`] exists to absorb. Arming more
/// than [`TRANSIENT_IO_ATTEMPTS`] exhausts the budget and makes the operation
/// report the failure, which is how the give-up path stays testable too.
#[cfg(test)]
#[derive(Default)]
struct IoFaults {
    failing_read_attempts: std::sync::atomic::AtomicUsize,
    failing_rename_attempts: std::sync::atomic::AtomicUsize,
    /// Config content a "sibling" installs the instant an injected rename fault
    /// fires. That is the race the fault stands in for — our rename lost
    /// because another writer got there first — and it is the only way to reach
    /// the adopt-after-a-lost-write arm of
    /// [`Config::create_default_config_if_missing`] deterministically.
    sibling_lands_on_rename_fault: Mutex<Option<String>>,
}

#[cfg(test)]
impl IoFaults {
    /// The error CI reported, verbatim: `Os { code: 5, kind: PermissionDenied,
    /// message: "Access is denied." }`.
    fn access_denied() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access is denied.")
    }

    fn failing_reads(&self) -> usize {
        self.failing_read_attempts
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn failing_renames(&self) -> usize {
        self.failing_rename_attempts
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Land the sibling's config, once, at the moment a rename fault fires.
    fn land_sibling_config(&self, at: &Path) {
        if let Some(content) = self.sibling_lands_on_rename_fault.lock().unwrap().take() {
            let _ = std::fs::write(at, content);
        }
    }
}

/// Test-only counts of what this instance actually did to the filesystem.
///
/// Separate from [`IoFaults`], which injects failures: these count successes,
/// and they exist because the properties worth asserting about the values
/// cache are all of the form *how many times did the disk get touched?*. A
/// test that only compares returned values cannot tell a cache hit from a
/// re-read that happened to agree, which is the assertion that matters here —
/// including for the retry budget, where the observable difference between
/// "reported the same error again" and "spent another ~63 ms first" is the
/// attempt count and nothing else.
#[cfg(test)]
#[derive(Default)]
struct IoProbe {
    /// Attempts made to read `config.yaml`, retries included.
    read_attempts: std::sync::atomic::AtomicUsize,
    /// Completed calls to [`Config::save_values`], successful or not.
    config_writes: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl IoProbe {
    fn note_read_attempt(&self) {
        self.read_attempts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn note_config_write(&self) {
        self.config_writes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn read_attempts(&self) -> usize {
        self.read_attempts
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn config_writes(&self) -> usize {
        self.config_writes
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Config {
    /// Get the global configuration instance.
    ///
    /// This will initialize the configuration with the default path (~/.config/biorouter/config.yaml)
    /// if it hasn't been initialized yet.
    pub fn global() -> &'static Config {
        GLOBAL_CONFIG.get_or_init(Config::default)
    }

    /// Create a new configuration instance with custom paths
    ///
    /// This is primarily useful for testing or for applications that need
    /// to manage multiple configuration files.
    pub fn new<P: AsRef<Path>>(config_path: P, service: &str) -> Result<Self, ConfigError> {
        Ok(Config {
            config_path: config_path.as_ref().to_path_buf(),
            secrets: SecretStorage::Keyring {
                service: service.to_string(),
            },
            guard: Mutex::new(()),
            secrets_cache: Mutex::new(None),
            secrets_read: Mutex::new(()),
            values_cache: Mutex::new(None),
            values_read: Mutex::new(()),
            last_write_error: Mutex::new(None),
            #[cfg(test)]
            test_keyring_store: None,
            #[cfg(test)]
            io_faults: IoFaults::default(),
            #[cfg(test)]
            io_probe: IoProbe::default(),
        })
    }

    /// Create a new configuration instance with custom paths
    ///
    /// This is primarily useful for testing or for applications that need
    /// to manage multiple configuration files.
    pub fn new_with_file_secrets<P1: AsRef<Path>, P2: AsRef<Path>>(
        config_path: P1,
        secrets_path: P2,
    ) -> Result<Self, ConfigError> {
        Ok(Config {
            config_path: config_path.as_ref().to_path_buf(),
            secrets: SecretStorage::File {
                path: secrets_path.as_ref().to_path_buf(),
            },
            guard: Mutex::new(()),
            secrets_cache: Mutex::new(None),
            secrets_read: Mutex::new(()),
            values_cache: Mutex::new(None),
            values_read: Mutex::new(()),
            last_write_error: Mutex::new(None),
            #[cfg(test)]
            test_keyring_store: None,
            #[cfg(test)]
            io_faults: IoFaults::default(),
            #[cfg(test)]
            io_probe: IoProbe::default(),
        })
    }

    pub fn exists(&self) -> bool {
        self.config_path.exists()
    }

    pub fn clear(&self) -> Result<(), ConfigError> {
        let removed = std::fs::remove_file(&self.config_path);
        // The only write path that does not go through `save_values`, so it is
        // the only one that has to say so itself. Unconditional: a removal that
        // reports an error may still have happened.
        self.invalidate_values_cache();
        Ok(removed?)
    }

    pub fn path(&self) -> String {
        self.config_path.to_string_lossy().to_string()
    }

    /// The config file's contents, parsed once and reused until the file moves.
    ///
    /// ⚠ **This is the hot path of the whole config layer.** Every
    /// [`Self::get_param`] lands here, and `get_param` is how
    /// `ModelConfig::new` resolves `max_tokens`, how the privacy tier resolver
    /// reads its five capability keys, how the extension map is rebuilt, and
    /// how the agent's turn loop reads its settings — so before this it was one
    /// `read_to_string` plus a full YAML parse per setting per lookup.
    ///
    /// Freshness is a `stat`, not a re-read: an external write (the CLI,
    /// `biorouter configure`, a hand edit) moves the file's [`FileStamp`] and
    /// the next lookup misses. Writes made *through this process* do not depend
    /// on that at all — [`Self::save_values`] invalidates explicitly.
    ///
    /// Shared rather than cloned because the mapping is the whole config file,
    /// and `get_param` wants one key out of it.
    fn load_shared(&self) -> Result<Arc<Mapping>, ConfigError> {
        if let Some(hit) = self.cached_if_fresh(FileStamp::of(&self.config_path)) {
            return hit;
        }

        // Cold. Exactly one caller performs the load; the rest queue here and
        // take what the winner leaves behind. The re-check inside the gate is
        // what makes that true, and on a MISSING config file it is what stops
        // the storm: without it every queued caller would go on to run the
        // create branch, and one needed file creation becomes N replacements of
        // it — the shape that failed `test (windows-latest)` in #197.
        let _single_flight = self.values_read.lock().unwrap_or_else(|e| e.into_inner());

        // Sampled again, now that it is our turn. The caller ahead of us may
        // have just CREATED the file, and the stamp we took before queueing
        // describes a state that is already gone — re-checking against it would
        // miss a cache entry that answers our question exactly.
        //
        // ⚠ Before the load and never after it. A writer landing between this
        // sample and the read leaves the entry filed under a stamp that is
        // OLDER than its contents, which costs a miss on the next lookup; filed
        // under a newer one it would serve the previous contents under the new
        // content's identity, and go on doing so.
        let stamp = FileStamp::of(&self.config_path);
        if let Some(hit) = self.cached_if_fresh(stamp) {
            return hit;
        }

        let loaded = self.load_uncached();
        let outcome = match &loaded {
            Ok(values) => Ok(Arc::new(values.clone())),
            // Only a read failure is remembered. Everything else `load` can
            // produce is either impossible here or a claim about a value rather
            // than about the file, and caching a claim is how a transient
            // becomes permanent.
            Err(ConfigError::FileError(err)) => Err(CachedFileError {
                kind: err.kind(),
                message: err.to_string(),
            }),
            Err(_) => return loaded.map(Arc::new),
        };
        let shared = match &outcome {
            Ok(values) => Ok(Arc::clone(values)),
            Err(cached) => Err(cached.rebuild()),
        };

        // ⚠ A load that CHANGED the file files nothing. `load_uncached` writes
        // — it creates a missing config, restores a backup over one, and
        // replaces one that will not parse — so its result describes the state
        // it produced, not the state it was handed, and storing it under the
        // latter would answer for a file that no longer looks like that. The
        // commonest instance is also the most obviously wrong one: the create
        // branch would otherwise cache "the file is absent, the values are the
        // default" *after* making the file exist.
        if FileStamp::of(&self.config_path) == stamp {
            *self.values_cache.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(CachedConfig { stamp, outcome });
        }
        shared
    }

    /// The cached outcome, if it was produced by the file state in `stamp`.
    fn cached_if_fresh(
        &self,
        stamp: Option<FileStamp>,
    ) -> Option<Result<Arc<Mapping>, ConfigError>> {
        let cache = self.values_cache.lock().unwrap_or_else(|e| e.into_inner());
        let cached = cache.as_ref()?;
        if cached.stamp != stamp {
            return None;
        }
        Some(match &cached.outcome {
            Ok(values) => Ok(Arc::clone(values)),
            Err(err) => Err(err.rebuild()),
        })
    }

    /// Drop the parsed-config cache so the next read consults the disk.
    ///
    /// Every write this process makes goes through [`Self::save_values`], which
    /// calls this — so the only callers that need it are the ones that change
    /// the file some other way ([`Self::clear`]) and the ones that deliberately
    /// want to re-derive from disk (`POST /config/recover`).
    pub fn invalidate_values_cache(&self) {
        *self.values_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The most recent failure to write a **default** config file, if any.
    ///
    /// Reported here rather than returned, because [`Self::load`] must keep
    /// answering: a config directory that cannot be written is an environment
    /// problem, and turning it into a hard failure would break start-up on the
    /// exact path #188 and #197 spent two PRs making survivable. What was wrong
    /// was that it was *silent* — the write error was logged per attempt at
    /// best and otherwise discarded, so an install running entirely on
    /// in-memory defaults looked identical to a healthy one.
    pub fn last_write_error(&self) -> Option<String> {
        self.last_write_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Record a failed default-config write, logging the first one loudly.
    ///
    /// Loud once, not per call: before the cache above this was reached on
    /// every `get_param`, and an error line per settings lookup is noise that
    /// buries itself.
    fn record_default_config_write_error(&self, error: &ConfigError) {
        let message = error.to_string();
        let mut slot = self
            .last_write_error
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            tracing::error!(
                "Failed to write default config file to {}: {}. Biorouter will run on \
                 in-memory defaults; settings changed in this session will not persist.",
                self.config_path.display(),
                message
            );
        } else {
            tracing::debug!("Failed to write default config file again: {}", message);
        }
        *slot = Some(message);
    }

    fn load(&self) -> Result<Mapping, ConfigError> {
        Ok((*self.load_shared()?).clone())
    }

    fn load_uncached(&self) -> Result<Mapping, ConfigError> {
        if self.config_path.exists() {
            match self.load_values_with_recovery() {
                Ok(values) => return Ok(values),
                // The file was there for the existence check and gone by the
                // time the read reached it: another writer is installing its
                // own copy right now. That is the missing-config case below,
                // not a failure to report to a caller who only asked for a key.
                Err(ConfigError::FileError(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                    tracing::debug!(
                        "config file disappeared between the existence check and the read; \
                         treating it as missing"
                    );
                }
                Err(other) => return Err(other),
            }
        }

        // Config file doesn't exist, try to recover from backup first
        tracing::info!("Config file doesn't exist, attempting recovery from backup");

        if let Ok(backup_values) = self.try_restore_from_backup() {
            tracing::info!("Successfully restored config from backup");
            return Ok(backup_values);
        }

        // No backup available, create a default config
        tracing::info!("No backup found, creating default configuration");

        // Try to load from init-config.yaml if it exists, otherwise use empty config
        let default_config = self.load_init_config_if_exists().unwrap_or_default();

        self.create_default_config_if_missing(default_config)
    }

    /// Read the config file, tolerating a name another writer is replacing.
    ///
    /// Every read of `config.yaml` on the load path goes through here. See
    /// [`is_transiently_unavailable`] for what it is tolerating and why an
    /// unmediated `read_to_string` is not enough on Windows.
    fn read_config_file(&self) -> std::io::Result<String> {
        #[cfg(test)]
        let mut faults = self.io_faults.failing_reads();
        retry_while_transiently_unavailable(|| {
            #[cfg(test)]
            {
                self.io_probe.note_read_attempt();
                if faults > 0 {
                    faults -= 1;
                    return Err(IoFaults::access_denied());
                }
            }
            std::fs::read_to_string(&self.config_path)
        })
    }

    /// The same tolerance for a file that is not `config.yaml` (a backup).
    fn read_file_tolerantly(path: &Path) -> std::io::Result<String> {
        retry_while_transiently_unavailable(|| std::fs::read_to_string(path))
    }

    /// Move a staged write onto the config file, tolerating the failure a
    /// concurrent replacement of the same name produces.
    fn install_staged_config(&self, staged: &Path) -> std::io::Result<()> {
        #[cfg(test)]
        let mut faults = self.io_faults.failing_renames();
        retry_while_transiently_unavailable(|| {
            #[cfg(test)]
            {
                if faults > 0 {
                    faults -= 1;
                    self.io_faults.land_sibling_config(&self.config_path);
                    return Err(IoFaults::access_denied());
                }
            }
            std::fs::rename(staged, &self.config_path)
        })
    }

    /// The config file's contents, if it is there and parses.
    ///
    /// `None` covers "not there" and "there but unreadable or unparseable"
    /// alike, because every caller uses it to answer one question — *did
    /// somebody else already create this file?* — for which those two are the
    /// same answer.
    fn adopt_existing_config(&self) -> Option<Mapping> {
        if !self.config_path.exists() {
            return None;
        }
        parse_yaml_content(&self.read_config_file().ok()?).ok()
    }

    /// Install `default_config` as the config file **only if nobody else has**.
    ///
    /// Every thread that reaches [`Self::load`] while `config.yaml` does not
    /// exist arrives here, and at start-up that is all of them at once. Each
    /// one used to stage its own copy of the same default and `rename` it over
    /// whatever was there — so a directory needing one file created got N
    /// replacements of it instead.
    ///
    /// On unix that is merely wasteful. On Windows a replacement makes the
    /// destination's NAME briefly unopenable, so the readers racing those
    /// replacements got `ERROR_ACCESS_DENIED` — which is how
    /// `test (windows-latest)` came to fail
    /// `a_startup_storm_on_a_missing_config_never_reports_anything_but_not_found`
    /// with "Access is denied." on a key nobody had set.
    ///
    /// The re-check is not a lock and does not need to be. The window it leaves
    /// open is closed by the second arm: a write that loses to a sibling is
    /// answered by reading what the sibling wrote, never by reporting failure.
    fn create_default_config_if_missing(
        &self,
        default_config: Mapping,
    ) -> Result<Mapping, ConfigError> {
        if let Some(existing) = self.adopt_existing_config() {
            tracing::debug!("Config file was created by another writer first; adopting it");
            return Ok(existing);
        }

        match self.save_values(default_config.clone()) {
            Ok(()) => {
                Self::log_default_config_created(&default_config);
                Ok(default_config)
            }
            Err(write_error) => {
                if let Some(existing) = self.adopt_existing_config() {
                    tracing::info!(
                        "Could not write the default config ({}), but another writer \
                         installed one; adopting it",
                        write_error
                    );
                    return Ok(existing);
                }
                // Nobody else installed one either, so this is a real failure to
                // write — not a lost race — and it is the only arm of this
                // function that should say so. Recorded rather than returned:
                // see `last_write_error`.
                self.record_default_config_write_error(&write_error);
                // Even if we can't write to disk, return config so app can still run
                Ok(default_config)
            }
        }
    }

    fn log_default_config_created(default_config: &Mapping) {
        if default_config.is_empty() {
            tracing::info!("Created fresh empty config file");
        } else {
            tracing::info!(
                "Created fresh config file from init-config.yaml with {} keys",
                default_config.len()
            );
        }
    }

    pub fn all_values(&self) -> Result<HashMap<String, Value>, ConfigError> {
        self.load_shared().map(|m| {
            HashMap::from_iter(m.iter().filter_map(|(k, v)| {
                k.as_str()
                    .map(|k| k.to_string())
                    .zip(serde_json::to_value(v).ok())
            }))
        })
    }

    /// Overwrite a config file that could not be used with a fresh default.
    ///
    /// ⚠ Unlike [`Self::create_default_config_if_missing`] this **replaces**
    /// what is on disk, so it belongs only to the recovery path below, where
    /// the file exists, does not parse, and no backup could be restored. Using
    /// it for a config that is merely *absent* is what made every thread in a
    /// start-up storm a writer.
    fn replace_unusable_config_with_default(
        &self,
        default_config: Mapping,
    ) -> Result<Mapping, ConfigError> {
        // Try to write the default config to disk
        match self.save_values(default_config.clone()) {
            Ok(_) => {
                Self::log_default_config_created(&default_config);
                Ok(default_config)
            }
            Err(write_error) => {
                // Unconditional here, unlike the create path above: this is
                // reached only when the file EXISTS, does not parse, and no
                // backup could be restored, so there is no sibling whose write
                // we could be losing to — a failure here is always a failure.
                self.record_default_config_write_error(&write_error);
                // Even if we can't write to disk, return config so app can still run
                Ok(default_config)
            }
        }
    }

    fn load_values_with_recovery(&self) -> Result<Mapping, ConfigError> {
        let file_content = self.read_config_file()?;

        match parse_yaml_content(&file_content) {
            Ok(values) => Ok(values),
            Err(parse_error) => {
                tracing::warn!(
                    "Config file appears corrupted, attempting recovery: {}",
                    parse_error
                );

                // Try to recover from backup
                if let Ok(backup_values) = self.try_restore_from_backup() {
                    tracing::info!("Successfully restored config from backup");
                    return Ok(backup_values);
                }

                // Last resort: create a fresh default config file
                tracing::error!("Could not recover config file, creating fresh default configuration. Original error: {}", parse_error);

                let default_config = self.load_init_config_if_exists().unwrap_or_default();

                self.replace_unusable_config_with_default(default_config)
            }
        }
    }

    fn try_restore_from_backup(&self) -> Result<Mapping, ConfigError> {
        let backup_paths = self.get_backup_paths();

        for backup_path in backup_paths {
            if backup_path.exists() {
                match Self::read_file_tolerantly(&backup_path) {
                    Ok(backup_content) => {
                        match parse_yaml_content(&backup_content) {
                            Ok(values) => {
                                // Successfully parsed backup, restore it as the main config
                                if let Err(e) = self.save_values(values.clone()) {
                                    tracing::warn!(
                                        "Failed to restore backup as main config: {}",
                                        e
                                    );
                                } else {
                                    tracing::info!(
                                        "Restored config from backup: {:?}",
                                        backup_path
                                    );
                                }
                                return Ok(values);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Backup file {:?} is also corrupted: {}",
                                    backup_path,
                                    e
                                );
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Could not read backup file {:?}: {}", backup_path, e);
                        continue;
                    }
                }
            }
        }

        Err(ConfigError::NotFound("No valid backup found".to_string()))
    }

    // Get list of backup file paths in order of preference
    fn get_backup_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Primary backup (created by backup_config endpoint)
        if let Some(file_name) = self.config_path.file_name() {
            let mut backup_name = file_name.to_os_string();
            backup_name.push(".bak");
            paths.push(self.config_path.with_file_name(backup_name));
        }

        // Timestamped backups
        for i in 1..=5 {
            if let Some(file_name) = self.config_path.file_name() {
                let mut backup_name = file_name.to_os_string();
                backup_name.push(format!(".bak.{}", i));
                paths.push(self.config_path.with_file_name(backup_name));
            }
        }

        paths
    }

    fn load_init_config_if_exists(&self) -> Result<Mapping, ConfigError> {
        load_init_config_from_workspace()
    }

    /// A staging path for one write, unique to this call.
    ///
    /// ⚠ **Never a fixed name.** It was `config.tmp` — one path shared by every
    /// writer in every process — and that is a correctness bug, not untidiness,
    /// because `save_values` holds an **exclusive lock** on whatever it opens
    /// there. Two concurrent writers interleave like this:
    ///
    /// 1. A and B both open the shared `config.tmp`; A wins the lock.
    /// 2. A writes, closes, and renames `config.tmp` onto `config.yaml`.
    /// 3. B's handle is still open on that same file object — which is now
    ///    `config.yaml` — and B's `lock_exclusive` succeeds on it.
    ///
    /// So a writer ends up holding an exclusive lock on the live config file.
    /// On unix `fs2` uses `flock`, which is advisory, and readers never notice.
    /// On Windows it is `LockFileEx`, which is **mandatory**: a concurrent
    /// `read_to_string` of that file fails with `ERROR_LOCK_VIOLATION`. That is
    /// how a `ModelConfig::new` in an unrelated test came back `Err` and
    /// panicked `test (windows-latest)` twice in one day — always in the first
    /// ~100 ms of the job's first test binary, the one window in which
    /// `config.yaml` does not exist yet and every thread races to create it.
    /// See the long note above `validate_max_tokens` in `model.rs`.
    ///
    /// Per process AND per call: the pid separates the daemon, the CLI and the
    /// Electron host (which `privacy::master_switch` already had to reason
    /// about), and the counter separates threads inside one of them.
    fn staging_path(&self) -> PathBuf {
        static NEXT_STAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nth = NEXT_STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut name = self
            .config_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("config.yaml"))
            .to_os_string();
        name.push(format!(".{}.{nth}.tmp", std::process::id()));
        self.config_path.with_file_name(name)
    }

    fn save_values(&self, values: Mapping) -> Result<(), ConfigError> {
        #[cfg(test)]
        self.io_probe.note_config_write();

        // Create backup before writing new config
        self.create_backup_if_needed()?;

        // Convert to YAML for storage
        let yaml_value = serde_yaml::to_string(&values)?;

        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::DirectoryError(e.to_string()))?;
        }

        // Write to a temporary file first for atomic operation
        let temp_path = self.staging_path();

        let staged = (|| -> Result<(), ConfigError> {
            {
                // Retried for the same reason the rename below is: a virus
                // scanner or indexer holding a freshly created file open makes
                // this fail with the same transient "Access is denied." on
                // Windows, and the staging name is ours alone, so a failure
                // here is never contention with another Biorouter writer.
                let mut file = retry_while_transiently_unavailable(|| {
                    OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&temp_path)
                })?;

                // Acquire an exclusive lock
                file.lock_exclusive()
                    .map_err(|e| ConfigError::LockError(e.to_string()))?;

                // Write the contents using the same file handle
                file.write_all(yaml_value.as_bytes())?;
                file.sync_all()?;

                // Unlock is handled automatically when file is dropped
            }

            // Replace the original file. Atomic on unix; on Windows the
            // destination NAME is briefly unopenable while it happens, and a
            // second writer landing in that window is answered "Access is
            // denied." rather than made to wait — hence the retry.
            self.install_staged_config(&temp_path)?;
            Ok(())
        })();

        // A per-call staging path cannot be reused by the next attempt, so a
        // failure that leaves it behind leaves litter in the user's config
        // directory forever. Removing it is best-effort: the write already
        // failed and that is the error worth reporting.
        if staged.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }

        // Every write this process makes passes through here, which is what
        // lets the read path treat its cache as fresh until the file's stamp
        // moves — an mtime has a resolution, and two writes inside one tick
        // would otherwise be one write as far as a reader could tell.
        //
        // Unconditional, including on failure: the point of invalidating is to
        // stop asserting what is on disk, and a write that reports an error is
        // precisely the case where this process is least sure.
        //
        // ⚠ Invalidation belongs HERE and not in `set_param`. Four of the seven
        // callers of this function do not hold `guard`, and one of them is the
        // create branch of `load` itself.
        self.invalidate_values_cache();

        staged
    }

    pub fn initialize_if_empty(&self, values: Mapping) -> Result<(), ConfigError> {
        let _guard = self.guard.lock().unwrap();
        if !self.exists() {
            self.save_values(values)
        } else {
            Ok(())
        }
    }

    // Create backup of current config file if it exists and is valid
    fn create_backup_if_needed(&self) -> Result<(), ConfigError> {
        if !self.config_path.exists() {
            return Ok(());
        }

        // Check if current config is valid before backing it up
        let current_content = match Self::read_file_tolerantly(&self.config_path) {
            Ok(content) => content,
            // Gone between the existence check above and this read: another
            // writer is replacing it. There is nothing to back up, and nothing
            // wrong — failing here would fail the caller's `set_param`.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        if parse_yaml_content(&current_content).is_err() {
            // Don't back up corrupted files
            return Ok(());
        }

        // Rotate existing backups
        self.rotate_backups()?;

        // Create new backup
        if let Some(file_name) = self.config_path.file_name() {
            let mut backup_name = file_name.to_os_string();
            backup_name.push(".bak");
            let backup_path = self.config_path.with_file_name(backup_name);

            if let Err(e) = std::fs::copy(&self.config_path, &backup_path) {
                tracing::warn!("Failed to create config backup: {}", e);
                // Don't fail the entire operation if backup fails
            } else {
                tracing::debug!("Created config backup: {:?}", backup_path);
            }
        }

        Ok(())
    }

    // Rotate backup files to keep the most recent ones
    fn rotate_backups(&self) -> Result<(), ConfigError> {
        if let Some(file_name) = self.config_path.file_name() {
            // Move .bak.4 to .bak.5, .bak.3 to .bak.4, etc.
            for i in (1..5).rev() {
                let mut current_backup = file_name.to_os_string();
                current_backup.push(format!(".bak.{}", i));
                let current_path = self.config_path.with_file_name(&current_backup);

                let mut next_backup = file_name.to_os_string();
                next_backup.push(format!(".bak.{}", i + 1));
                let next_path = self.config_path.with_file_name(&next_backup);

                if current_path.exists() {
                    let _ = std::fs::rename(&current_path, &next_path);
                }
            }

            // Move .bak to .bak.1
            let mut backup_name = file_name.to_os_string();
            backup_name.push(".bak");
            let backup_path = self.config_path.with_file_name(&backup_name);

            if backup_path.exists() {
                let mut backup_1_name = file_name.to_os_string();
                backup_1_name.push(".bak.1");
                let backup_1_path = self.config_path.with_file_name(&backup_1_name);
                let _ = std::fs::rename(&backup_path, &backup_1_path);
            }
        }

        Ok(())
    }

    pub fn all_secrets(&self) -> Result<HashMap<String, Value>, ConfigError> {
        // Plaintext secrets.yaml stays uncached so hand edits are picked up
        // immediately; reading it is cheap and never prompts.
        if matches!(self.secrets, SecretStorage::File { .. }) {
            return self.read_all_secrets_uncached();
        }

        if let Some(cached) = self.secrets_cache.lock().unwrap().clone() {
            return Ok(cached);
        }
        // Cold. Exactly one caller performs the store read; the rest queue here
        // and take the cache the winner leaves behind. The re-check inside the
        // gate is what makes that true -- without it every queued caller would
        // proceed to read once it acquired the gate, serialising the storm
        // rather than collapsing it.
        let _single_flight = self.secrets_read.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = self.secrets_cache.lock().unwrap().clone() {
            return Ok(cached);
        }
        let values = self.read_all_secrets_uncached()?;
        *self.secrets_cache.lock().unwrap() = Some(values.clone());
        Ok(values)
    }

    /// Drop the in-memory secrets cache so the next access re-reads the
    /// backing store (e.g. after another process is known to have written).
    pub fn invalidate_secrets_cache(&self) {
        *self.secrets_cache.lock().unwrap() = None;
    }

    fn read_all_secrets_uncached(&self) -> Result<HashMap<String, Value>, ConfigError> {
        match &self.secrets {
            SecretStorage::Keyring { service } => {
                let result =
                    self.handle_keyring_operation(|| self.keyring_read_blob(service), None);

                match result {
                    Ok(content) => {
                        let values: HashMap<String, Value> = serde_json::from_str(&content)?;
                        Ok(values)
                    }
                    Err(ConfigError::FallbackToFileStorage) => self.fallback_to_file_storage(),
                    Err(ConfigError::KeyringError(msg))
                        if msg.contains("No entry found")
                            || msg.contains("No matching entry found") =>
                    {
                        Ok(HashMap::new())
                    }
                    Err(e) => Err(e),
                }
            }
            SecretStorage::File { path } => self.read_secrets_from_file(path),
        }
    }

    /// Parse an environment variable value into a JSON Value.
    ///
    /// This function tries to intelligently parse environment variable values:
    /// 1. First attempts JSON parsing (for structured data)
    /// 2. If that fails, tries primitive type parsing for common cases
    /// 3. Falls back to string if nothing else works
    fn parse_env_value(val: &str) -> Result<Value, ConfigError> {
        // First try JSON parsing - this handles quoted strings, objects, arrays, etc.
        if let Ok(json_value) = serde_json::from_str(val) {
            return Ok(json_value);
        }

        let trimmed = val.trim();

        match trimmed.to_lowercase().as_str() {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            _ => {}
        }

        if let Ok(int_val) = trimmed.parse::<i64>() {
            return Ok(Value::Number(int_val.into()));
        }

        if let Ok(float_val) = trimmed.parse::<f64>() {
            if let Some(num) = serde_json::Number::from_f64(float_val) {
                return Ok(Value::Number(num));
            }
        }

        Ok(Value::String(val.to_string()))
    }

    // check all possible places for a parameter
    pub fn get(&self, key: &str, is_secret: bool) -> Result<Value, ConfigError> {
        if is_secret {
            self.get_secret(key)
        } else {
            self.get_param(key)
        }
    }

    // save a parameter in the appropriate location based on if it's secret or not
    pub fn set<V>(&self, key: &str, value: &V, is_secret: bool) -> Result<(), ConfigError>
    where
        V: Serialize,
    {
        if is_secret {
            self.set_secret(key, value)
        } else {
            self.set_param(key, value)
        }
    }

    /// Get a configuration value (non-secret).
    ///
    /// This will attempt to get the value from:
    /// 1. Environment variable with the exact key name
    /// 2. Configuration file
    ///
    /// The value will be deserialized into the requested type. This works with
    /// both simple types (String, i32, etc.) and complex types that implement
    /// serde::Deserialize.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - The key doesn't exist in either environment or config file
    /// - The value cannot be deserialized into the requested type
    /// - There is an error reading the config file
    ///
    /// ⚠ **Issue #56: `BIOROUTER_PRIVACY_TIERS` must never be read through this
    /// function**, and there is no exception list here because the rule is
    /// enforced from the other end — [`crate::privacy::load_privacy_tiers_from_config`]
    /// reads [`crate::privacy::master_switch`]'s own record instead, and a Task
    /// 30 gate greps this function's name out of it. The middle branch below
    /// resolves an environment variable, and the agent holds `developer__shell`:
    /// if the master privacy switch were env-readable then
    /// `BIOROUTER_PRIVACY_TIERS=off biorouterd`, or one line in the user's shell
    /// profile, would be a one-token disable of the control the agent is subject
    /// to.
    ///
    /// The single surviving read of that key out of `config.yaml` — Task 42's
    /// one-time migration — reads [`Self::all_values`] for the same reason, so
    /// the environment cannot reach it on the one start-up where it still
    /// matters.
    pub fn get_param<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<T, ConfigError> {
        let env_key = key.to_uppercase();
        // A task-local override (set during provider auto-detection) wins over
        // both env and the config file, but only within the probing task.
        if let Some(val) = override_lookup(&env_key) {
            let value = Self::parse_env_value(&val)?;
            return Ok(serde_json::from_value(value)?);
        }
        if let Ok(val) = env::var(&env_key) {
            let value = Self::parse_env_value(&val)?;
            return Ok(serde_json::from_value(value)?);
        }

        // Shared, not cloned: this reads ONE key, and the mapping is the whole
        // config file.
        let values = self.load_shared()?;
        let raw = values
            .get(key)
            .ok_or_else(|| ConfigError::NotFound(key.to_string()))?;

        match serde_yaml::from_value::<T>(raw.clone()) {
            Ok(parsed) => Ok(parsed),
            // The env branch above repairs `"11543"` into a number before
            // deserializing; this branch did not, and the asymmetry was a silent
            // data-loss bug. The settings UI writes every field as a quoted
            // string, so `LLAMACPP_PORT: '11543'`, `LLAMACPP_CONTEXT_SIZE: '0'`
            // and `LLAMACPP_ENABLE_THINKING: 'false'` all land quoted; a typed
            // `get_param::<usize>()` then failed and every caller swallowed the
            // error with `.ok()` or `.unwrap_or(default)`. The setting appeared
            // to save and did nothing.
            //
            // Repairing on READ (rather than only on write) is what heals the
            // configs already on disk — a write-side fix alone leaves them inert
            // until the user retypes each field.
            //
            // Deliberately only on the failure path: a key that already
            // deserializes correctly never reaches here, so a string key whose
            // value merely looks numeric (`AWS_PROFILE: '12345'`) still parses
            // as a string on the first attempt and is untouched.
            Err(direct_err) => {
                let Some(text) = raw.as_str() else {
                    return Err(direct_err.into());
                };
                let coerced = Self::parse_env_value(text)?;
                serde_json::from_value(coerced).map_err(|_| direct_err.into())
            }
        }
    }

    /// Set a configuration value in the config file (non-secret).
    ///
    /// This will immediately write the value to the config file. The value
    /// can be any type that can be serialized to JSON/YAML.
    ///
    /// Note that this does not affect environment variables - those can only
    /// be set through the system environment.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - There is an error reading or writing the config file
    /// - There is an error serializing the value
    pub fn set_param<V: Serialize>(&self, key: &str, value: V) -> Result<(), ConfigError> {
        let _guard = self.guard.lock().unwrap();
        let mut values = self.load()?;
        values.insert(serde_yaml::to_value(key)?, serde_yaml::to_value(value)?);
        self.save_values(values)
    }

    /// Atomically read, mutate, and persist one non-secret configuration value
    /// with respect to every writer in this process.
    pub(crate) fn update_param<T, R, F>(&self, key: &str, update: F) -> Result<R, ConfigError>
    where
        T: for<'de> Deserialize<'de> + Serialize + Default,
        F: FnOnce(&mut T) -> R,
    {
        let _guard = self.guard.lock().unwrap();
        let mut values = self.load()?;
        let mut value = values
            .get(key)
            .cloned()
            .map(serde_yaml::from_value::<T>)
            .transpose()?
            .unwrap_or_default();
        let result = update(&mut value);
        values.insert(serde_yaml::to_value(key)?, serde_yaml::to_value(value)?);
        self.save_values(values)?;
        Ok(result)
    }

    /// Delete a configuration value in the config file.
    ///
    /// This will immediately write the value to the config file. The value
    /// can be any type that can be serialized to JSON/YAML.
    ///
    /// Note that this does not affect environment variables - those can only
    /// be set through the system environment.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - There is an error reading or writing the config file
    /// - There is an error serializing the value
    pub fn delete(&self, key: &str) -> Result<(), ConfigError> {
        // Lock before reading to prevent race condition.
        let _guard = self.guard.lock().unwrap();

        let mut values = self.load()?;
        values.shift_remove(key);

        self.save_values(values)
    }

    /// Get a secret value.
    ///
    /// This will attempt to get the value from:
    /// 1. Environment variable with the exact key name
    /// 2. System keyring
    ///
    /// The value will be deserialized into the requested type. This works with
    /// both simple types (String, i32, etc.) and complex types that implement
    /// serde::Deserialize.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - The key doesn't exist in either environment or keyring
    /// - The value cannot be deserialized into the requested type
    /// - There is an error accessing the keyring
    pub fn get_secret<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<T, ConfigError> {
        let env_key = key.to_uppercase();
        // A task-local override (set during provider auto-detection) wins over
        // both env and the keyring, but only within the probing task — see
        // `with_config_overrides`. This avoids mutating the process environment
        // to test a candidate credential.
        if let Some(val) = override_lookup(&env_key) {
            let value = Self::parse_env_value(&val)?;
            return Ok(serde_json::from_value(value)?);
        }
        // First check environment variables (convert to uppercase)
        if let Ok(val) = env::var(&env_key) {
            let value = Self::parse_env_value(&val)?;
            return Ok(serde_json::from_value(value)?);
        }

        // Then check keyring
        let values = self.all_secrets()?;
        values
            .get(key)
            .ok_or_else(|| ConfigError::NotFound(key.to_string()))
            .and_then(|v| Ok(serde_json::from_value(v.clone())?))
    }

    /// Get secrets. If primary is in env, use env for all keys. Otherwise use secret storage.
    pub fn get_secrets(
        &self,
        primary: &str,
        maybe_secret: &[&str],
    ) -> Result<HashMap<String, String>, ConfigError> {
        let primary_env_key = primary.to_uppercase();
        let use_overrides = override_lookup(&primary_env_key).is_some();
        let use_env = !use_overrides && env::var(&primary_env_key).is_ok();
        let get_value = |key: &str| -> Result<String, ConfigError> {
            let env_key = key.to_uppercase();
            if use_overrides {
                override_lookup(&env_key)
                    .or_else(|| env::var(&env_key).ok())
                    .ok_or_else(|| ConfigError::NotFound(key.to_string()))
            } else if use_env {
                env::var(env_key).map_err(|_| ConfigError::NotFound(key.to_string()))
            } else {
                self.get_secret(key)
            }
        };

        let mut result = HashMap::new();
        result.insert(primary.to_string(), get_value(primary)?);
        for &key in maybe_secret {
            if let Ok(v) = get_value(key) {
                result.insert(key.to_string(), v);
            }
        }
        Ok(result)
    }

    /// Set a secret value in the system keyring.
    ///
    /// This will store the value in a single JSON object in the system keyring,
    /// alongside any other secrets. The value can be any type that can be
    /// serialized to JSON.
    ///
    /// Note that this does not affect environment variables - those can only
    /// be set through the system environment.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - There is an error accessing the keyring
    /// - There is an error serializing the value
    pub fn set_secret<V>(&self, key: &str, value: &V) -> Result<(), ConfigError>
    where
        V: Serialize,
    {
        // Lock before reading to prevent race condition.
        let _guard = self.guard.lock().unwrap();

        let mut values = self.all_secrets()?;
        values.insert(key.to_string(), serde_json::to_value(value)?);

        self.persist_secrets(&values)
    }

    /// Delete a secret from the system keyring.
    ///
    /// This will remove the specified key from the JSON object in the system keyring.
    /// Other secrets will remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - There is an error accessing the keyring
    /// - There is an error serializing the remaining values
    pub fn delete_secret(&self, key: &str) -> Result<(), ConfigError> {
        // Lock before reading to prevent race condition.
        let _guard = self.guard.lock().unwrap();

        let mut values = self.all_secrets()?;
        values.remove(key);

        self.persist_secrets(&values)
    }

    /// Write the full secrets map to the active backend and keep the
    /// in-memory cache in sync. On success the cache holds the new values;
    /// on any failure (including keyring → file fallback) the cache is
    /// dropped so the next read consults the backing store.
    fn persist_secrets(&self, values: &HashMap<String, Value>) -> Result<(), ConfigError> {
        let result = match &self.secrets {
            SecretStorage::Keyring { service } => {
                let json_value = serde_json::to_string(values)?;
                self.handle_keyring_operation(
                    || self.keyring_write_blob(service, &json_value),
                    Some(values),
                )
            }
            SecretStorage::File { path } => {
                let yaml_value = serde_yaml::to_string(values)?;
                std::fs::write(path, yaml_value)?;
                Ok(())
            }
        };

        match &result {
            Ok(()) => *self.secrets_cache.lock().unwrap() = Some(values.clone()),
            Err(_) => *self.secrets_cache.lock().unwrap() = None,
        }
        result
    }

    /// Read secrets from a YAML file
    fn read_secrets_from_file(&self, path: &Path) -> Result<HashMap<String, Value>, ConfigError> {
        if path.exists() {
            let file_content = std::fs::read_to_string(path)?;
            let yaml_value: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
            let json_value: Value = serde_json::to_value(yaml_value)?;
            match json_value {
                Value::Object(map) => Ok(map.into_iter().collect()),
                _ => Ok(HashMap::new()),
            }
        } else {
            Ok(HashMap::new())
        }
    }

    /// Get the path to the secrets storage file
    fn secrets_file_path() -> PathBuf {
        Paths::config_dir().join("secrets.yaml")
    }

    /// Perform fallback to file storage when keyring is unavailable
    fn fallback_to_file_storage(&self) -> Result<HashMap<String, Value>, ConfigError> {
        let path = Self::secrets_file_path();
        self.read_secrets_from_file(&path)
    }

    /// Write secrets to file storage (used for fallback)
    fn write_secrets_to_file(&self, values: &HashMap<String, Value>) -> Result<(), ConfigError> {
        std::fs::create_dir_all(Paths::config_dir())?;
        let path = Self::secrets_file_path();
        let yaml_value = serde_yaml::to_string(values)?;
        std::fs::write(path, yaml_value)?;
        Ok(())
    }

    /// Check if an error string indicates a keyring availability issue that should trigger fallback
    fn is_keyring_availability_error(&self, error_str: &str) -> bool {
        // keyring::Error renders NoStorageAccess as "Couldn't access platform
        // secure storage: ..." and PlatformFailure as "Platform secure storage
        // failure: ..." — both mean the credential store is unusable (headless
        // Linux without a Secret Service daemon, locked collection, WSL, ...).
        let error_str = error_str.to_lowercase();
        error_str.contains("keyring")
            || error_str.contains("dbus error")
            || error_str.contains("org.freedesktop.secrets")
            || error_str.contains("couldn't access platform secure storage")
            || error_str.contains("platform secure storage failure")
    }

    /// Username for the i-th continuation entry of a chunked secrets blob.
    fn chunk_username(index: usize) -> String {
        format!("{}.{}", KEYRING_USERNAME, index)
    }

    /// Split a blob into pieces small enough for one credential entry,
    /// measured in UTF-16 code units (the unit Windows enforces its
    /// credential blob limit in). Always returns at least one piece.
    fn split_blob_into_chunks(blob: &str, limit: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut current_units = 0usize;
        for ch in blob.chars() {
            let units = ch.len_utf16();
            if current_units + units > limit && !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_units = 0;
            }
            current.push(ch);
            current_units += units;
        }
        if !current.is_empty() || chunks.is_empty() {
            chunks.push(current);
        }
        chunks
    }

    /// Read the secrets blob, reassembling continuation entries if the main
    /// entry holds a chunk-count header.
    fn keyring_read_blob(&self, service: &str) -> Result<String, keyring::Error> {
        #[cfg(test)]
        if let Some(store) = &self.test_keyring_store {
            return Self::read_blob_from(store.as_ref());
        }
        Self::read_blob_from(&OsKeyringStore { service })
    }

    /// Write the secrets blob, splitting it across continuation entries when
    /// it exceeds the per-credential limit (Windows only in practice).
    fn keyring_write_blob(&self, service: &str, blob: &str) -> Result<(), keyring::Error> {
        #[cfg(test)]
        if let Some(store) = &self.test_keyring_store {
            return Self::write_blob_to(store.as_ref(), blob, KEYRING_CHUNK_UTF16_LIMIT);
        }
        Self::write_blob_to(&OsKeyringStore { service }, blob, KEYRING_CHUNK_UTF16_LIMIT)
    }

    fn read_blob_from(store: &dyn KeyringBlobStore) -> Result<String, keyring::Error> {
        let main = store.get(KEYRING_USERNAME)?;
        let Some(count_str) = main.strip_prefix(KEYRING_CHUNK_MARKER) else {
            return Ok(main);
        };
        let count: usize = count_str.trim().parse().map_err(|_| {
            keyring::Error::Invalid("chunk header".to_string(), "not a number".to_string())
        })?;
        let count = count.min(KEYRING_MAX_CHUNKS);
        let mut blob = String::new();
        for i in 1..=count {
            blob.push_str(&store.get(&Self::chunk_username(i))?);
        }
        Ok(blob)
    }

    /// Write `blob` to the store, chunking when it exceeds `limit`, and
    /// remove any stale continuation entries left over from a larger
    /// previous write.
    fn write_blob_to(
        store: &dyn KeyringBlobStore,
        blob: &str,
        limit: usize,
    ) -> Result<(), keyring::Error> {
        let chunks = Self::split_blob_into_chunks(blob, limit);
        let stale_start = if chunks.len() == 1 {
            store.set(KEYRING_USERNAME, blob)?;
            1
        } else {
            for (i, chunk) in chunks.iter().enumerate() {
                store.set(&Self::chunk_username(i + 1), chunk)?;
            }
            // Write the header last so an interrupted write never leaves a
            // header pointing at missing chunks.
            store.set(
                KEYRING_USERNAME,
                &format!("{}{}", KEYRING_CHUNK_MARKER, chunks.len()),
            )?;
            chunks.len() + 1
        };
        for i in stale_start..=KEYRING_MAX_CHUNKS {
            match store.delete(&Self::chunk_username(i)) {
                Ok(()) => continue,
                Err(keyring::Error::NoEntry) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Handle keyring errors with automatic fallback to file storage
    fn handle_keyring_fallback_error<T>(
        &self,
        keyring_err: &keyring::Error,
        fallback_values: Option<&HashMap<String, Value>>,
    ) -> Result<T, ConfigError> {
        if self.is_keyring_availability_error(&keyring_err.to_string()) {
            std::env::set_var("BIOROUTER_DISABLE_KEYRING", "1");
            tracing::warn!("Keyring unavailable. Using file storage for secrets.");

            if let Some(values) = fallback_values {
                self.write_secrets_to_file(values)?;
                Err(ConfigError::FallbackToFileStorage)
            } else {
                Err(ConfigError::FallbackToFileStorage)
            }
        } else {
            Err(ConfigError::KeyringError(keyring_err.to_string()))
        }
    }

    /// Handle keyring operation with automatic fallback to file storage
    fn handle_keyring_operation<T>(
        &self,
        operation: impl FnOnce() -> Result<T, keyring::Error>,
        fallback_values: Option<&HashMap<String, Value>>,
    ) -> Result<T, ConfigError> {
        match operation() {
            Ok(result) => Ok(result),
            Err(keyring_err) => self.handle_keyring_fallback_error(&keyring_err, fallback_values),
        }
    }
}

config_value!(BIOROUTER_SEARCH_PATHS, Vec<String>);
config_value!(BIOROUTER_MODE, BioRouterMode);
config_value!(BIOROUTER_PROVIDER, String);
config_value!(BIOROUTER_MODEL, String);
config_value!(BIOROUTER_MAX_ACTIVE_AGENTS, usize);

/// Load init-config.yaml from workspace root if it exists.
/// This function is shared between the config recovery and the init_config endpoint.
pub fn load_init_config_from_workspace() -> Result<Mapping, ConfigError> {
    let workspace_root = match std::env::current_exe() {
        Ok(mut exe_path) => {
            while let Some(parent) = exe_path.parent() {
                let cargo_toml = parent.join("Cargo.toml");
                if cargo_toml.exists() {
                    if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                        if content.contains("[workspace]") {
                            exe_path = parent.to_path_buf();
                            break;
                        }
                    }
                }
                exe_path = parent.to_path_buf();
            }
            exe_path
        }
        Err(_) => {
            return Err(ConfigError::FileError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine executable path",
            )))
        }
    };

    let init_config_path = workspace_root.join("init-config.yaml");
    if !init_config_path.exists() {
        return Err(ConfigError::NotFound(
            "init-config.yaml not found".to_string(),
        ));
    }

    let init_content = std::fs::read_to_string(&init_config_path)?;
    parse_yaml_content(&init_content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::NamedTempFile;
    #[test]
    fn test_basic_config() -> Result<(), ConfigError> {
        let config = new_test_config();

        // Set a simple string value
        config.set_param("test_key", "test_value")?;

        // Test simple string retrieval
        let value: String = config.get_param("test_key")?;
        assert_eq!(value, "test_value");

        // Test with environment variable override
        std::env::set_var("TEST_KEY", "env_value");
        let value: String = config.get_param("test_key")?;
        assert_eq!(value, "env_value");

        Ok(())
    }

    /// The settings UI writes every provider field as a quoted string, so a
    /// numeric or boolean key lands on disk as `LLAMACPP_PORT: '11543'`. A typed
    /// `get_param::<usize>()` used to fail on that, and every caller swallowed
    /// the error with `.ok()` / `.unwrap_or(default)` — the setting appeared to
    /// save and did nothing. The env-var branch of `get_param` had always
    /// repaired this via `parse_env_value`; only the config-FILE branch did not.
    #[test]
    #[serial]
    fn quoted_scalars_in_the_config_file_still_parse_as_their_real_type() -> Result<(), ConfigError>
    {
        let config = new_test_config();

        // Exactly the four shapes the Llama Server settings pane writes.
        config.set_param("LLAMACPP_PORT", "11543")?;
        config.set_param("LLAMACPP_CONTEXT_SIZE", "0")?;
        config.set_param("LLAMACPP_TIMEOUT", "600")?;
        config.set_param("LLAMACPP_ENABLE_THINKING", "false")?;

        assert_eq!(config.get_param::<usize>("LLAMACPP_PORT")?, 11543);
        assert_eq!(config.get_param::<usize>("LLAMACPP_CONTEXT_SIZE")?, 0);
        assert_eq!(config.get_param::<u64>("LLAMACPP_TIMEOUT")?, 600);
        assert!(!config.get_param::<bool>("LLAMACPP_ENABLE_THINKING")?);

        Ok(())
    }

    /// The repair above must run ONLY on the failure path. A string-typed key
    /// whose value merely looks numeric has to survive as a string — coercing it
    /// would corrupt real settings (an AWS profile named "12345", a numeric
    /// account id, a version string).
    #[test]
    #[serial]
    fn a_string_key_that_looks_numeric_is_not_coerced() -> Result<(), ConfigError> {
        let config = new_test_config();
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let profile_key = format!("BIOROUTER_TEST_NUMERIC_PROFILE_{suffix}");
        let version_key = format!("BIOROUTER_TEST_NUMERIC_VERSION_{suffix}");
        config.set_param(&profile_key, "12345")?;
        config.set_param(&version_key, "2024")?;

        assert_eq!(config.get_param::<String>(&profile_key)?, "12345");
        assert_eq!(config.get_param::<String>(&version_key)?, "2024");
        Ok(())
    }

    /// A genuinely unparseable value must report the ORIGINAL type error, not a
    /// confusing secondary parse failure — the caller needs to know the key was
    /// the wrong type, not that some fallback parser also gave up.
    #[test]
    #[serial]
    fn an_uncoercible_value_still_errors() {
        let config = new_test_config();
        config.set_param("LLAMACPP_PORT", "not-a-port").unwrap();
        assert!(config.get_param::<usize>("LLAMACPP_PORT").is_err());
    }

    #[test]
    fn test_complex_type() -> Result<(), ConfigError> {
        #[derive(Deserialize, Debug, PartialEq)]
        struct TestStruct {
            field1: String,
            field2: i32,
        }

        let config = new_test_config();

        // Set a complex value
        config.set_param(
            "complex_key",
            serde_json::json!({
                "field1": "hello",
                "field2": 42
            }),
        )?;

        let value: TestStruct = config.get_param("complex_key")?;
        assert_eq!(value.field1, "hello");
        assert_eq!(value.field2, 42);

        Ok(())
    }

    #[test]
    fn test_missing_value() {
        let config = new_test_config();

        let result: Result<String, ConfigError> = config.get_param("nonexistent_key");
        assert!(matches!(result, Err(ConfigError::NotFound(_))));
    }

    #[test]
    fn test_yaml_formatting() -> Result<(), ConfigError> {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        let config = Config::new_with_file_secrets(config_file.path(), secrets_file.path())?;

        config.set_param("key1", "value1")?;
        config.set_param("key2", 42)?;

        // Read the file directly to check YAML formatting
        let content = std::fs::read_to_string(config_file.path())?;
        assert!(content.contains("key1: value1"));
        assert!(content.contains("key2: 42"));

        Ok(())
    }

    #[test]
    fn test_value_management() -> Result<(), ConfigError> {
        let config = new_test_config();

        config.set_param("test_key", "test_value")?;
        config.set_param("another_key", 42)?;
        config.set_param("third_key", true)?;

        let _values = config.load()?;

        let result: Result<String, ConfigError> = config.get_param("key");
        assert!(matches!(result, Err(ConfigError::NotFound(_))));

        Ok(())
    }

    #[test]
    fn test_file_based_secrets_management() -> Result<(), ConfigError> {
        let config = new_test_config();

        config.set_secret("key", &"value")?;

        let value: String = config.get_secret("key")?;
        assert_eq!(value, "value");

        config.delete_secret("key")?;

        let result: Result<String, ConfigError> = config.get_secret("key");
        assert!(matches!(result, Err(ConfigError::NotFound(_))));

        Ok(())
    }

    #[test]
    #[serial]
    fn test_secret_management() -> Result<(), ConfigError> {
        let config = new_test_config();

        // Test setting and getting a simple secret
        config.set_secret("api_key", &Value::String("secret123".to_string()))?;
        let value: String = config.get_secret("api_key")?;
        assert_eq!(value, "secret123");

        // Test environment variable override
        std::env::set_var("API_KEY", "env_secret");
        let value: String = config.get_secret("api_key")?;
        assert_eq!(value, "env_secret");
        std::env::remove_var("API_KEY");

        // Test deleting a secret
        config.delete_secret("api_key")?;
        let result: Result<String, ConfigError> = config.get_secret("api_key");
        assert!(matches!(result, Err(ConfigError::NotFound(_))));

        Ok(())
    }

    #[test]
    fn test_multiple_secrets() -> Result<(), ConfigError> {
        let config = new_test_config();

        // Set multiple secrets
        config.set_secret("key1", &Value::String("secret1".to_string()))?;
        config.set_secret("key2", &Value::String("secret2".to_string()))?;

        // Verify both exist
        let value1: String = config.get_secret("key1")?;
        let value2: String = config.get_secret("key2")?;
        assert_eq!(value1, "secret1");
        assert_eq!(value2, "secret2");

        // Delete one secret
        config.delete_secret("key1")?;

        // Verify key1 is gone but key2 remains
        let result1: Result<String, ConfigError> = config.get_secret("key1");
        let value2: String = config.get_secret("key2")?;
        assert!(matches!(result1, Err(ConfigError::NotFound(_))));
        assert_eq!(value2, "secret2");

        Ok(())
    }

    #[test]
    fn test_concurrent_writes() -> Result<(), ConfigError> {
        use std::sync::{Arc, Barrier, Mutex};
        use std::thread;

        let config = Arc::new(new_test_config());
        let barrier = Arc::new(Barrier::new(3)); // For 3 concurrent threads
        let values = Arc::new(Mutex::new(Mapping::new()));
        let mut handles = vec![];

        // Initialize with empty values
        config.save_values(Default::default())?;

        // Spawn 3 threads that will try to write simultaneously
        for i in 0..3 {
            let config = Arc::clone(&config);
            let barrier = Arc::clone(&barrier);
            let values = Arc::clone(&values);
            let handle = thread::spawn(move || -> Result<(), ConfigError> {
                // Wait for all threads to reach this point
                barrier.wait();

                // Get the lock and update values
                let mut values = values.lock().unwrap();
                values.insert(
                    serde_yaml::to_value(format!("key{}", i)).unwrap(),
                    serde_yaml::to_value(format!("value{}", i)).unwrap(),
                );

                // Write all values
                config.save_values(values.clone())?;
                Ok(())
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap()?;
        }

        // Verify all values were written correctly
        let final_values = config.all_values()?;

        // Print the final values for debugging
        println!("Final values: {:?}", final_values);

        assert_eq!(
            final_values.len(),
            3,
            "Expected 3 values, got {}",
            final_values.len()
        );

        for i in 0..3 {
            let key = format!("key{}", i);
            let value = format!("value{}", i);
            assert!(
                final_values.contains_key(&key),
                "Missing key {} in final values",
                key
            );
            assert_eq!(
                final_values.get(&key).unwrap(),
                &Value::String(value),
                "Incorrect value for key {}",
                key
            );
        }

        Ok(())
    }

    #[test]
    fn test_config_recovery_from_backup() -> Result<(), ConfigError> {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        let config = Config::new_with_file_secrets(config_file.path(), secrets_file.path())?;

        // Create a valid config first
        config.set_param("key1", "value1")?;

        // Verify the backup was created by the first write
        let backup_paths = config.get_backup_paths();
        println!("Backup paths: {:?}", backup_paths);
        for (i, path) in backup_paths.iter().enumerate() {
            println!("Backup {} exists: {}", i, path.exists());
        }

        // Make another write to ensure backup is created
        config.set_param("key2", 42)?;

        // Check again
        for (i, path) in backup_paths.iter().enumerate() {
            println!(
                "After second write - Backup {} exists: {}",
                i,
                path.exists()
            );
        }

        // Corrupt the main config file
        std::fs::write(config_file.path(), "invalid: yaml: content: [unclosed")?;

        // Try to load values - should recover from backup
        let recovered_values = config.all_values()?;
        println!("Recovered values: {:?}", recovered_values);

        // Should have recovered the data
        assert!(
            !recovered_values.is_empty(),
            "Should have recovered at least one key"
        );

        Ok(())
    }

    #[test]
    fn test_config_recovery_creates_fresh_file() -> Result<(), ConfigError> {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        let config = Config::new_with_file_secrets(config_file.path(), secrets_file.path())?;

        // Create a corrupted config file with no backup
        std::fs::write(config_file.path(), "invalid: yaml: content: [unclosed")?;

        // Try to load values - should create a fresh default config
        let recovered_values = config.all_values()?;

        // Should return empty config
        assert_eq!(recovered_values.len(), 0);

        // Verify that a clean config file was written to disk
        let file_content = std::fs::read_to_string(config_file.path())?;

        // Should be valid YAML (empty object)
        let parsed: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
        assert!(parsed.is_mapping());

        // Should be able to load it again without issues
        let reloaded_values = config.all_values()?;
        assert_eq!(reloaded_values.len(), 0);

        Ok(())
    }

    #[test]
    fn test_config_file_creation_when_missing() -> Result<(), ConfigError> {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        let config_path = config_file.path().to_path_buf();
        let config = Config::new_with_file_secrets(&config_path, secrets_file.path())?;

        // Delete the file to simulate it not existing
        std::fs::remove_file(&config_path)?;
        assert!(!config_path.exists());

        // Try to load values - should create a fresh default config file
        let values = config.all_values()?;

        // Should return empty config
        assert_eq!(values.len(), 0);

        // Verify that the config file was created
        assert!(config_path.exists());

        // Verify that it's valid YAML
        let file_content = std::fs::read_to_string(&config_path)?;
        let parsed: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
        assert!(parsed.is_mapping());

        // Should be able to load it again without issues
        let reloaded_values = config.all_values()?;
        assert_eq!(reloaded_values.len(), 0);

        Ok(())
    }

    #[test]
    fn test_config_recovery_from_backup_when_missing() -> Result<(), ConfigError> {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        let config_path = config_file.path().to_path_buf();
        let config = Config::new_with_file_secrets(&config_path, secrets_file.path())?;

        // First, create a config with some data
        config.set_param("test_key_backup", "backup_value")?;
        config.set_param("another_key", 42)?;

        // Verify the backup was created
        let backup_paths = config.get_backup_paths();
        let primary_backup = &backup_paths[0]; // .bak file

        // Make sure we have a backup by doing another write
        config.set_param("third_key", true)?;
        assert!(primary_backup.exists(), "Backup should exist after writes");

        // Now delete the main config file to simulate it being lost
        std::fs::remove_file(&config_path)?;
        assert!(!config_path.exists());

        // Try to load values - should recover from backup
        let recovered_values = config.all_values()?;

        // Should have recovered the data from backup
        assert!(
            !recovered_values.is_empty(),
            "Should have recovered data from backup"
        );

        // Verify the main config file was restored
        assert!(config_path.exists(), "Main config file should be restored");

        // Verify we can load the data (using a key that won't conflict with env vars)
        if let Ok(backup_value) = config.get_param::<String>("test_key_backup") {
            // If we recovered the key, great!
            assert_eq!(backup_value, "backup_value");
        }
        // Note: Due to back up rotation, we might not get the exact same data,
        // but we should get some data back

        Ok(())
    }

    #[test]
    fn test_atomic_write_prevents_corruption() -> Result<(), ConfigError> {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        let config = Config::new_with_file_secrets(config_file.path(), secrets_file.path())?;

        // Set initial values
        config.set_param("key1", "value1")?;

        // Verify the config file exists and is valid
        assert!(config_file.path().exists());
        let content = std::fs::read_to_string(config_file.path())?;
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&content).is_ok());

        // No staging file should survive a successful write. Asserting on one
        // fixed name would go vacuous the moment the staging path stopped being
        // a fixed name — which is exactly what it had to stop being — so look
        // for any `.tmp` sibling instead.
        let dir = config_file.path().parent().unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.ends_with(".tmp")
                    && name.starts_with(
                        &config_file
                            .path()
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    )
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging files should be cleaned up, found {leftovers:?}"
        );

        Ok(())
    }

    /// Two writers must never stage through the same path.
    ///
    /// It was `config_path.with_extension("tmp")` — one name for every writer
    /// in every process — and `save_values` holds an **exclusive lock** on
    /// whatever it opens there. Once one writer renames that shared file onto
    /// `config.yaml`, a second writer that already had it open is holding an
    /// exclusive lock on the live config file. Under `flock` (unix) that is
    /// advisory and invisible; under `LockFileEx` (Windows) it is mandatory,
    /// and a concurrent reader gets `ERROR_LOCK_VIOLATION` instead of the file.
    /// That read is how `ModelConfig::new` came back `Err` and panicked
    /// `test (windows-latest)` in tests that never touch the config layer.
    ///
    /// Asserted on the path rather than by racing threads, deliberately: the
    /// race needs an interleaving CI produces and a laptop rarely does, and on
    /// unix it cannot be observed at all. Uniqueness is the property, and it is
    /// checkable everywhere.
    #[test]
    fn two_writes_never_share_a_staging_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let staged = config.staging_path();
            assert_eq!(
                staged.parent(),
                config_path.parent(),
                "staging must be a sibling of the config file or the rename is not atomic"
            );
            assert_ne!(staged, config_path, "staging must not BE the config file");
            assert!(
                staged.to_string_lossy().ends_with(".tmp"),
                "staging paths stay recognisable as staging: {}",
                staged.display()
            );
            assert!(
                seen.insert(staged.clone()),
                "two writes staged through {} — the second one can end up holding an \
                 exclusive lock on config.yaml after the first renames it into place",
                staged.display()
            );
        }
        assert!(
            seen.iter().all(|p| p
                .to_string_lossy()
                .contains(&std::process::id().to_string())),
            "the staging name must also separate PROCESSES: the daemon, the CLI and the \
             Electron host all write this directory"
        );
    }

    /// A reader must never see a config-layer failure that the config layer
    /// itself caused.
    ///
    /// This is the shape the flake took: many threads reach `Config::load` at
    /// once at start-up, `config.yaml` does not exist yet, and every one of
    /// them takes the create-and-save branch. The only answer any of them may
    /// give for a key that is not set is `NotFound` — never an I/O or lock
    /// error, which callers are entitled to read as "the value is broken".
    ///
    /// On unix this passes with or without the fix (`flock` is advisory), so it
    /// is not the fail-before test — `two_writes_never_share_a_staging_path` is.
    /// It is here because it is the assertion that runs on the platform where
    /// the bug actually happens.
    ///
    /// ⚠ It caught a **second** cause on Windows after the first was fixed, and
    /// that is the argument for keeping an assertion whose failure mode is
    /// platform-only: with the staging paths made unique, the storm's remaining
    /// hazard was the *renames themselves*, which make the destination name
    /// briefly unopenable. See
    /// [`Config::create_default_config_if_missing`] for that cause and
    /// `a_startup_storm_survives_the_denials_windows_answers_a_replacement_with`
    /// for the same assertion with the denial injected, which fails everywhere.
    #[test]
    fn a_startup_storm_on_a_missing_config_never_reports_anything_but_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let config = std::sync::Arc::new(
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap(),
        );

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let config = std::sync::Arc::clone(&config);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    (0..8)
                        .map(|_| config.get_param::<i32>("A_KEY_NOBODY_SET"))
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        for worker in workers {
            for outcome in worker.join().unwrap() {
                assert!(
                    matches!(outcome, Err(ConfigError::NotFound(_))),
                    "an unset key must read as NotFound even while other threads are creating \
                     the config file; got {outcome:?}"
                );
            }
        }
    }

    /// The three ways Windows says "this name is being replaced right now",
    /// and the failures that mean something else and must not be retried.
    #[test]
    fn a_name_being_replaced_is_transient_and_a_missing_file_is_not() {
        assert!(
            is_transiently_unavailable(&IoFaults::access_denied()),
            "ERROR_ACCESS_DENIED is what a read of a name mid-replacement gets, and it is \
             the error the Windows CI failure carried"
        );

        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::InvalidData,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            assert!(
                !is_transiently_unavailable(&std::io::Error::new(kind, "settled")),
                "{kind:?} is an answer about the file, not about contention for its name"
            );
        }

        // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION reach Rust with no
        // named ErrorKind, so they are recognised by raw code — and only on the
        // platform those codes belong to.
        for code in [32, 33] {
            assert_eq!(
                is_transiently_unavailable(&std::io::Error::from_raw_os_error(code)),
                cfg!(windows),
                "raw os error {code} is a Windows sharing/lock violation; on unix the same \
                 number means something else entirely"
            );
        }
    }

    /// An operation that fails the way a name mid-replacement fails is retried,
    /// and the caller is told about it only if it never settles.
    ///
    /// Injected, because the failure is Windows-only, sub-millisecond, and
    /// produced by an interleaving of concurrent renames that no test can
    /// arrange — on unix `rename(2)` is atomic and the seam does not exist.
    #[test]
    fn a_read_that_fails_the_way_windows_fails_is_retried_rather_than_reported() {
        let attempts = std::cell::Cell::new(0usize);
        let outcome = retry_while_transiently_unavailable(|| {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 4 {
                return Err(IoFaults::access_denied());
            }
            Ok("BIOROUTER_PROVIDER: versa_azure\n")
        });

        assert_eq!(
            outcome.unwrap(),
            "BIOROUTER_PROVIDER: versa_azure\n",
            "a transient denial must not become the caller's answer"
        );
        assert_eq!(attempts.get(), 4, "the operation must be retried in place");
    }

    #[test]
    fn a_failure_that_is_not_a_replacement_in_progress_is_reported_at_once() {
        let attempts = std::cell::Cell::new(0usize);
        let outcome = retry_while_transiently_unavailable(|| -> std::io::Result<()> {
            attempts.set(attempts.get() + 1);
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such file",
            ))
        });

        assert_eq!(outcome.unwrap_err().kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            attempts.get(),
            1,
            "retrying a settled answer only delays it; the budget is for contention alone"
        );
    }

    #[test]
    fn a_denial_that_never_lifts_is_reported_once_the_budget_is_spent() {
        let attempts = std::cell::Cell::new(0usize);
        let outcome = retry_while_transiently_unavailable(|| -> std::io::Result<()> {
            attempts.set(attempts.get() + 1);
            Err(IoFaults::access_denied())
        });

        assert_eq!(
            outcome.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied,
            "a config the user genuinely cannot read must still say so"
        );
        assert_eq!(attempts.get(), TRANSIENT_IO_ATTEMPTS);
    }

    /// The Windows failure, simulated on every platform: a start-up storm in
    /// which **every** read of the config file is first answered "Access is
    /// denied." three times, exactly as a read racing a sibling's rename is.
    ///
    /// This is the fail-before test for the tolerance half.
    /// [`a_startup_storm_on_a_missing_config_never_reports_anything_but_not_found`]
    /// above is the same assertion against the real filesystem, and it can only
    /// fail on the platform that produces the failure — which is why it took a
    /// CI run to catch, and why this one exists beside it.
    #[test]
    fn a_startup_storm_survives_the_denials_windows_answers_a_replacement_with() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();
        config.io_faults.failing_read_attempts =
            std::sync::atomic::AtomicUsize::new(TRANSIENT_IO_ATTEMPTS / 2);
        let config = std::sync::Arc::new(config);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let config = std::sync::Arc::clone(&config);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    (0..4)
                        .map(|_| config.get_param::<i32>("A_KEY_NOBODY_SET"))
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        for worker in workers {
            for outcome in worker.join().unwrap() {
                assert!(
                    matches!(outcome, Err(ConfigError::NotFound(_))),
                    "an unset key must read as NotFound even while every read of the config \
                     file is being denied the way Windows denies a name mid-replacement; \
                     got {outcome:?}"
                );
            }
        }
    }

    /// Creating a config that is missing must not overwrite a config that is
    /// not.
    ///
    /// This is the fail-before test for the root cause. Every thread in a
    /// start-up storm reaches the create path, and each one used to stage its
    /// own copy of the same default and rename it over whatever had landed
    /// meanwhile — turning one needed file creation into N replacements, each
    /// of which makes the destination name briefly unopenable on Windows.
    #[test]
    fn a_create_that_loses_the_race_adopts_the_winner_instead_of_clobbering_it() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        // Another writer got there between our existence check and our write.
        let sibling = "A_KEY_A_SIBLING_WROTE: 7\n";
        std::fs::write(&config_path, sibling).unwrap();

        let values = config
            .create_default_config_if_missing(Mapping::new())
            .unwrap();

        assert_eq!(
            values.get("A_KEY_A_SIBLING_WROTE"),
            Some(&serde_yaml::Value::from(7)),
            "the sibling's config is the answer, not the default we were about to write"
        );
        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            sibling,
            "and it must still be on disk: a create-if-missing that replaces is a data loss"
        );
    }

    /// The narrow half of the same rule: the file appears *after* the existence
    /// check, so our write is the one that loses.
    #[test]
    fn a_write_that_loses_to_a_sibling_answers_with_the_siblings_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        // Our rename is denied the way Windows denies one against a name being
        // replaced — because that is exactly what happened: the sibling landed.
        let sibling = "A_KEY_A_SIBLING_WROTE: 7\n";
        config.io_faults.failing_rename_attempts =
            std::sync::atomic::AtomicUsize::new(TRANSIENT_IO_ATTEMPTS);
        *config
            .io_faults
            .sibling_lands_on_rename_fault
            .lock()
            .unwrap() = Some(sibling.to_string());

        let values = config
            .create_default_config_if_missing(Mapping::new())
            .unwrap();

        assert_eq!(
            values.get("A_KEY_A_SIBLING_WROTE"),
            Some(&serde_yaml::Value::from(7)),
            "a write that lost the race is answered by reading the winner, not by \
             reporting a failure or returning an empty default"
        );
        assert!(
            !config
                .staging_path()
                .parent()
                .unwrap()
                .read_dir()
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")),
            "a failed write must not leave its staging file behind"
        );
    }

    /// A config file that has not moved is read from disk once.
    ///
    /// The assertion is the ATTEMPT COUNT, not the returned value: two reads
    /// that agree are indistinguishable from one read and a cache hit, and it
    /// is the second read that this exists to remove. `get_param` is on the
    /// path of `ModelConfig::new`, the privacy tier resolver, the extension map
    /// and the agent's turn loop, and before this every one of those lookups
    /// re-read and re-parsed the whole file.
    #[test]
    fn a_second_lookup_of_an_unchanged_config_reads_the_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        std::fs::write(&config_path, "A_KEY_THAT_IS_SET: 7\n").unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        assert_eq!(config.get_param::<i32>("A_KEY_THAT_IS_SET").unwrap(), 7);
        let after_first = config.io_probe.read_attempts();
        assert_eq!(after_first, 1, "the first lookup must read the file");

        for _ in 0..32 {
            assert_eq!(config.get_param::<i32>("A_KEY_THAT_IS_SET").unwrap(), 7);
            // A key that is absent still resolves through the same load.
            assert!(matches!(
                config.get_param::<i32>("A_KEY_NOBODY_SET"),
                Err(ConfigError::NotFound(_))
            ));
        }

        assert_eq!(
            config.io_probe.read_attempts(),
            after_first,
            "an unchanged config file must be read once, not once per lookup"
        );
    }

    /// A start-up storm on a missing config file creates it **once**.
    ///
    /// The width of that storm is the whole reason #188 and #197 exist: every
    /// thread reaching `load` while `config.yaml` is absent takes the create
    /// branch, so one needed file creation became N replacements of it, and on
    /// Windows each replacement makes the destination name briefly unopenable.
    /// Both PRs made the storm SURVIVABLE. This is the half that makes it stop
    /// happening: the single-flight gate means exactly one caller runs the
    /// create branch and the rest take its result.
    ///
    /// Deterministic despite the threads, because the count is bounded by the
    /// gate rather than by scheduling: a thread that arrives after the file
    /// exists reads it, and a thread that queued behind the winner finds the
    /// winner's answer under the same stamp it sampled.
    #[test]
    fn a_startup_storm_creates_the_config_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let config = std::sync::Arc::new(
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap(),
        );

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let config = std::sync::Arc::clone(&config);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    (0..8)
                        .map(|_| config.get_param::<i32>("A_KEY_NOBODY_SET"))
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        for worker in workers {
            for outcome in worker.join().unwrap() {
                assert!(
                    matches!(outcome, Err(ConfigError::NotFound(_))),
                    "an unset key must still read as NotFound; got {outcome:?}"
                );
            }
        }

        assert!(config_path.exists(), "the storm must still create the file");
        assert_eq!(
            config.io_probe.config_writes(),
            1,
            "128 lookups against a missing config file must produce ONE write, not one \
             per thread that saw it missing"
        );
    }

    /// A write made through this process is visible to the very next lookup —
    /// on every path that writes, not just the one that is easiest to test.
    ///
    /// ⚠ This is why invalidation lives in `save_values` and not in
    /// `set_param`. Four of that function's callers do not hold `guard`, and
    /// one of them is the create branch of `load` itself.
    #[test]
    fn every_write_path_is_visible_to_the_next_lookup() -> Result<(), ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let config = Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml"))?;

        // Warm the cache on a file that does not exist yet.
        assert!(config.all_values()?.is_empty());

        config.set_param("k_set", "one")?;
        assert_eq!(config.get_param::<String>("k_set")?, "one");

        config.update_param::<String, _, _>("k_set", |v| *v = "two".to_string())?;
        assert_eq!(config.get_param::<String>("k_set")?, "two");

        config.delete("k_set")?;
        assert!(matches!(
            config.get_param::<String>("k_set"),
            Err(ConfigError::NotFound(_))
        ));

        config.save_values(Mapping::from_iter([(
            serde_yaml::to_value("k_saved")?,
            serde_yaml::to_value("direct")?,
        )]))?;
        assert_eq!(config.get_param::<String>("k_saved")?, "direct");

        config.clear()?;
        assert!(
            config.all_values()?.is_empty(),
            "a cleared config must not be served out of the cache"
        );

        Ok(())
    }

    /// A change another process made is picked up without a restart.
    ///
    /// The desktop app and the CLI share one `config.yaml`, so this is a
    /// correctness requirement rather than a nicety: `BIOROUTER_PROVIDER` is a
    /// capability key the privacy tier resolver reads through `get_param`, and
    /// a stale answer there is a stale tier.
    #[test]
    fn a_change_made_outside_this_process_is_picked_up_by_the_next_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        std::fs::write(&config_path, "A_SHARED_KEY: before\n").unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        assert_eq!(
            config.get_param::<String>("A_SHARED_KEY").unwrap(),
            "before"
        );

        // Another process rewrites the file. Nothing tells us it happened.
        std::fs::write(&config_path, "A_SHARED_KEY: after-an-external-write\n").unwrap();

        assert_eq!(
            config.get_param::<String>("A_SHARED_KEY").unwrap(),
            "after-an-external-write",
            "a config file rewritten by another process must not be served from cache"
        );

        // And a file that is removed underneath us is not served either.
        std::fs::remove_file(&config_path).unwrap();
        assert!(matches!(
            config.get_param::<String>("A_SHARED_KEY"),
            Err(ConfigError::NotFound(_))
        ));
        assert!(
            config_path.exists(),
            "reading a missing config still re-creates it"
        );
    }

    /// A load that CREATED the config file must not be filed under the absence
    /// it replaced.
    ///
    /// The load path writes — it creates a missing config, restores a backup
    /// over one, replaces one that will not parse — so its result describes the
    /// state it produced and not the state it started from. The create branch
    /// is the instance where getting this wrong is most obviously wrong: the
    /// entry would read "the file is absent, the values are the default" and
    /// be stored *after* making the file exist, so a later deletion would find
    /// a cache entry that matches, answer from it, and neither notice nor
    /// re-create the file.
    #[test]
    fn a_load_that_created_the_file_is_not_cached_as_the_absence_it_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        assert!(config.all_values().unwrap().is_empty());
        assert!(config_path.exists(), "the first read creates the file");

        std::fs::remove_file(&config_path).unwrap();

        assert!(config.all_values().unwrap().is_empty());
        assert!(
            config_path.exists(),
            "the file is absent again and this read must act on that, not answer from an \
             entry describing the absence the previous read already ended"
        );
    }

    /// `POST /config/recover` is implemented as a forced re-read, so there has
    /// to be a way to force one.
    #[test]
    fn an_explicit_invalidation_forces_a_re_read() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        std::fs::write(&config_path, "A_KEY_THAT_IS_SET: 1\n").unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        assert_eq!(config.get_param::<i32>("A_KEY_THAT_IS_SET").unwrap(), 1);
        let after_first = config.io_probe.read_attempts();

        config.invalidate_values_cache();
        assert_eq!(config.get_param::<i32>("A_KEY_THAT_IS_SET").unwrap(), 1);

        assert!(
            config.io_probe.read_attempts() > after_first,
            "invalidating must actually send the next lookup to the disk"
        );
    }

    /// Nothing on the uncached load path may reach back through the cache.
    ///
    /// `load_shared` holds `values_read` across the whole of `load_uncached`,
    /// and a `std::sync::Mutex` is not reentrant — so a `get_param` or
    /// `all_values` added anywhere underneath it (a settings lookup inside a
    /// backup routine, an extension read while recovering) deadlocks the
    /// process against itself.
    ///
    /// ⚠ Asserted from the source rather than by running anything, because the
    /// failure mode is a **hung** `cargo test` and not a failing assertion —
    /// the most expensive shape a defect in this file can take, and one no
    /// green run can rule out. `load_uncached` writes (it creates a missing
    /// config), so the reachable set is much larger than it looks.
    #[test]
    fn nothing_on_the_uncached_load_path_reaches_back_through_the_cache() {
        /// The body of `fn <name>`, by brace matching from its declaration.
        ///
        /// `get` rather than `[..]` throughout: this file is full of non-ASCII
        /// prose, `clippy::string_slice` is denied, and a matcher that returns
        /// `None` where it would have panicked is caught by the non-vacuity
        /// floor below rather than taking the test run down.
        fn body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
            let decl = ["(", "<"]
                .iter()
                .filter_map(|suffix| src.find(&format!("fn {name}{suffix}")))
                .min()?;
            let open = decl + src.get(decl..)?.find('{')?;
            let mut depth = 0usize;
            for (offset, ch) in src.get(open..)?.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return src.get(open..open + offset + 1);
                        }
                    }
                    _ => {}
                }
            }
            None
        }

        let source = include_str!("base.rs");
        let production = source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("this file ends with its test module")
            .0;
        // ⚠ Slicing at the FIRST `#[cfg(test)]` would cut the file off at
        // `IoFaults`, half way up the production half, and the scan would go
        // quietly vacuous. The floor below is what catches that if this
        // marker ever stops being the last one.
        let calls = regex::Regex::new(r"(?:self\.|Self::)([a-z_][a-z0-9_]*)\s*\(")
            .expect("a compile-time-constant pattern");

        let mut reachable = std::collections::BTreeSet::new();
        let mut frontier = vec!["load_uncached".to_string()];
        while let Some(name) = frontier.pop() {
            if !reachable.insert(name.clone()) {
                continue;
            }
            let Some(fn_body) = body(production, &name) else {
                continue;
            };
            for callee in calls.captures_iter(fn_body) {
                let callee = callee[1].to_string();
                if !reachable.contains(&callee) && body(production, &callee).is_some() {
                    frontier.push(callee);
                }
            }
        }

        // Non-vacuity floor: a scan that walks nothing proves nothing, and the
        // shape it would take here is a body matcher that failed and returned
        // an empty set.
        for expected in [
            "save_values",
            "create_backup_if_needed",
            "create_default_config_if_missing",
            "try_restore_from_backup",
            "read_config_file",
            "install_staged_config",
        ] {
            assert!(
                reachable.contains(expected),
                "the scan did not reach {expected}, so it is not measuring the load path: \
                 {reachable:?}"
            );
        }

        for forbidden in ["load", "load_shared", "get_param", "all_values"] {
            assert!(
                !reachable.contains(forbidden),
                "{forbidden} is reachable from load_uncached, which runs while `values_read` \
                 is held — that is a self-deadlock, and it will show up as a HUNG test run \
                 rather than a failing one. Reached via: {reachable:?}"
            );
        }
    }

    /// A config the user genuinely cannot read costs the retry budget ONCE,
    /// not once per lookup.
    ///
    /// #197's retry admits `PermissionDenied` on every platform, because that
    /// is what Windows answers a read of a name mid-replacement with and the
    /// rule has to be testable where it can be tested. The cost it left behind
    /// is a unix config with genuinely wrong permissions paying ~63 ms (8
    /// attempts) per `get_param` before reporting the same error. Narrowing the
    /// predicate to `cfg!(windows)` would make the rule untestable everywhere
    /// it matters — so the verdict is cached instead, and the predicate is
    /// untouched.
    ///
    /// A `chmod` that repairs the file is still noticed: permission bits are
    /// part of the [`FileStamp`], precisely because a repair moves neither the
    /// length nor the modification time.
    #[test]
    fn an_unreadable_config_spends_the_retry_budget_once_not_once_per_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        std::fs::write(&config_path, "A_KEY_THAT_IS_SET: 1\n").unwrap();
        let mut config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();
        // Denied more times than the budget allows, on every attempt: the
        // failure never lifts, which is the case the retry cannot help with.
        config.io_faults.failing_read_attempts =
            std::sync::atomic::AtomicUsize::new(TRANSIENT_IO_ATTEMPTS + 1);

        let first = config.get_param::<i32>("A_KEY_THAT_IS_SET");
        assert!(
            matches!(first, Err(ConfigError::FileError(_))),
            "a denial that never lifts is still reported; got {first:?}"
        );
        let after_first = config.io_probe.read_attempts();
        assert_eq!(
            after_first, TRANSIENT_IO_ATTEMPTS,
            "the first lookup spends the whole budget, exactly as before"
        );

        for _ in 0..16 {
            let repeated = config.get_param::<i32>("A_KEY_THAT_IS_SET");
            assert!(
                matches!(repeated, Err(ConfigError::FileError(_))),
                "the cached verdict must be the same answer, not a different one; \
                 got {repeated:?}"
            );
        }

        assert_eq!(
            config.io_probe.read_attempts(),
            after_first,
            "16 further lookups must not each spend another ~63 ms re-deriving the same \
             refusal"
        );
    }

    /// A default config that could not be written is REPORTED, not swallowed.
    ///
    /// Both write paths returned `Ok` regardless, so an install running
    /// entirely on in-memory defaults — a config directory that cannot be
    /// written, a full disk — looked exactly like a healthy one, and every
    /// setting the user changed vanished at exit with nothing said.
    ///
    /// Reported out of band rather than returned: `load` staying answerable is
    /// what makes the start-up storm survivable, and
    /// `a_startup_storm_on_a_missing_config_never_reports_anything_but_not_found`
    /// asserts it directly.
    #[test]
    fn a_default_config_that_cannot_be_written_is_recorded_rather_than_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        // The parent of the config file is a FILE, so creating the directory
        // fails. Portable, unlike a mode-0 directory: `chmod` does not mean the
        // same thing on Windows, and CI runs there.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "not a directory").unwrap();
        let config_path = blocked.join("config.yaml");
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        assert_eq!(config.last_write_error(), None, "nothing has failed yet");

        // The lookup still answers — that is deliberate and load-bearing.
        assert!(matches!(
            config.get_param::<i32>("A_KEY_NOBODY_SET"),
            Err(ConfigError::NotFound(_))
        ));

        let reported = config
            .last_write_error()
            .expect("a default config that could not be written must be reported somewhere");
        assert!(
            reported.contains("config directory"),
            "the report must name what actually failed; got {reported:?}"
        );

        // And it is not re-derived on every lookup either.
        for _ in 0..8 {
            let _ = config.get_param::<i32>("A_KEY_NOBODY_SET");
        }
        assert_eq!(
            config.io_probe.config_writes(),
            1,
            "an unwritable config directory must be hammered once, not once per lookup"
        );
    }

    /// A write that merely LOST A RACE is not a write failure.
    ///
    /// The two default-config paths need different answers here, which is why
    /// they are two functions: creating a missing config adopts whatever a
    /// sibling installed instead, and only a failure with no sibling to adopt
    /// is worth reporting. Recording the lost race as well would make every
    /// start-up storm report a write error.
    #[test]
    fn a_create_that_lost_a_race_is_not_recorded_as_a_write_failure() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        let sibling = "A_KEY_A_SIBLING_WROTE: 7\n";
        config.io_faults.failing_rename_attempts =
            std::sync::atomic::AtomicUsize::new(TRANSIENT_IO_ATTEMPTS);
        *config
            .io_faults
            .sibling_lands_on_rename_fault
            .lock()
            .unwrap() = Some(sibling.to_string());

        let values = config
            .create_default_config_if_missing(Mapping::new())
            .unwrap();

        assert_eq!(
            values.get("A_KEY_A_SIBLING_WROTE"),
            Some(&serde_yaml::Value::from(7))
        );
        assert_eq!(
            config.last_write_error(),
            None,
            "losing a race to a sibling is the expected outcome of a storm, not a fault \
             to report to the user"
        );
    }

    /// 1,000 `get_param` lookups against a warm config file on disk.
    ///
    /// Ignored because it measures rather than asserts. Run it with
    /// `cargo test -p biorouter --lib -- config::base::tests::measure --ignored --nocapture`.
    ///
    /// ⚠ It must use a config file that EXISTS. `new_test_config()` drops its
    /// `NamedTempFile`, so its path does not — a benchmark built on that
    /// measures the create branch and reports a number about a case that
    /// happens once per install.
    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn measure_a_thousand_get_param_lookups() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut body = String::from("BIOROUTER_MODEL: gpt-5.5\nextensions:\n");
        for i in 0..40 {
            body.push_str(&format!(
                "  ext_{i}:\n    enabled: true\n    type: stdio\n    cmd: biorouter\n    \
                 args: [mcp, developer]\n    envs: {{}}\n    timeout: 300\n"
            ));
        }
        body.push_str("A_KEY_THAT_IS_SET: 7\n");
        std::fs::write(&config_path, &body).unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        // Warm, so the number is about steady-state lookups.
        assert_eq!(config.get_param::<i32>("A_KEY_THAT_IS_SET").unwrap(), 7);

        let started = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(config.get_param::<i32>("A_KEY_THAT_IS_SET").unwrap(), 7);
        }
        let elapsed = started.elapsed();
        println!(
            "1000 get_param lookups over a {} byte config: {elapsed:?} ({:?} each), \
             {} file reads",
            body.len(),
            elapsed / 1000,
            config.io_probe.read_attempts()
        );
    }

    #[test]
    fn test_backup_rotation() -> Result<(), ConfigError> {
        let config = new_test_config();

        // Create multiple versions to test rotation
        for i in 1..=7 {
            config.set_param("version", i)?;
        }

        let backup_paths = config.get_backup_paths();

        // Should have backups but not more than our limit
        let existing_backups: Vec<_> = backup_paths.iter().filter(|p| p.exists()).collect();
        assert!(
            existing_backups.len() <= 6,
            "Should not exceed backup limit"
        ); // .bak + .bak.1 through .bak.5

        Ok(())
    }

    #[test]
    fn test_env_var_parsing_strings() -> Result<(), ConfigError> {
        // Test unquoted strings
        let value = Config::parse_env_value("ANTHROPIC")?;
        assert_eq!(value, Value::String("ANTHROPIC".to_string()));

        // Test strings with spaces
        let value = Config::parse_env_value("hello world")?;
        assert_eq!(value, Value::String("hello world".to_string()));

        // Test JSON quoted strings
        let value = Config::parse_env_value("\"ANTHROPIC\"")?;
        assert_eq!(value, Value::String("ANTHROPIC".to_string()));

        // Test empty string
        let value = Config::parse_env_value("")?;
        assert_eq!(value, Value::String("".to_string()));

        Ok(())
    }

    #[test]
    fn test_env_var_parsing_numbers() -> Result<(), ConfigError> {
        // Test integers
        let value = Config::parse_env_value("42")?;
        assert_eq!(value, Value::Number(42.into()));

        let value = Config::parse_env_value("-123")?;
        assert_eq!(value, Value::Number((-123).into()));

        // Test floats
        let value = Config::parse_env_value("3.41")?;
        assert!(matches!(value, Value::Number(_)));
        if let Value::Number(n) = value {
            assert_eq!(n.as_f64().unwrap(), 3.41);
        }

        let value = Config::parse_env_value("0.01")?;
        assert!(matches!(value, Value::Number(_)));
        if let Value::Number(n) = value {
            assert_eq!(n.as_f64().unwrap(), 0.01);
        }

        // Test zero
        let value = Config::parse_env_value("0")?;
        assert_eq!(value, Value::Number(0.into()));

        let value = Config::parse_env_value("0.0")?;
        assert!(matches!(value, Value::Number(_)));
        if let Value::Number(n) = value {
            assert_eq!(n.as_f64().unwrap(), 0.0);
        }

        // Test numbers starting with decimal point
        let value = Config::parse_env_value(".5")?;
        assert!(matches!(value, Value::Number(_)));
        if let Value::Number(n) = value {
            assert_eq!(n.as_f64().unwrap(), 0.5);
        }

        let value = Config::parse_env_value(".00001")?;
        assert!(matches!(value, Value::Number(_)));
        if let Value::Number(n) = value {
            assert_eq!(n.as_f64().unwrap(), 0.00001);
        }

        Ok(())
    }

    #[test]
    fn test_env_var_parsing_booleans() -> Result<(), ConfigError> {
        // Test true variants
        let value = Config::parse_env_value("true")?;
        assert_eq!(value, Value::Bool(true));

        let value = Config::parse_env_value("True")?;
        assert_eq!(value, Value::Bool(true));

        let value = Config::parse_env_value("TRUE")?;
        assert_eq!(value, Value::Bool(true));

        // Test false variants
        let value = Config::parse_env_value("false")?;
        assert_eq!(value, Value::Bool(false));

        let value = Config::parse_env_value("False")?;
        assert_eq!(value, Value::Bool(false));

        let value = Config::parse_env_value("FALSE")?;
        assert_eq!(value, Value::Bool(false));

        Ok(())
    }

    #[test]
    fn test_env_var_parsing_json() -> Result<(), ConfigError> {
        // Test JSON objects
        let value = Config::parse_env_value("{\"host\": \"localhost\", \"port\": 8080}")?;
        assert!(matches!(value, Value::Object(_)));
        if let Value::Object(obj) = value {
            assert_eq!(
                obj.get("host"),
                Some(&Value::String("localhost".to_string()))
            );
            assert_eq!(obj.get("port"), Some(&Value::Number(8080.into())));
        }

        // Test JSON arrays
        let value = Config::parse_env_value("[1, 2, 3]")?;
        assert!(matches!(value, Value::Array(_)));
        if let Value::Array(arr) = value {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], Value::Number(1.into()));
            assert_eq!(arr[1], Value::Number(2.into()));
            assert_eq!(arr[2], Value::Number(3.into()));
        }

        // Test JSON null
        let value = Config::parse_env_value("null")?;
        assert_eq!(value, Value::Null);

        Ok(())
    }

    #[test]
    fn test_env_var_parsing_edge_cases() -> Result<(), ConfigError> {
        // Test whitespace handling
        let value = Config::parse_env_value(" 42 ")?;
        assert_eq!(value, Value::Number(42.into()));

        let value = Config::parse_env_value(" true ")?;
        assert_eq!(value, Value::Bool(true));

        // Test strings that look like numbers but aren't
        let value = Config::parse_env_value("123abc")?;
        assert_eq!(value, Value::String("123abc".to_string()));

        let value = Config::parse_env_value("abc123")?;
        assert_eq!(value, Value::String("abc123".to_string()));

        // Test strings that look like booleans but aren't
        let value = Config::parse_env_value("truthy")?;
        assert_eq!(value, Value::String("truthy".to_string()));

        let value = Config::parse_env_value("falsy")?;
        assert_eq!(value, Value::String("falsy".to_string()));

        Ok(())
    }

    #[test]
    fn test_env_var_parsing_numeric_edge_cases() -> Result<(), ConfigError> {
        // Test leading zeros (should be treated as integers, not octal)
        let value = Config::parse_env_value("007")?;
        assert_eq!(value, Value::Number(7.into()));

        // Test large numbers
        let value = Config::parse_env_value("9223372036854775807")?; // i64::MAX
        assert_eq!(value, Value::Number(9223372036854775807i64.into()));

        // Test scientific notation (JSON parsing should handle this correctly)
        let value = Config::parse_env_value("1e10")?;
        assert!(matches!(value, Value::Number(_)));
        if let Value::Number(n) = value {
            assert_eq!(n.as_f64().unwrap(), 1e10);
        }

        // Test infinity (should be treated as string)
        let value = Config::parse_env_value("inf")?;
        assert_eq!(value, Value::String("inf".to_string()));

        Ok(())
    }

    #[test]
    fn test_env_var_with_config_integration() -> Result<(), ConfigError> {
        let config = new_test_config();

        // Test string environment variable (the original issue case)
        std::env::set_var("PROVIDER", "ANTHROPIC");
        let value: String = config.get_param("provider")?;
        assert_eq!(value, "ANTHROPIC");

        // Test number environment variable
        std::env::set_var("PORT", "8080");
        let value: i32 = config.get_param("port")?;
        assert_eq!(value, 8080);

        // Test boolean environment variable
        std::env::set_var("ENABLED", "true");
        let value: bool = config.get_param("enabled")?;
        assert!(value);

        // Test JSON object environment variable
        std::env::set_var("CONFIG", "{\"debug\": true, \"level\": 5}");
        #[derive(Deserialize, Debug, PartialEq)]
        struct TestConfig {
            debug: bool,
            level: i32,
        }
        let value: TestConfig = config.get_param("config")?;
        assert!(value.debug);
        assert_eq!(value.level, 5);

        // Clean up
        std::env::remove_var("PROVIDER");
        std::env::remove_var("PORT");
        std::env::remove_var("ENABLED");
        std::env::remove_var("CONFIG");

        Ok(())
    }

    #[test]
    fn test_env_var_precedence_over_config_file() -> Result<(), ConfigError> {
        let config = new_test_config();

        // Set value in config file
        config.set_param("test_precedence", "file_value")?;

        // Verify file value is returned when no env var
        let value: String = config.get_param("test_precedence")?;
        assert_eq!(value, "file_value");

        // Set environment variable
        std::env::set_var("TEST_PRECEDENCE", "env_value");

        // Environment variable should take precedence
        let value: String = config.get_param("test_precedence")?;
        assert_eq!(value, "env_value");

        // Clean up
        std::env::remove_var("TEST_PRECEDENCE");

        Ok(())
    }

    #[test]
    fn get_secrets_primary_from_env_uses_env_for_secondary() {
        temp_env::with_vars(
            [
                ("TEST_PRIMARY", Some("primary_env")),
                ("TEST_SECONDARY", Some("secondary_env")),
            ],
            || {
                let config = new_test_config();
                let secrets = config
                    .get_secrets("TEST_PRIMARY", &["TEST_SECONDARY"])
                    .unwrap();

                assert_eq!(secrets["TEST_PRIMARY"], "primary_env");
                assert_eq!(secrets["TEST_SECONDARY"], "secondary_env");
            },
        );
    }

    #[test]
    fn get_secrets_primary_from_secret_uses_secret_for_secondary() {
        temp_env::with_vars(
            [("TEST_PRIMARY", None::<&str>), ("TEST_SECONDARY", None)],
            || {
                let config = new_test_config();
                config
                    .set_secret("TEST_PRIMARY", &"primary_secret")
                    .unwrap();
                config
                    .set_secret("TEST_SECONDARY", &"secondary_secret")
                    .unwrap();

                let secrets = config
                    .get_secrets("TEST_PRIMARY", &["TEST_SECONDARY"])
                    .unwrap();

                assert_eq!(secrets["TEST_PRIMARY"], "primary_secret");
                assert_eq!(secrets["TEST_SECONDARY"], "secondary_secret");
            },
        );
    }

    #[test]
    fn get_secrets_primary_missing_returns_error() {
        temp_env::with_vars([("TEST_PRIMARY", None::<&str>)], || {
            let config = new_test_config();

            let result = config.get_secrets("TEST_PRIMARY", &[]);

            assert!(matches!(result, Err(ConfigError::NotFound(_))));
        });
    }

    fn new_test_config() -> Config {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        Config::new_with_file_secrets(config_file.path(), secrets_file.path()).unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn task_local_override_is_used_and_scoped() {
        let config = new_test_config();
        let key = "auto_detect_test_key";
        let env_key = "AUTO_DETECT_TEST_KEY";
        std::env::remove_var(env_key);

        // Outside any override scope the secret is simply absent.
        assert!(config.get_secret::<String>(key).is_err());

        // Inside the scope, the override is returned without touching env.
        let mut overrides = HashMap::new();
        overrides.insert(env_key.to_string(), "candidate-123".to_string());
        let observed = with_config_overrides(overrides, async {
            // The process environment must NOT have been mutated.
            assert!(std::env::var(env_key).is_err());
            config.get_secret::<String>(key).unwrap()
        })
        .await;
        assert_eq!(observed, "candidate-123");

        // After the scope the override is gone and env is still untouched.
        assert!(config.get_secret::<String>(key).is_err());
        assert!(std::env::var(env_key).is_err());
    }

    #[tokio::test]
    #[serial]
    async fn task_local_override_wins_over_env() {
        let config = new_test_config();
        let key = "auto_detect_test_key2";
        let env_key = "AUTO_DETECT_TEST_KEY2";
        std::env::set_var(env_key, "real-env-value");

        let mut overrides = HashMap::new();
        overrides.insert(env_key.to_string(), "candidate-override".to_string());
        let inside = with_config_overrides(overrides, async {
            config.get_secret::<String>(key).unwrap()
        })
        .await;
        assert_eq!(inside, "candidate-override");

        // Outside the scope the real env value resolves again.
        assert_eq!(config.get_secret::<String>(key).unwrap(), "real-env-value");
        std::env::remove_var(env_key);
    }

    #[tokio::test]
    async fn task_local_secret_bundle_never_falls_through_to_keyring() {
        struct PanicsOnRead;

        impl KeyringBlobStore for PanicsOnRead {
            fn get(&self, _username: &str) -> Result<String, keyring::Error> {
                panic!("task-local credential detection must not read the keyring")
            }

            fn set(&self, _username: &str, _value: &str) -> Result<(), keyring::Error> {
                unreachable!()
            }

            fn delete(&self, _username: &str) -> Result<(), keyring::Error> {
                unreachable!()
            }
        }

        let config_file = NamedTempFile::new().unwrap();
        let config = Config {
            config_path: config_file.path().to_path_buf(),
            secrets: SecretStorage::Keyring {
                service: "biorouter-test".to_string(),
            },
            guard: Mutex::new(()),
            secrets_cache: Mutex::new(None),
            secrets_read: Mutex::new(()),
            values_cache: Mutex::new(None),
            values_read: Mutex::new(()),
            last_write_error: Mutex::new(None),
            test_keyring_store: Some(std::sync::Arc::new(PanicsOnRead)),
            io_faults: IoFaults::default(),
            io_probe: IoProbe::default(),
        };
        let overrides = HashMap::from([
            (
                "TASK_OVERRIDE_BUNDLE_PRIMARY".to_string(),
                "candidate-primary".to_string(),
            ),
            (
                "TASK_OVERRIDE_BUNDLE_SECONDARY".to_string(),
                "candidate-secondary".to_string(),
            ),
        ]);

        let secrets = with_config_overrides(overrides, async {
            config
                .get_secrets(
                    "TASK_OVERRIDE_BUNDLE_PRIMARY",
                    &[
                        "TASK_OVERRIDE_BUNDLE_SECONDARY",
                        "TASK_OVERRIDE_BUNDLE_MISSING",
                    ],
                )
                .unwrap()
        })
        .await;

        assert_eq!(secrets["TASK_OVERRIDE_BUNDLE_PRIMARY"], "candidate-primary");
        assert_eq!(
            secrets["TASK_OVERRIDE_BUNDLE_SECONDARY"],
            "candidate-secondary"
        );
        assert!(!secrets.contains_key("TASK_OVERRIDE_BUNDLE_MISSING"));
    }

    #[test]
    fn test_split_blob_into_chunks() {
        assert_eq!(Config::split_blob_into_chunks("", 10), vec![String::new()]);

        let blob = "a".repeat(25);
        let chunks = Config::split_blob_into_chunks(&blob, 10);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.concat(), blob);
        assert!(chunks.iter().all(|c| c.encode_utf16().count() <= 10));

        // Multibyte characters: each emoji is 2 UTF-16 units and must never
        // be split across chunks.
        let blob = "🌍".repeat(7);
        let chunks = Config::split_blob_into_chunks(&blob, 4);
        assert_eq!(chunks.concat(), blob);
        assert!(chunks.iter().all(|c| c.encode_utf16().count() <= 4));
    }

    /// In-memory KeyringBlobStore for exercising the chunking logic.
    struct MapStore(Mutex<HashMap<String, String>>);

    impl MapStore {
        fn new() -> Self {
            MapStore(Mutex::new(HashMap::new()))
        }
    }

    impl KeyringBlobStore for MapStore {
        fn get(&self, username: &str) -> Result<String, keyring::Error> {
            self.0
                .lock()
                .unwrap()
                .get(username)
                .cloned()
                .ok_or(keyring::Error::NoEntry)
        }
        fn set(&self, username: &str, value: &str) -> Result<(), keyring::Error> {
            self.0
                .lock()
                .unwrap()
                .insert(username.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, username: &str) -> Result<(), keyring::Error> {
            self.0
                .lock()
                .unwrap()
                .remove(username)
                .map(|_| ())
                .ok_or(keyring::Error::NoEntry)
        }
    }

    #[test]
    fn test_chunked_blob_roundtrip() {
        let store = MapStore::new();

        // Small blobs use a single entry, no header.
        Config::write_blob_to(&store, "{\"k\":\"v\"}", 1000).unwrap();
        assert_eq!(store.0.lock().unwrap().len(), 1);
        assert_eq!(Config::read_blob_from(&store).unwrap(), "{\"k\":\"v\"}");

        // A large blob is split across continuation entries (the Windows
        // Credential Manager case) and reassembles exactly.
        let big: String = (0..200)
            .map(|i| format!("\"key{i}\":\"value with ünïcode 🌍 {i}\","))
            .collect();
        let big = format!("{{{}}}", big.trim_end_matches(','));
        Config::write_blob_to(&store, &big, 1000).unwrap();
        assert!(store
            .get(KEYRING_USERNAME)
            .unwrap()
            .starts_with(KEYRING_CHUNK_MARKER));
        assert!(store
            .0
            .lock()
            .unwrap()
            .values()
            .all(|v| v.encode_utf16().count() <= 1000));
        assert_eq!(Config::read_blob_from(&store).unwrap(), big);

        // Shrinking back to a small blob removes stale continuation entries.
        Config::write_blob_to(&store, "{\"k\":\"v2\"}", 1000).unwrap();
        assert_eq!(Config::read_blob_from(&store).unwrap(), "{\"k\":\"v2\"}");
        assert_eq!(store.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_cold_cache_reads_the_credential_store_exactly_once_under_concurrency() {
        // `/config/providers` fans out over every provider at once, and each row
        // asks whether its keys are set. Before the single-flight gate the cache
        // could not collapse that: all of them missed together and all of them
        // read the store. On macOS each read is a potential Keychain prompt, and
        // a read that loses the race returns an error -- which is how one blob
        // produced a grid with `versa_azure` Configured and `versa_bedrock` not.
        struct CountingStore {
            blob: String,
            reads: std::sync::atomic::AtomicUsize,
        }
        impl KeyringBlobStore for CountingStore {
            fn get(&self, _username: &str) -> Result<String, keyring::Error> {
                self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Slow enough that concurrent callers genuinely overlap; a fast
                // fake would let them serialise by luck and pass either way.
                std::thread::sleep(std::time::Duration::from_millis(40));
                Ok(self.blob.clone())
            }
            fn set(&self, _username: &str, _value: &str) -> Result<(), keyring::Error> {
                Ok(())
            }
            fn delete(&self, _username: &str) -> Result<(), keyring::Error> {
                Ok(())
            }
        }

        let store = std::sync::Arc::new(CountingStore {
            blob: "{\"VERSA_AZURE_API_KEY\":\"a\",\"VERSA_BEDROCK_ACCESS_KEY_ID\":\"b\"}"
                .to_string(),
            reads: std::sync::atomic::AtomicUsize::new(0),
        });
        let config_file = NamedTempFile::new().unwrap();
        let config = std::sync::Arc::new(Config {
            config_path: config_file.path().to_path_buf(),
            secrets: SecretStorage::Keyring {
                service: "biorouter-test".to_string(),
            },
            guard: Mutex::new(()),
            secrets_cache: Mutex::new(None),
            secrets_read: Mutex::new(()),
            values_cache: Mutex::new(None),
            values_read: Mutex::new(()),
            last_write_error: Mutex::new(None),
            test_keyring_store: Some(store.clone()),
            io_faults: IoFaults::default(),
            io_probe: IoProbe::default(),
        });

        let handles: Vec<_> = (0..24)
            .map(|_| {
                let config = config.clone();
                std::thread::spawn(move || config.all_secrets().map(|m| m.len()))
            })
            .collect();
        for handle in handles {
            // Every caller gets the whole blob, not just the winner.
            assert_eq!(handle.join().unwrap().unwrap(), 2);
        }
        assert_eq!(
            store.reads.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "24 concurrent cold readers must produce exactly one credential-store read"
        );
    }

    #[test]
    fn test_secrets_cache_serves_reads_and_tracks_writes() -> Result<(), ConfigError> {
        // The cache is what collapses N keychain reads (each a potential
        // macOS authorization prompt) into a single read per process.
        let config_file = NamedTempFile::new().unwrap();
        let store = std::sync::Arc::new(MapStore::new());
        let config = Config {
            config_path: config_file.path().to_path_buf(),
            secrets: SecretStorage::Keyring {
                service: "biorouter-test".to_string(),
            },
            guard: Mutex::new(()),
            secrets_cache: Mutex::new(None),
            secrets_read: Mutex::new(()),
            values_cache: Mutex::new(None),
            values_read: Mutex::new(()),
            last_write_error: Mutex::new(None),
            test_keyring_store: Some(store.clone()),
            io_faults: IoFaults::default(),
            io_probe: IoProbe::default(),
        };

        config.set_secret("cache_test_key", &"v1")?;
        let value: String = config.get_secret("cache_test_key")?;
        assert_eq!(value, "v1");

        // Mutate the underlying store directly; the cached value must be
        // served without another store read.
        store
            .set(KEYRING_USERNAME, "{\"cache_test_key\":\"external\"}")
            .unwrap();
        let value: String = config.get_secret("cache_test_key")?;
        assert_eq!(value, "v1");

        // Invalidation forces a re-read from the store.
        config.invalidate_secrets_cache();
        let value: String = config.get_secret("cache_test_key")?;
        assert_eq!(value, "external");

        // Writes go through to the store, not just the cache.
        config.set_secret("cache_test_key", &"v2")?;
        assert!(store.get(KEYRING_USERNAME).unwrap().contains("v2"));
        let value: String = config.get_secret("cache_test_key")?;
        assert_eq!(value, "v2");

        // Deletes persist and update the cache.
        config.delete_secret("cache_test_key")?;
        let missing: Result<String, _> = config.get_secret("cache_test_key");
        assert!(matches!(missing, Err(ConfigError::NotFound(_))));
        Ok(())
    }
}
