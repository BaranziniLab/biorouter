//! Explicit, profile-bound credential storage. The persistent selector makes a
//! missing vault an error; no failed vault operation can choose the OS keyring.
use anyhow::{ensure, Context, Result};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::{
    aead::{AeadInPlace, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use zeroize::Zeroizing;

const VERSION: u32 = 1;
const PURPOSE: &str = "biorouter-crew-credentials";
const MAX_PLAINTEXT_BYTES: usize = 1024 * 1024;
const MAX_FILE_BYTES: usize = 2 * (MAX_PLAINTEXT_BYTES + 16) + 4096;
const MAX_ENTRIES: usize = 1024;
const MAX_ID_BYTES: usize = 512;
const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
const MAX_PASSPHRASE_BYTES: usize = 1024;

#[derive(Serialize)]
pub struct CredentialStatus {
    pub backend: &'static str,
    pub initialized: bool,
    pub locked: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    version: u32,
    backend: String,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct KdfParameters {
    algorithm: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    output_bytes: usize,
}

impl KdfParameters {
    fn v1() -> Self {
        Self {
            algorithm: "argon2id".into(),
            version: 0x13,
            memory_kib: 65536,
            iterations: 3,
            parallelism: 1,
            output_bytes: 32,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    purpose: String,
    kdf: KdfParameters,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize)]
struct AssociatedData<'a> {
    version: u32,
    purpose: &'static str,
    profile_identity: &'a str,
    kdf: &'a KdfParameters,
    salt: &'a str,
}

struct Secret(Zeroizing<String>);

struct Contents {
    version: u32,
    credentials: BTreeMap<String, Secret>,
}

struct Unlocked {
    key: Zeroizing<[u8; 32]>,
    salt: String,
    profile_identity: String,
}

#[derive(Default)]
struct State {
    vault_selected: bool,
    legacy_credentials_used: bool,
    unlocked: Option<Unlocked>,
}

pub(super) struct CredentialVault {
    root: PathBuf,
    state: Mutex<State>,
}

impl CredentialVault {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("Crew credential vault lock is unavailable"))
    }

    fn selection_path(&self) -> PathBuf {
        self.root.join("credential-backend.json")
    }

    fn vault_path(&self) -> PathBuf {
        self.root.join("credential-vault.json")
    }

    fn selected(&self, state: &mut State) -> Result<bool> {
        let selection_exists = path_exists(&self.selection_path())?;
        let vault_exists = path_exists(&self.vault_path())?;
        if state.vault_selected || selection_exists || vault_exists {
            state.vault_selected = true;
            ensure!(
                selection_exists && vault_exists,
                "Crew encrypted vault is incomplete or missing; restore its existing files. Keyring fallback and automatic reinitialization are disabled"
            );
            let selection: Selection =
                serde_json::from_slice(&read_bounded(&self.selection_path(), 1024)?)
                    .context("Invalid Crew credential backend selection")?;
            ensure!(
                selection.version == VERSION && selection.backend == "encrypted_vault",
                "Unsupported Crew credential backend selection"
            );
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn status(&self) -> Result<CredentialStatus> {
        let mut state = self.state()?;
        let selected = self.selected(&mut state)?;
        if selected {
            self.read_envelope()?;
        }
        Ok(CredentialStatus {
            backend: if selected {
                "encrypted_vault"
            } else {
                "keyring"
            },
            initialized: selected,
            locked: selected && state.unlocked.is_none(),
        })
    }

    pub(super) fn init(&self, passphrase: Zeroizing<String>) -> Result<()> {
        let mut state = self.state()?;
        validate_passphrase(&passphrase)?;
        ensure!(
            !state.vault_selected
                && !path_exists(&self.selection_path())?
                && !path_exists(&self.vault_path())?,
            "Crew credential vault already exists or requires recovery; use unlock, never initialize over it"
        );
        ensure!(
            !state.legacy_credentials_used,
            "Crew has already used credentials from another backend; initialize the vault in a fresh profile before creating Crew identities"
        );
        let plaintext_path = self.root.join("credentials");
        ensure!(
            !path_exists(&plaintext_path)?,
            "Development plaintext Crew credentials exist; vault initialization does not import or replace them"
        );
        let profile_identity = crate::daemon_runtime::profile_identity()?;
        let mut salt = [0u8; 16];
        OsRng.try_fill_bytes(&mut salt)?;
        let salt = super::hex(&salt);
        let key = derive_key(&passphrase, &salt)?;
        drop(passphrase);
        let unlocked = Unlocked {
            key,
            salt,
            profile_identity,
        };
        let contents = Contents {
            version: VERSION,
            credentials: BTreeMap::new(),
        };
        let envelope = encrypt(&contents, &unlocked)?;
        create_private_directory(&self.root)?;
        let selection = serde_json::to_vec(&Selection {
            version: VERSION,
            backend: "encrypted_vault".into(),
        })?;
        // Persist the opt-in first, so an interrupted initialization can never
        // silently select another credential backend on the next daemon start.
        atomic_write(&self.selection_path(), &selection, false)?;
        state.vault_selected = true;
        atomic_write(&self.vault_path(), &envelope, false)?;
        state.unlocked = Some(unlocked);
        Ok(())
    }

    pub(super) fn unlock(&self, passphrase: Zeroizing<String>) -> Result<()> {
        let mut state = self.state()?;
        state.unlocked = None;
        validate_passphrase(&passphrase)?;
        ensure!(
            self.selected(&mut state)?,
            "Crew uses the OS keyring; initialize an encrypted vault explicitly before unlocking"
        );
        let envelope = self.read_envelope()?;
        let profile_identity = crate::daemon_runtime::profile_identity()?;
        let key = derive_key(&passphrase, &envelope.salt)?;
        drop(passphrase);
        let unlocked = Unlocked {
            key,
            salt: envelope.salt.clone(),
            profile_identity,
        };
        decrypt(&envelope, &unlocked)?;
        state.unlocked = Some(unlocked);
        Ok(())
    }

    pub(super) fn lock(&self) -> Result<()> {
        self.state()?.unlocked = None;
        Ok(())
    }

    pub(super) fn read(
        &self,
        id: &str,
        legacy_read: impl FnOnce() -> Result<Zeroizing<String>>,
    ) -> Result<Zeroizing<String>> {
        let mut state = self.state()?;
        if !self.selected(&mut state)? {
            let value = legacy_read()?;
            state.legacy_credentials_used = true;
            return Ok(value);
        }
        let unlocked = require_unlocked(&state)?;
        let mut contents = decrypt(&self.read_envelope()?, unlocked)?;
        let value = contents
            .credentials
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("Crew credential is absent from the encrypted vault"))?;
        Ok(value.0)
    }

    pub(super) fn write(
        &self,
        id: &str,
        value: &str,
        legacy_write: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let mut state = self.state()?;
        if !self.selected(&mut state)? {
            legacy_write()?;
            state.legacy_credentials_used = true;
            return Ok(());
        }
        validate_entry(id, value)?;
        let unlocked = require_unlocked(&state)?;
        let mut contents = decrypt(&self.read_envelope()?, unlocked)?;
        contents
            .credentials
            .insert(id.to_owned(), Secret(Zeroizing::new(value.to_owned())));
        validate_contents(&contents)?;
        let bytes = encrypt(&contents, unlocked)?;
        atomic_write(&self.vault_path(), &bytes, true)?;
        Ok(())
    }

    /// Remove `id`, from whichever backend this profile selected; an absent entry is not an
    /// error. As with every operation here, a selected vault that is missing or locked is an
    /// error, never a fall back to the keyring.
    pub(super) fn delete(
        &self,
        id: &str,
        legacy_delete: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let mut state = self.state()?;
        if !self.selected(&mut state)? {
            return legacy_delete();
        }
        let unlocked = require_unlocked(&state)?;
        let mut contents = decrypt(&self.read_envelope()?, unlocked)?;
        if contents.credentials.remove(id).is_some() {
            let bytes = encrypt(&contents, unlocked)?;
            atomic_write(&self.vault_path(), &bytes, true)?;
        }
        Ok(())
    }

    fn read_envelope(&self) -> Result<Envelope> {
        let bytes = read_bounded(&self.vault_path(), MAX_FILE_BYTES)?;
        let envelope: Envelope =
            serde_json::from_slice(&bytes).context("Invalid Crew encrypted vault format")?;
        ensure!(
            envelope.version == VERSION
                && envelope.purpose == PURPOSE
                && envelope.kdf == KdfParameters::v1(),
            "Unsupported Crew encrypted vault version or KDF parameters"
        );
        ensure!(
            envelope.salt.len() == 32
                && envelope.nonce.len() == 48
                && envelope.ciphertext.len() >= 32
                && envelope.ciphertext.len() <= 2 * (MAX_PLAINTEXT_BYTES + 16),
            "Invalid Crew encrypted vault bounds"
        );
        super::unhex(&envelope.salt)?;
        super::unhex(&envelope.nonce)?;
        ensure!(
            envelope.ciphertext.len().is_multiple_of(2)
                && envelope.ciphertext.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid Crew encrypted vault ciphertext encoding"
        );
        Ok(envelope)
    }
}

fn require_unlocked(state: &State) -> Result<&Unlocked> {
    state.unlocked.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Crew credential vault is locked; explicitly unlock it for this daemon session"
        )
    })
}

fn validate_passphrase(passphrase: &str) -> Result<()> {
    ensure!(
        !passphrase.is_empty() && passphrase.len() <= MAX_PASSPHRASE_BYTES,
        "Vault passphrase must contain between 1 and 1024 UTF-8 bytes"
    );
    Ok(())
}

fn validate_entry(id: &str, value: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control),
        "Invalid Crew vault credential identifier"
    );
    ensure!(
        !value.is_empty() && value.len() <= MAX_CREDENTIAL_BYTES,
        "Crew vault credential must contain between 1 and 16384 UTF-8 bytes"
    );
    Ok(())
}

fn validate_contents(contents: &Contents) -> Result<()> {
    ensure!(
        contents.version == VERSION && contents.credentials.len() <= MAX_ENTRIES,
        "Invalid Crew vault credential map version or size"
    );
    for (id, value) in &contents.credentials {
        validate_entry(id, &value.0)?;
    }
    Ok(())
}

fn derive_key(passphrase: &str, salt: &str) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(65536, 3, 1, Some(32))
        .map_err(|_| anyhow::anyhow!("Invalid fixed Crew vault KDF parameters"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    let mut memory = Zeroizing::new(vec![Block::default(); params.block_count()]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(
            passphrase.as_bytes(),
            &super::unhex(salt)?,
            &mut *key,
            &mut *memory,
        )
        .map_err(|_| anyhow::anyhow!("Crew vault key derivation failed"))?;
    Ok(key)
}

fn associated_data(envelope: &Envelope, profile_identity: &str) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AssociatedData {
        version: VERSION,
        purpose: PURPOSE,
        profile_identity,
        kdf: &envelope.kdf,
        salt: &envelope.salt,
    })?)
}

fn decrypt(envelope: &Envelope, unlocked: &Unlocked) -> Result<Contents> {
    ensure!(
        unlocked.salt == envelope.salt,
        "Crew encrypted vault changed; lock and unlock it again"
    );
    ensure!(
        crate::daemon_runtime::profile_identity()? == unlocked.profile_identity,
        "Crew runtime profile changed; lock and unlock the vault again"
    );
    let nonce = super::unhex(&envelope.nonce)?;
    let mut plaintext = Zeroizing::new(super::unhex(&envelope.ciphertext)?);
    XChaCha20Poly1305::new((&*unlocked.key).into())
        .decrypt_in_place(
            XNonce::from_slice(&nonce),
            &associated_data(envelope, &unlocked.profile_identity)?,
            &mut *plaintext,
        )
        .map_err(|_| anyhow::anyhow!("Crew vault authentication failed: incorrect passphrase, profile mismatch, or damaged vault"))?;
    ensure!(
        plaintext.len() <= MAX_PLAINTEXT_BYTES,
        "Crew vault exceeds its size limit"
    );
    let contents = decode_contents(&plaintext)?;
    validate_contents(&contents)?;
    Ok(contents)
}

fn encrypt(contents: &Contents, unlocked: &Unlocked) -> Result<Vec<u8>> {
    validate_contents(contents)?;
    let mut plaintext = encode_contents(contents)?;
    let mut nonce = [0u8; 24];
    OsRng.try_fill_bytes(&mut nonce)?;
    let mut envelope = Envelope {
        version: VERSION,
        purpose: PURPOSE.into(),
        kdf: KdfParameters::v1(),
        salt: unlocked.salt.clone(),
        nonce: super::hex(&nonce),
        ciphertext: String::new(),
    };
    XChaCha20Poly1305::new((&*unlocked.key).into())
        .encrypt_in_place(
            XNonce::from_slice(&nonce),
            &associated_data(&envelope, &unlocked.profile_identity)?,
            &mut *plaintext,
        )
        .map_err(|_| anyhow::anyhow!("Crew vault encryption failed"))?;
    envelope.ciphertext = super::hex(&plaintext);
    Ok(serde_json::to_vec(&envelope)?)
}

fn encode_contents(contents: &Contents) -> Result<Zeroizing<Vec<u8>>> {
    let size = 8 + contents
        .credentials
        .iter()
        .map(|(id, value)| 8 + id.len() + value.0.len())
        .sum::<usize>();
    ensure!(
        size <= MAX_PLAINTEXT_BYTES,
        "Crew vault exceeds its size limit"
    );
    // V1 plaintext: little-endian u32 version/count, then u32 byte length plus
    // UTF-8 bytes for each identifier and credential, in identifier order.
    // Exact capacity includes the authentication tag; plaintext never passes
    // through a serializer's non-zeroizing scratch buffers or reallocations.
    let mut bytes = Zeroizing::new(Vec::with_capacity(size + 16));
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(contents.credentials.len() as u32).to_le_bytes());
    for (id, value) in &contents.credentials {
        bytes.extend_from_slice(&(id.len() as u32).to_le_bytes());
        bytes.extend_from_slice(id.as_bytes());
        bytes.extend_from_slice(&(value.0.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value.0.as_bytes());
    }
    Ok(bytes)
}

fn decode_contents(mut bytes: &[u8]) -> Result<Contents> {
    let version = read_u32(&mut bytes)?;
    let count = read_u32(&mut bytes)? as usize;
    ensure!(
        version == VERSION && count <= MAX_ENTRIES,
        "Invalid Crew vault credential map version or size"
    );
    let mut credentials = BTreeMap::new();
    for _ in 0..count {
        let id_len = read_u32(&mut bytes)? as usize;
        ensure!(
            id_len <= MAX_ID_BYTES,
            "Invalid Crew vault credential identifier length"
        );
        let id = std::str::from_utf8(take_bytes(&mut bytes, id_len)?)?.to_owned();
        let value_len = read_u32(&mut bytes)? as usize;
        ensure!(
            value_len <= MAX_CREDENTIAL_BYTES,
            "Invalid Crew vault credential length"
        );
        let value =
            Zeroizing::new(std::str::from_utf8(take_bytes(&mut bytes, value_len)?)?.to_owned());
        validate_entry(&id, &value)?;
        ensure!(
            !credentials.contains_key(&id),
            "Duplicate Crew vault credential identifier"
        );
        credentials.insert(id, Secret(value));
    }
    ensure!(
        bytes.is_empty(),
        "Unexpected trailing Crew vault credential data"
    );
    Ok(Contents {
        version,
        credentials,
    })
}

fn read_u32(bytes: &mut &[u8]) -> Result<u32> {
    let value: [u8; 4] = take_bytes(bytes, 4)?.try_into()?;
    Ok(u32::from_le_bytes(value))
}

fn take_bytes<'a>(bytes: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    ensure!(
        length <= bytes.len(),
        "Truncated Crew vault credential data"
    );
    let (value, rest) = (*bytes).split_at(length);
    *bytes = rest;
    Ok(value)
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "Crew credential files must be regular files, not symlinks"
    );
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() <= max as u64,
        "Crew credential file exceeds its size limit"
    );
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= max,
        "Crew credential file exceeds its size limit"
    );
    Ok(bytes)
}

fn create_private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    let created_parents = {
        let mut parents = Vec::new();
        let mut current = path;
        while !path_exists(current)? {
            let parent = current
                .parent()
                .context("Crew vault directory has no parent")?;
            parents.push(parent.to_path_buf());
            current = parent;
        }
        parents
    };
    fs::create_dir_all(path)?;
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_dir(),
        "Crew credential directory must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        for parent in created_parents {
            File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path.parent().context("Crew vault path has no parent")?;
    ensure!(
        fs::symlink_metadata(parent)?.file_type().is_dir(),
        "Crew credential directory must not be a symlink"
    );
    if replace {
        ensure!(
            fs::symlink_metadata(path)?.file_type().is_file(),
            "Existing Crew vault must be a regular file"
        );
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(path)?;
    } else {
        temporary.persist_noclobber(path)?;
    }
    #[cfg(unix)]
    File::open(parent)?.sync_all().context("Crew vault was written but directory synchronization failed; inspect its status before retrying")?;
    Ok(())
}

#[cfg(test)]
#[path = "credentials_tests.rs"]
mod credentials_tests;
