//! Where speech models are found, in order (DD-19, `docs/dictation/engine-and-models.md`,
//! "Lookup order and writers").
//!
//! For each catalog id the first verified copy wins:
//!
//! 1. the managed `dictation.models_dir`, the only source while managed policy sets
//!    `model_downloads: false`;
//! 2. the environment's `BIOROUTER_DICTATION_MODELS_DIR`;
//! 3. this account's folder, `Paths::in_local_data_dir("dictation/models")`;
//! 4. the computer's system folder.
//!
//! In every folder a copy is `<folder>/<id>/<file_name>`; the read only folders (1, 2
//! and 4) also take `<folder>/<file_name>`, so an administrator can drop files in flat.
//! A copy whose hash is wrong in a read only folder is skipped and left in place; one in
//! this account's folder is deleted. Only this account's folder is written by the
//! daemon and `biorouter dictation install`; `--system` writes the system folder.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use biorouter_dictation::catalog::{Catalog, SttCatalogEntry};
use biorouter_dictation::models::{self, Verdict};
use serde::Serialize;

use crate::config::paths::Paths;
use crate::managed::{DictationPolicy, ManagedPolicy};

/// Environment only: a read only folder of models, after the managed one.
pub const MODELS_DIR_ENV: &str = "BIOROUTER_DICTATION_MODELS_DIR";

/// Which folder a copy was found in (`status`'s `location`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Managed,
    Env,
    User,
    System,
}

impl Location {
    pub fn as_str(self) -> &'static str {
        match self {
            Location::Managed => "managed",
            Location::Env => "env",
            Location::User => "user",
            Location::System => "system",
        }
    }
}

/// This account's models folder.
pub fn user_models_dir() -> PathBuf {
    Paths::in_local_data_dir("dictation/models")
}

/// The computer's shared folder, which `sudo biorouter dictation install --system`
/// fills and every account reads. Under `BIOROUTER_PATH_ROOT` it is
/// `<root>/system/dictation/models`, so a sandbox never reads the real one.
pub fn system_models_dir() -> Option<PathBuf> {
    if let Some(root) = Paths::path_root_override() {
        return Some(
            PathBuf::from(root)
                .join("system")
                .join("dictation")
                .join("models"),
        );
    }
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(
            "/Library/Application Support/Biorouter/dictation/models",
        ))
    }
    #[cfg(target_os = "linux")]
    {
        Some(PathBuf::from("/var/lib/biorouter/dictation/models"))
    }
    #[cfg(windows)]
    {
        std::env::var_os("ProgramData").map(|data| {
            PathBuf::from(data)
                .join("Biorouter")
                .join("dictation")
                .join("models")
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

/// The folders a lookup reads, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    pub managed: Option<PathBuf>,
    pub env: Option<PathBuf>,
    pub user: PathBuf,
    pub system: Option<PathBuf>,
    /// Managed `model_downloads: false`: the managed folder is the only source.
    pub managed_only: bool,
}

impl Roots {
    /// This process's folders: the managed policy as it reads now, the environment,
    /// this account and the computer.
    pub fn current() -> Roots {
        let policy = ManagedPolicy::load();
        Roots::from_parts(
            policy.dictation(),
            std::env::var_os(MODELS_DIR_ENV),
            user_models_dir(),
            system_models_dir(),
        )
    }

    pub fn from_parts(
        policy: &DictationPolicy,
        env: Option<OsString>,
        user: PathBuf,
        system: Option<PathBuf>,
    ) -> Roots {
        let env = env
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        Roots {
            managed: policy.models_dir.clone(),
            env,
            user,
            system,
            managed_only: policy.managed && !policy.model_downloads.is_allowed(),
        }
    }

    /// The folders in lookup order.
    pub fn ordered(&self) -> Vec<(Location, &Path)> {
        let mut roots: Vec<(Location, &Path)> = Vec::new();
        if let Some(managed) = &self.managed {
            roots.push((Location::Managed, managed));
        }
        if self.managed_only {
            return roots;
        }
        if let Some(env) = &self.env {
            roots.push((Location::Env, env));
        }
        roots.push((Location::User, &self.user));
        if let Some(system) = &self.system {
            roots.push((Location::System, system));
        }
        roots
    }

    /// The folder a write at `location` goes to: this account's, or the system's.
    pub fn writable(&self, system: bool) -> Option<&Path> {
        if system {
            self.system.as_deref()
        } else {
            Some(&self.user)
        }
    }
}

/// A verified copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub entry: &'static SttCatalogEntry,
    pub location: Location,
    pub path: PathBuf,
    /// The copy's SHA-256: the pin, or an accepted earlier revision.
    pub sha256: String,
    /// Verified by an earlier revision's hash, not the current pin.
    pub superseded: bool,
}

/// The first verified copy of `entry`, in lookup order. Hashes what it must (DD-17), and
/// only that: each candidate is checked when the ones before it failed, so a copy found
/// in `<folder>/<id>/` never costs a read of the flat copy beside it. Call it off the
/// async runtime when a copy may need a full hash.
pub fn find(roots: &Roots, entry: &'static SttCatalogEntry) -> Option<Found> {
    for (location, root) in roots.ordered() {
        let candidates: Vec<Box<dyn Fn() -> Verdict + '_>> = if location == Location::User {
            vec![Box::new(|| models::verify_writable(root, entry))]
        } else {
            vec![
                Box::new(|| models::verify_readonly(&models::model_path(root, entry), entry)),
                Box::new(|| models::verify_readonly(&models::flat_path(root, entry), entry)),
            ]
        };
        for candidate in candidates {
            if let Verdict::Verified {
                path,
                sha256,
                superseded,
            } = candidate()
            {
                return Some(Found {
                    entry,
                    location,
                    path,
                    sha256,
                    superseded,
                });
            }
        }
    }
    None
}

/// Where verified copies come from: a full [`Inventory`], or a [`Lookup`] that finds
/// only what it is asked for.
pub trait Installed {
    /// The first verified copy of `entry`, if there is one.
    fn found(&mut self, entry: &'static SttCatalogEntry) -> Option<Found>;

    /// A speech model whose voice detector is found too: what can be loaded. The model is
    /// looked up first, and its detector only when the model is there.
    fn usable(&mut self, catalog: &Catalog, entry: &'static SttCatalogEntry) -> bool {
        entry.is_speech()
            && self.found(entry).is_some()
            && entry
                .requires
                .iter()
                .all(|id| catalog.entry(id).is_some_and(|e| self.found(e).is_some()))
    }
}

impl Installed for Inventory {
    fn found(&mut self, entry: &'static SttCatalogEntry) -> Option<Found> {
        self.get(entry.id).cloned()
    }
}

/// Finds catalog entries one at a time, each at most once: what a load needs, and
/// nothing else. A model nobody asked for is never opened, so the first load in a
/// process, which hashes every copy it reads in a read only folder, reads only the files
/// it will load (DD-17, the arming path).
pub struct Lookup<'a> {
    roots: &'a Roots,
    seen: Vec<(&'static str, Option<Found>)>,
}

impl<'a> Lookup<'a> {
    pub fn new(roots: &'a Roots) -> Lookup<'a> {
        Lookup {
            roots,
            seen: Vec::new(),
        }
    }

    /// The ids looked up so far, in order.
    pub fn looked_up(&self) -> Vec<&'static str> {
        self.seen.iter().map(|(id, _)| *id).collect()
    }
}

impl Installed for Lookup<'_> {
    fn found(&mut self, entry: &'static SttCatalogEntry) -> Option<Found> {
        if let Some((_, found)) = self.seen.iter().find(|(id, _)| *id == entry.id) {
            return found.clone();
        }
        let found = find(self.roots, entry);
        self.seen.push((entry.id, found.clone()));
        found
    }
}

/// A file or folder in a models folder that no catalog entry claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stray {
    pub path: PathBuf,
    pub bytes: u64,
    /// A folder named after a retired id.
    pub retired: bool,
}

/// What every folder holds: each catalog entry's verified copy, and what nobody claims.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub found: Vec<Found>,
    pub strays: Vec<Stray>,
}

impl Inventory {
    pub fn get(&self, id: &str) -> Option<&Found> {
        self.found.iter().find(|f| f.entry.id == id)
    }

    /// Speech models with their voice detector found too: what can be loaded.
    pub fn usable(&self) -> impl Iterator<Item = &Found> {
        self.found.iter().filter(move |f| {
            f.entry.is_speech() && f.entry.requires.iter().all(|id| self.get(id).is_some())
        })
    }
}

/// Looks up every catalog entry and lists what no entry claims. Never hashes a stray.
pub fn inventory(roots: &Roots, catalog: &Catalog) -> Inventory {
    let found: Vec<Found> = catalog
        .entries
        .iter()
        .filter_map(|entry| find(roots, entry))
        .collect();
    let mut strays = Vec::new();
    for (location, root) in roots.ordered() {
        let Ok(items) = std::fs::read_dir(root) else {
            continue;
        };
        let mut items: Vec<_> = items.flatten().collect();
        items.sort_by_key(|item| item.file_name());
        for item in items {
            let name = item.file_name().to_string_lossy().into_owned();
            let path = item.path();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if catalog.entry(&name).is_some() {
                    continue;
                }
                strays.push(Stray {
                    bytes: folder_bytes(&path),
                    retired: catalog.retired.iter().any(|r| r.id == name),
                    path,
                });
            } else if path.is_file() {
                let claimed = location != Location::User && found.iter().any(|f| f.path == path);
                let companion = name.starts_with("LICENSE-") || name == models::INSTALLED_JSON;
                if !claimed && !companion {
                    strays.push(Stray {
                        bytes: item.metadata().map_or(0, |m| m.len()),
                        retired: false,
                        path,
                    });
                }
            }
        }
    }
    Inventory { found, strays }
}

fn folder_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|items| {
            items
                .flatten()
                .filter_map(|item| item.metadata().ok())
                .filter(|meta| meta.is_file())
                .map(|meta| meta.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::Rule;
    use biorouter_dictation::catalog::{SttKind, CATALOG, VAD_ID};

    /// The vendored detector fixture is byte identical to the production detector, so
    /// it stands in for a real catalog file without a download.
    fn detector_bytes() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/whisper.cpp/source/models/for-tests-silero-v6.2.0-ggml.bin");
        std::fs::read(path).expect("the vendored detector fixture")
    }

    fn vad() -> &'static SttCatalogEntry {
        CATALOG.entry(VAD_ID).unwrap()
    }

    fn roots(base: &Path, policy: &DictationPolicy, env: bool) -> Roots {
        Roots::from_parts(
            policy,
            env.then(|| base.join("env").into_os_string()),
            base.join("user"),
            Some(base.join("system")),
        )
    }

    fn managed(dir: &Path, downloads: bool) -> DictationPolicy {
        DictationPolicy {
            managed: true,
            models_dir: Some(dir.to_path_buf()),
            model_downloads: if downloads {
                Rule::Allowed
            } else {
                Rule::Forbidden("downloads off".into())
            },
            ..DictationPolicy::unmanaged()
        }
    }

    fn put(root: &Path, flat: bool, bytes: &[u8]) -> PathBuf {
        let path = if flat {
            models::flat_path(root, vad())
        } else {
            models::model_path(root, vad())
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_lookup_order_is_managed_env_user_system() {
        let base = tempfile::tempdir().unwrap();
        let policy = managed(&base.path().join("managed"), true);
        let roots = roots(base.path(), &policy, true);
        let order: Vec<Location> = roots.ordered().iter().map(|(l, _)| *l).collect();
        assert_eq!(
            order,
            [
                Location::Managed,
                Location::Env,
                Location::User,
                Location::System
            ]
        );
        let bytes = detector_bytes();
        put(&base.path().join("system"), true, &bytes);
        assert_eq!(find(&roots, vad()).unwrap().location, Location::System);
        put(&base.path().join("user"), false, &bytes);
        assert_eq!(find(&roots, vad()).unwrap().location, Location::User);
        put(&base.path().join("env"), true, &bytes);
        assert_eq!(find(&roots, vad()).unwrap().location, Location::Env);
        let managed_copy = put(&base.path().join("managed"), false, &bytes);
        let found = find(&roots, vad()).unwrap();
        assert_eq!(found.location, Location::Managed);
        assert_eq!(found.path, managed_copy);
        assert_eq!(found.sha256, vad().sha256);
        assert!(!found.superseded);
    }

    #[test]
    fn a_wrong_copy_in_an_earlier_folder_is_skipped_and_left_in_place() {
        let base = tempfile::tempdir().unwrap();
        let policy = managed(&base.path().join("managed"), true);
        let roots = roots(base.path(), &policy, true);
        let bytes = detector_bytes();
        let mut wrong = bytes.clone();
        wrong[1000] ^= 1;
        let bad_managed = put(&base.path().join("managed"), true, &wrong);
        let bad_env = put(&base.path().join("env"), false, &wrong);
        put(&base.path().join("user"), false, &bytes);
        assert_eq!(find(&roots, vad()).unwrap().location, Location::User);
        assert!(bad_managed.exists() && bad_env.exists());
    }

    #[test]
    fn with_downloads_off_the_managed_folder_is_the_only_source() {
        let base = tempfile::tempdir().unwrap();
        let policy = managed(&base.path().join("managed"), false);
        let roots = roots(base.path(), &policy, true);
        assert_eq!(
            roots.ordered().iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            [Location::Managed]
        );
        let bytes = detector_bytes();
        put(&base.path().join("user"), false, &bytes);
        put(&base.path().join("system"), false, &bytes);
        assert!(find(&roots, vad()).is_none());
        put(&base.path().join("managed"), true, &bytes);
        assert_eq!(find(&roots, vad()).unwrap().location, Location::Managed);
    }

    #[test]
    fn a_relative_env_folder_is_ignored() {
        let policy = DictationPolicy::unmanaged();
        let roots = Roots::from_parts(
            &policy,
            Some("relative/models".into()),
            PathBuf::from("/u"),
            None,
        );
        assert_eq!(roots.env, None);
        assert!(!roots.managed_only);
        assert_eq!(roots.writable(false), Some(Path::new("/u")));
        assert_eq!(roots.writable(true), None);
    }

    #[test]
    fn the_inventory_lists_what_no_entry_claims() {
        let base = tempfile::tempdir().unwrap();
        let policy = DictationPolicy::unmanaged();
        let roots = roots(base.path(), &policy, false);
        let user = base.path().join("user");
        put(&user, false, &detector_bytes());
        std::fs::create_dir_all(user.join("whisper-tiny")).unwrap();
        std::fs::write(user.join("whisper-tiny").join("x.bin"), b"12345").unwrap();
        std::fs::write(user.join("loose.bin"), b"123").unwrap();
        std::fs::create_dir_all(user.join(models::PARTIAL_DIR)).unwrap();
        let inventory = inventory(&roots, &CATALOG);
        assert_eq!(inventory.found.len(), 1);
        assert_eq!(inventory.found[0].entry.kind, SttKind::Vad);
        assert_eq!(
            inventory.usable().count(),
            0,
            "a detector alone cannot dictate"
        );
        let strays: Vec<(String, u64)> = inventory
            .strays
            .iter()
            .map(|s| {
                (
                    s.path.file_name().unwrap().to_string_lossy().into_owned(),
                    s.bytes,
                )
            })
            .collect();
        assert_eq!(
            strays,
            [
                ("loose.bin".to_string(), 3),
                ("whisper-tiny".to_string(), 5)
            ]
        );
    }

    /// A load's lookup reads one entry at a time and each at most once: a speech model
    /// that is missing ends `usable` before its detector is looked up, and nothing else
    /// in the catalog is read.
    #[test]
    fn a_lookup_finds_only_what_it_is_asked_for() {
        use biorouter_dictation::catalog::SMALL_EN_ID;
        let base = tempfile::tempdir().unwrap();
        let policy = DictationPolicy::unmanaged();
        let roots = roots(base.path(), &policy, false);
        let copy = put(&base.path().join("user"), false, &detector_bytes());
        let mut lookup = Lookup::new(&roots);
        let small = CATALOG.entry(SMALL_EN_ID).unwrap();
        assert!(!lookup.usable(&CATALOG, small));
        assert_eq!(lookup.looked_up(), [SMALL_EN_ID]);
        assert_eq!(lookup.found(vad()).unwrap().location, Location::User);
        // A second ask is answered from what the first found.
        std::fs::remove_file(&copy).unwrap();
        assert!(lookup.found(vad()).is_some());
        assert_eq!(lookup.looked_up(), [SMALL_EN_ID, VAD_ID]);
    }

    /// `Paths::in_local_data_dir("dictation/models")` under a sandbox root, and the
    /// system folder beside it.
    #[test]
    fn the_folders_follow_the_sandbox_root() {
        let root = tempfile::tempdir().unwrap();
        let _env =
            env_lock::lock_env([("BIOROUTER_PATH_ROOT", Some(root.path().to_str().unwrap()))]);
        assert_eq!(
            user_models_dir(),
            root.path().join("data").join("dictation").join("models")
        );
        assert_eq!(
            system_models_dir(),
            Some(root.path().join("system").join("dictation").join("models"))
        );
    }
}
