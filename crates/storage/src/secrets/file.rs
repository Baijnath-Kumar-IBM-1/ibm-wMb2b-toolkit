//! Encrypted-file secret store (fallback when no OS keychain is available).
//!
//! File layout (JSON): a header with the format version, Argon2id parameters
//! and salt, then a random 96-bit nonce and the AES-256-GCM ciphertext of a
//! JSON map `secret id -> value`. The header is bound to the ciphertext as
//! associated data, so tampering with the KDF parameters or version is
//! detected. Every write uses a fresh random nonce and replaces the file
//! atomically (temp file in the same directory, `fsync`, rename) with
//! owner-only permissions on Unix.
//!
//! The passphrase is never stored. Losing it means losing the secrets; the
//! user can re-enter API keys and passwords in the UI.
//!
//! The store is safe for use from multiple threads of one process. It does
//! not coordinate between processes: run one application instance per data
//! directory.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use aes_gcm::aead::{Aead, Nonce, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use trace_core::secrets::{SecretId, SecretStore};
use trace_core::{Error, Result};
use zeroize::Zeroizing;

use crate::fsutil;

const FORMAT_VERSION: u32 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// Passphrases shorter than this are refused when a new file is created.
pub const MIN_PASSPHRASE_CHARS: usize = 12;

// Upper bounds accepted from a file, so a tampered header cannot make the
// application allocate gigabytes or spin for minutes before authentication.
const MAX_M_COST_KIB: u32 = 1_048_576;
const MAX_T_COST: u32 = 10;
const MAX_P_COST: u32 = 16;

fn secret_err(msg: impl fmt::Display) -> Error {
    Error::Secret(format!("encrypted secrets file: {msg}"))
}

/// Argon2id cost parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    /// OWASP minimum recommendation for Argon2id: 19 MiB, 2 iterations, 1 lane.
    fn default() -> Self {
        Self {
            m_cost: 19_456,
            t_cost: 2,
            p_cost: 1,
        }
    }
}

impl KdfParams {
    fn check(&self) -> Result<()> {
        if self.m_cost > MAX_M_COST_KIB || self.t_cost > MAX_T_COST || self.p_cost > MAX_P_COST {
            return Err(secret_err("unsupported key-derivation parameters"));
        }
        Ok(())
    }

    fn derive(&self, passphrase: &SecretString, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
        self.check()?;
        let params = Params::new(self.m_cost, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(secret_err)?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(
                passphrase.expose_secret().as_bytes(),
                salt,
                key.as_mut_slice(),
            )
            .map_err(secret_err)?;
        Ok(key)
    }
}

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    kdf: KdfParams,
    /// Base64 salt.
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct FileDoc {
    #[serde(flatten)]
    header: Header,
    /// Base64 nonce of the latest write.
    nonce: String,
    /// Base64 ciphertext (AES-256-GCM, tag appended).
    ciphertext: String,
}

impl Header {
    /// Canonical bytes used as AEAD associated data.
    fn aad(&self) -> Vec<u8> {
        format!(
            "trace-secrets|v{}|argon2id|m={}|t={}|p={}|salt={}",
            self.version, self.kdf.m_cost, self.kdf.t_cost, self.kdf.p_cost, self.salt
        )
        .into_bytes()
    }
}

struct State {
    header: Header,
    key: Zeroizing<[u8; KEY_LEN]>,
    values: BTreeMap<String, String>,
}

impl Drop for State {
    fn drop(&mut self) {
        // Overwrite secret values before the map is freed.
        for v in self.values.values_mut() {
            zeroize::Zeroize::zeroize(v);
        }
    }
}

pub struct EncryptedFileSecretStore {
    path: PathBuf,
    state: Mutex<State>,
}

impl fmt::Debug for EncryptedFileSecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedFileSecretStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl EncryptedFileSecretStore {
    /// Opens the store, creating the file when it does not exist.
    pub fn open(path: &Path, passphrase: &SecretString) -> Result<Self> {
        Self::open_with(path, passphrase, KdfParams::default())
    }

    /// Like [`open`](Self::open) with explicit KDF parameters for a *new*
    /// file. Existing files always use the parameters recorded in them.
    pub fn open_with(path: &Path, passphrase: &SecretString, new_kdf: KdfParams) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => Self::load(path, passphrase, &bytes),
            Ok(_) => Self::create(path, passphrase, new_kdf),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Self::create(path, passphrase, new_kdf)
            }
            Err(e) => Err(secret_err(format!("cannot read file: {e}"))),
        }
    }

    fn create(path: &Path, passphrase: &SecretString, kdf: KdfParams) -> Result<Self> {
        if passphrase.expose_secret().chars().count() < MIN_PASSPHRASE_CHARS {
            return Err(Error::validation(
                "passphrase",
                format!("must be at least {MIN_PASSPHRASE_CHARS} characters"),
            ));
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fsutil::create_private_dir_all(dir)
                .map_err(|e| secret_err(format!("cannot create directory: {e}")))?;
        }
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|e| secret_err(format!("no random source: {e}")))?;
        let key = kdf.derive(passphrase, &salt)?;
        let store = Self {
            path: path.to_path_buf(),
            state: Mutex::new(State {
                header: Header {
                    version: FORMAT_VERSION,
                    kdf,
                    salt: B64.encode(salt),
                },
                key,
                values: BTreeMap::new(),
            }),
        };
        store.persist(&store.lock())?;
        Ok(store)
    }

    fn load(path: &Path, passphrase: &SecretString, bytes: &[u8]) -> Result<Self> {
        let doc: FileDoc =
            serde_json::from_slice(bytes).map_err(|_| secret_err("file is not valid"))?;
        if doc.header.version != FORMAT_VERSION {
            return Err(secret_err(format!(
                "unsupported format version {}",
                doc.header.version
            )));
        }
        let salt = B64
            .decode(&doc.header.salt)
            .map_err(|_| secret_err("file is not valid"))?;
        let nonce_bytes = B64
            .decode(&doc.nonce)
            .map_err(|_| secret_err("file is not valid"))?;
        let ciphertext = B64
            .decode(&doc.ciphertext)
            .map_err(|_| secret_err("file is not valid"))?;
        let nonce_arr: [u8; NONCE_LEN] = nonce_bytes
            .as_slice()
            .try_into()
            .map_err(|_| secret_err("file is not valid"))?;
        let key = doc.header.kdf.derive(passphrase, &salt)?;
        let cipher =
            Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| secret_err("bad key"))?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    &Nonce::<Aes256Gcm>::from(nonce_arr),
                    Payload {
                        msg: &ciphertext,
                        aad: &doc.header.aad(),
                    },
                )
                .map_err(|_| secret_err("wrong passphrase, or the file is damaged"))?,
        );
        let values: BTreeMap<String, String> =
            serde_json::from_slice(&plaintext).map_err(|_| secret_err("file is not valid"))?;
        Ok(Self {
            path: path.to_path_buf(),
            state: Mutex::new(State {
                header: doc.header,
                key,
                values,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Encrypts `state.values` with a fresh nonce and replaces the file.
    fn persist(&self, state: &State) -> Result<()> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&state.values).map_err(|_| secret_err("cannot serialise"))?,
        );
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| secret_err(format!("no random source: {e}")))?;
        let cipher =
            Aes256Gcm::new_from_slice(state.key.as_slice()).map_err(|_| secret_err("bad key"))?;
        let ciphertext = cipher
            .encrypt(
                &Nonce::<Aes256Gcm>::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &state.header.aad(),
                },
            )
            .map_err(|_| secret_err("encryption failed"))?;
        let doc = FileDoc {
            header: Header {
                version: state.header.version,
                kdf: state.header.kdf,
                salt: state.header.salt.clone(),
            },
            nonce: B64.encode(nonce),
            ciphertext: B64.encode(ciphertext),
        };
        let json = serde_json::to_vec(&doc).map_err(|_| secret_err("cannot serialise"))?;
        write_atomically(&self.path, &json)
    }

    #[cfg(test)]
    fn salt(&self) -> Vec<u8> {
        B64.decode(&self.lock().header.salt).expect("salt")
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let io = |e: std::io::Error| secret_err(format!("cannot write file: {e}"));
    let mut file = fsutil::create_private_file(&tmp).map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(io)?;
    // Best effort: persist the rename itself.
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

impl SecretStore for EncryptedFileSecretStore {
    fn put(&self, id: &SecretId, value: &SecretString) -> Result<()> {
        let mut state = self.lock();
        let previous = state
            .values
            .insert(id.as_str().to_string(), value.expose_secret().to_string());
        if let Err(e) = self.persist(&state) {
            // Keep memory and disk in step.
            match previous {
                Some(p) => state.values.insert(id.as_str().to_string(), p),
                None => state.values.remove(id.as_str()),
            };
            return Err(e);
        }
        Ok(())
    }

    fn get(&self, id: &SecretId) -> Result<Option<SecretString>> {
        Ok(self
            .lock()
            .values
            .get(id.as_str())
            .map(|v| SecretString::from(v.clone())))
    }

    fn delete(&self, id: &SecretId) -> Result<()> {
        let mut state = self.lock();
        let Some(previous) = state.values.remove(id.as_str()) else {
            return Ok(());
        };
        if let Err(e) = self.persist(&state) {
            state.values.insert(id.as_str().to_string(), previous);
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters so tests run quickly.
    const FAST: KdfParams = KdfParams {
        m_cost: 8,
        t_cost: 1,
        p_cost: 1,
    };

    fn pass(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    fn open(path: &Path, p: &str) -> Result<EncryptedFileSecretStore> {
        EncryptedFileSecretStore::open_with(path, &pass(p), FAST)
    }

    const GOOD: &str = "correct horse battery";

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        let id = SecretId::generate();
        {
            let s = open(&path, GOOD).unwrap();
            s.put(&id, &pass("api-key-value")).unwrap();
        }
        let s = open(&path, GOOD).unwrap();
        assert_eq!(
            s.get(&id).unwrap().unwrap().expose_secret(),
            "api-key-value"
        );
    }

    #[test]
    fn file_never_contains_plaintext_or_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        let id = SecretId::generate();
        let s = open(&path, GOOD).unwrap();
        s.put(&id, &pass("very-secret-value-123")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("very-secret-value-123"));
        assert!(!raw.contains(id.as_str()));
        assert!(!raw.contains(GOOD));
    }

    #[test]
    fn wrong_passphrase_is_rejected_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        open(&path, GOOD)
            .unwrap()
            .put(&SecretId::generate(), &pass("v"))
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        let err = open(&path, "another passphrase!!").err().unwrap();
        assert!(err.to_string().contains("wrong passphrase"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn tampering_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        open(&path, GOOD)
            .unwrap()
            .put(&SecretId::generate(), &pass("v"))
            .unwrap();

        // Flip a ciphertext byte.
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut ct = B64.decode(doc["ciphertext"].as_str().unwrap()).unwrap();
        ct[0] ^= 1;
        doc["ciphertext"] = B64.encode(ct).into();
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(open(&path, GOOD).is_err());
    }

    #[test]
    fn changing_kdf_header_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        open(&path, GOOD).unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        doc["t_cost"] = 1.into();
        doc["kdf"]["t_cost"] = 2.into();
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(open(&path, GOOD).is_err());
    }

    #[test]
    fn absurd_kdf_parameters_are_refused_before_deriving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        open(&path, GOOD).unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        doc["kdf"]["m_cost"] = u32::MAX.into();
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let err = open(&path, GOOD).err().unwrap();
        assert!(err.to_string().contains("unsupported key-derivation"));
    }

    #[test]
    fn garbage_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        std::fs::write(&path, b"not json").unwrap();
        assert!(open(&path, GOOD).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"not json",
            "must not overwrite"
        );
    }

    #[test]
    fn short_passphrase_is_refused_for_new_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        assert!(open(&path, "short").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn delete_removes_and_missing_delete_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        let id = SecretId::generate();
        let s = open(&path, GOOD).unwrap();
        s.put(&id, &pass("v")).unwrap();
        s.delete(&id).unwrap();
        s.delete(&id).unwrap();
        drop(s);
        assert!(open(&path, GOOD).unwrap().get(&id).unwrap().is_none());
    }

    #[test]
    fn each_write_uses_a_fresh_nonce_and_keeps_the_salt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        let s = open(&path, GOOD).unwrap();
        let salt = s.salt();
        let nonce = |p: &Path| {
            let d: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
            d["nonce"].as_str().unwrap().to_string()
        };
        let n0 = nonce(&path);
        s.put(&SecretId::generate(), &pass("a")).unwrap();
        let n1 = nonce(&path);
        s.put(&SecretId::generate(), &pass("b")).unwrap();
        let n2 = nonce(&path);
        assert!(n0 != n1 && n1 != n2 && n0 != n2);
        assert_eq!(s.salt(), salt);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private_and_no_temp_file_remains() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        let s = open(&path, GOOD).unwrap();
        s.put(&SecretId::generate(), &pass("a")).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!dir.path().join("secrets.enc.tmp").exists());
    }

    #[test]
    fn default_parameters_work_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.enc");
        let id = SecretId::generate();
        EncryptedFileSecretStore::open(&path, &pass(GOOD))
            .unwrap()
            .put(&id, &pass("v"))
            .unwrap();
        let again = EncryptedFileSecretStore::open(&path, &pass(GOOD)).unwrap();
        assert!(again.get(&id).unwrap().is_some());
    }

    #[test]
    fn debug_output_has_no_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir.path().join("secrets.enc"), GOOD).unwrap();
        s.put(&SecretId::generate(), &pass("hunter2hunter2"))
            .unwrap();
        let shown = format!("{s:?}");
        assert!(!shown.contains("hunter2") && !shown.contains(GOOD));
    }
}
