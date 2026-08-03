//! Face gallery store — encrypted on-disk persistence of feature vectors.
//!
//! Port of the reference `face_hello/store.py` approach:
//! - Stores **feature vectors only** (no photos).
//! - Encrypted with **DPAPI machine scope** (`CRYPTPROTECT_LOCAL_MACHINE`)
//!   + fixed entropy, so the SYSTEM service can decrypt what the admin
//!   console wrote (and vice-versa).
//! - Versioned binary format with a magic header.
//! - Multi-template per profile (same-name "add angle" appends, FIFO cap).
//! - Atomic write (temp file + rename) to survive interrupted saves.
#![allow(clippy::doc_lazy_continuation)]

use crate::hw::face::config::{EMBEDDING_DIM, MAX_TEMPLATES_PER_NAME, STORE_MAGIC};
use crate::hw::face::errors::{FaceError, FaceResult};
use crate::hw::face::FaceSettings;
use serde::{Deserialize, Serialize};

/// One enrolled template (feature vector + metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceTemplate {
    pub name: String,
    pub embedding: Vec<f32>,
    pub enrolled_at_unix: i64,
    pub label: String,
}

/// One profile = one user, with up to N templates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceProfile {
    pub name: String,
    pub templates: Vec<FaceTemplate>,
}

/// The whole gallery.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FaceStore {
    pub profiles: Vec<FaceProfile>,
    pub settings: FaceSettings,
}

impl FaceStore {
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
            settings: FaceSettings::default(),
        }
    }

    /// Serialize to bytes (JSON, before encryption).
    pub fn to_bytes(&self) -> FaceResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| FaceError::Store(format!("serialize: {e}")))
    }

    /// Deserialize from bytes (after decryption).
    pub fn from_bytes(bytes: &[u8]) -> FaceResult<Self> {
        let mut store: FaceStore =
            serde_json::from_slice(bytes).map_err(|e| FaceError::Store(format!("parse: {e}")))?;
        // Sanitize settings against defaults (reject unknown/invalid keys).
        let serialized_settings = serde_json::to_value(&store.settings)
            .map_err(|e| FaceError::Store(format!("settings serialize: {e}")))?;
        let (sanitized, _rejected) =
            FaceSettings::sanitize(&serialized_settings.as_object().cloned().unwrap_or_default());
        store.settings = sanitized;
        Ok(store)
    }

    /// Add or append a template to a profile (FIFO cap).
    pub fn add_template(&mut self, name: &str, embedding: Vec<f32>, label: &str) -> FaceResult<()> {
        if embedding.len() != EMBEDDING_DIM {
            return Err(FaceError::Store(format!(
                "embedding dim {} != {EMBEDDING_DIM}",
                embedding.len()
            )));
        }
        let now = chrono_like_now();
        if let Some(profile) = self.profiles.iter_mut().find(|p| p.name == name) {
            while profile.templates.len() >= MAX_TEMPLATES_PER_NAME {
                profile.templates.remove(0);
            }
            profile.templates.push(FaceTemplate {
                name: name.to_string(),
                embedding,
                enrolled_at_unix: now,
                label: label.to_string(),
            });
        } else {
            self.profiles.push(FaceProfile {
                name: name.to_string(),
                templates: vec![FaceTemplate {
                    name: name.to_string(),
                    embedding,
                    enrolled_at_unix: now,
                    label: label.to_string(),
                }],
            });
        }
        Ok(())
    }

    /// Remove one template by (name, index within profile).
    pub fn remove_template(&mut self, name: &str, index: usize) -> FaceResult<()> {
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| FaceError::Store(format!("profile not found: {name}")))?;
        if index >= profile.templates.len() {
            return Err(FaceError::Store(format!(
                "template index {index} out of range for {name}"
            )));
        }
        profile.templates.remove(index);
        if profile.templates.is_empty() {
            self.profiles.retain(|p| p.name != name);
        }
        Ok(())
    }

    /// All embeddings with their profile names (for matching).
    pub fn flat_gallery(&self) -> (Vec<Vec<f32>>, Vec<String>) {
        let mut gallery = Vec::new();
        let mut names = Vec::new();
        for p in &self.profiles {
            for t in &p.templates {
                gallery.push(t.embedding.clone());
                names.push(t.name.clone());
            }
        }
        (gallery, names)
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

/// Current unix timestamp (seconds). Small helper to avoid a chrono dep.
fn chrono_like_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── DPAPI helpers (Windows) ─────────────────────────────────────────────────

/// Fixed entropy for DPAPI, mirroring the reference `platform_backend.py`.
const DPAPI_ENTROPY: &[u8] = b"micontrol_face_v1";
/// CRYPTPROTECT_LOCAL_MACHINE
#[allow(dead_code)] // used by the windows-only dpapi module
const CRYPTPROTECT_LOCAL_MACHINE: u32 = 0x4;

#[cfg(windows)]
mod dpapi {
    use super::*;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPT_INTEGER_BLOB,
    };

    /// Encrypt `raw` with DPAPI machine scope + entropy.
    pub fn protect(raw: &[u8]) -> FaceResult<Vec<u8>> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: raw.len() as u32,
            pbData: raw.as_ptr() as *mut u8,
        };
        let entropy = DPAPI_ENTROPY;
        let ent_blob = CRYPT_INTEGER_BLOB {
            cbData: entropy.len() as u32,
            pbData: entropy.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB::default();
        // SAFETY: valid pointers; out_blob allocated by DPAPI (we intentionally
        // do not free — small, process-lifetime blobs).
        let hr = unsafe {
            CryptProtectData(
                &in_blob,
                None,
                Some(&ent_blob),
                None,
                None,
                CRYPTPROTECT_LOCAL_MACHINE,
                &mut out_blob,
            )
        };
        if let Err(e) = hr {
            return Err(FaceError::Store(format!("CryptProtectData: {e}")));
        }
        // Copy out before (intentionally) not freeing.
        let out = unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) }
            .to_vec();
        Ok(out)
    }

    /// Decrypt `blob` (DPAPI machine scope + entropy).
    pub fn unprotect(blob: &[u8]) -> FaceResult<Vec<u8>> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: blob.len() as u32,
            pbData: blob.as_ptr() as *mut u8,
        };
        let entropy = DPAPI_ENTROPY;
        let ent_blob = CRYPT_INTEGER_BLOB {
            cbData: entropy.len() as u32,
            pbData: entropy.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB::default();
        // SAFETY: valid pointers; out_blob allocated by DPAPI.
        let hr = unsafe {
            CryptUnprotectData(
                &in_blob,
                None,
                Some(&ent_blob),
                None,
                None,
                0,
                &mut out_blob,
            )
        };
        if let Err(e) = hr {
            return Err(FaceError::Store(format!("CryptUnprotectData: {e}")));
        }
        let out = unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) }
            .to_vec();
        Ok(out)
    }
}

/// Save the store to `path` (DPAPI-encrypted, atomic).
pub fn save_store(path: &std::path::Path, store: &FaceStore) -> FaceResult<()> {
    let raw = store.to_bytes()?;
    let encrypted = dpapi::protect(&raw)?;
    let mut full = Vec::with_capacity(STORE_MAGIC.len() + encrypted.len());
    full.extend_from_slice(STORE_MAGIC);
    full.extend_from_slice(&encrypted);

    // Atomic write: temp + rename.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &full).map_err(|e| FaceError::Store(format!("write tmp: {e}")))?;
    std::fs::rename(&tmp, path).map_err(|e| FaceError::Store(format!("rename: {e}")))?;
    Ok(())
}

/// Load the store from `path`. Returns an empty store if the file is missing.
pub fn load_store(path: &std::path::Path) -> FaceResult<FaceStore> {
    if !path.exists() {
        return Ok(FaceStore::new());
    }
    let full = std::fs::read(path).map_err(|e| FaceError::Store(format!("read: {e}")))?;
    if full.len() < STORE_MAGIC.len() || &full[..STORE_MAGIC.len()] != STORE_MAGIC {
        return Err(FaceError::Store("bad store magic".into()));
    }
    let encrypted = &full[STORE_MAGIC.len()..];
    let raw = dpapi::unprotect(encrypted)?;
    FaceStore::from_bytes(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_flat_gallery() {
        let mut store = FaceStore::new();
        store
            .add_template("alice", vec![0.1; 512], "front")
            .unwrap();
        store.add_template("alice", vec![0.2; 512], "side").unwrap();
        store.add_template("bob", vec![0.3; 512], "front").unwrap();
        let (gallery, names) = store.flat_gallery();
        assert_eq!(gallery.len(), 3);
        assert_eq!(names, vec!["alice", "alice", "bob"]);
    }

    #[test]
    fn fifo_cap_per_name() {
        let mut store = FaceStore::new();
        for i in 0..(MAX_TEMPLATES_PER_NAME + 3) {
            store
                .add_template("alice", vec![i as f32; 512], &format!("t{i}"))
                .unwrap();
        }
        let profile = store.profiles.iter().find(|p| p.name == "alice").unwrap();
        assert_eq!(profile.templates.len(), MAX_TEMPLATES_PER_NAME);
        // Oldest removed: first 3 dropped, labels start at t3.
        assert_eq!(profile.templates[0].label, "t3");
    }

    #[test]
    fn remove_template_removes_profile_when_empty() {
        let mut store = FaceStore::new();
        store
            .add_template("alice", vec![0.1; 512], "front")
            .unwrap();
        store.remove_template("alice", 0).unwrap();
        assert!(store.is_empty());
        assert!(store.remove_template("alice", 0).is_err());
    }

    #[test]
    fn reject_wrong_embedding_dim() {
        let mut store = FaceStore::new();
        assert!(store.add_template("alice", vec![0.1; 10], "front").is_err());
    }

    #[test]
    fn from_bytes_roundtrip_sanitizes_settings() {
        let mut store = FaceStore::new();
        store.settings.match_threshold = 0.66;
        store.settings.language = "pt".into();
        let bytes = store.to_bytes().unwrap();
        let loaded = FaceStore::from_bytes(&bytes).unwrap();
        assert_eq!(loaded.settings.match_threshold, 0.66);
        assert_eq!(loaded.settings.language, "pt");
    }
}
