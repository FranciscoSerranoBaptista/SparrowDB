use super::{Role, TokenError, TokenRecord};
use heed3::{Database, Env, EnvOpenOptions, types::Bytes};
use sha2::{Digest, Sha256};
use std::{fs, time::{SystemTime, UNIX_EPOCH}};
use subtle::ConstantTimeEq;

// ---------------------------------------------------------------------------
// Inline hex helpers (no `hex` crate dependency)
// ---------------------------------------------------------------------------

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// TokenStore
// ---------------------------------------------------------------------------

pub struct TokenStore {
    env: Env,
    db: Database<Bytes, Bytes>,
}

impl TokenStore {
    /// Open (or create) the token store at `path`.
    pub fn open(path: &str) -> Result<Self, TokenError> {
        fs::create_dir_all(path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024) // 10 MiB — plenty for tokens
                .max_dbs(4)
                .max_readers(128)
                .open(std::path::Path::new(path))?
        };

        let mut wtxn = env.write_txn()?;
        let db: Database<Bytes, Bytes> = env
            .database_options()
            .types::<Bytes, Bytes>()
            .name("tokens")
            .create(&mut wtxn)?;
        wtxn.commit()?;

        Ok(Self { env, db })
    }

    /// SHA-256 hash of `raw_key` as a fixed 32-byte array.
    fn hash_key(raw_key: &str) -> [u8; 32] {
        let h = Sha256::digest(raw_key.as_bytes());
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&h);
        arr
    }

    /// Returns `true` if at least one token exists (auth is enforced).
    pub fn is_auth_required(&self) -> bool {
        let rtxn = match self.env.read_txn() {
            Ok(t) => t,
            Err(_) => return false,
        };
        match self.db.first(&rtxn) {
            Ok(entry) => entry.is_some(),
            Err(_) => false,
        }
    }

    /// Verify a raw token string.  Returns the associated `TokenRecord` on
    /// success, or `TokenError::Unauthorized` / `TokenError::InvalidKey` on
    /// failure.
    ///
    /// Token format rules:
    ///  - If the key starts with `sparrow_`: must be exactly `sparrow_` + 32
    ///    lowercase hex chars.  Any deviation is `TokenError::InvalidKey`.
    ///  - Any other string (legacy `SPARROW_API_KEY` values): validated only
    ///    by looking up the SHA-256 hash; `Unauthorized` if not found.
    pub fn verify(&self, raw_key: &str) -> Result<TokenRecord, TokenError> {
        if let Some(hex_part) = raw_key.strip_prefix("sparrow_") {
            // New-style token path: enforce exact format.
            if hex_part.len() != 32 {
                return Err(TokenError::InvalidKey);
            }
            // Fix 3: reject uppercase hex
            if !hex_part.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(TokenError::InvalidKey);
            }
        }
        // Fix 1: constant-time comparison — iterate all entries using
        // subtle::ConstantTimeEq so comparison time does not leak which byte
        // differs (prevents timing side-channel attacks).
        let candidate_hash = Self::hash_key(raw_key);
        let rtxn = self.env.read_txn()?;
        for result in self.db.iter(&rtxn)? {
            let (stored_key, value) = result?;
            if stored_key.len() == 32 {
                let mut stored = [0u8; 32];
                stored.copy_from_slice(stored_key);
                if bool::from(candidate_hash.ct_eq(&stored)) {
                    return Ok(serde_json::from_slice(value)?);
                }
            }
        }
        Err(TokenError::Unauthorized)
    }

    /// Create a new named token with the given role.
    ///
    /// Returns `(raw_token_string, record)`.  The raw token is shown only
    /// once — it is never stored.
    pub fn create(&self, name: &str, role: Role) -> Result<(String, TokenRecord), TokenError> {
        // Generate 16 random bytes → 32-char hex payload.
        let random_bytes: [u8; 16] = rand::random();
        let payload_hex = bytes_to_hex(&random_bytes);
        let raw_token = format!("sparrow_{payload_hex}");

        let hash = Self::hash_key(&raw_token);
        let id = bytes_to_hex(&hash[..4]); // first 4 bytes → 8 hex chars

        let record = TokenRecord {
            id,
            name: name.to_string(),
            role,
            created_at: unix_now(),
        };

        let value = serde_json::to_vec(&record)?;

        let mut wtxn = self.env.write_txn()?;
        self.db.put(&mut wtxn, &hash, &value)?;
        wtxn.commit()?;

        Ok((raw_token, record))
    }

    /// List all stored token records.
    pub fn list(&self) -> Result<Vec<TokenRecord>, TokenError> {
        let rtxn = self.env.read_txn()?;
        let mut records = Vec::new();
        let iter = self.db.iter(&rtxn)?;
        for result in iter {
            let (_key, value) = result?;
            let record: TokenRecord = serde_json::from_slice(value)?;
            records.push(record);
        }
        Ok(records)
    }

    /// Revoke a token by its 8-char hex id.
    ///
    /// Returns `true` if found and deleted, `false` if not found.
    pub fn revoke(&self, id: &str) -> Result<bool, TokenError> {
        let rtxn = self.env.read_txn()?;

        // Scan for the record matching the given id.
        let mut found_key: Option<Vec<u8>> = None;
        {
            let iter = self.db.iter(&rtxn)?;
            for result in iter {
                let (key, value) = result?;
                let record: TokenRecord = serde_json::from_slice(value)?;
                if record.id == id {
                    found_key = Some(key.to_vec());
                    break;
                }
            }
        }
        drop(rtxn);

        match found_key {
            None => Ok(false),
            Some(key) => {
                let mut wtxn = self.env.write_txn()?;
                self.db.delete(&mut wtxn, &key)?;
                wtxn.commit()?;
                Ok(true)
            }
        }
    }

    /// Seed `raw_key` as an Admin token named `"SPARROW_API_KEY"`.
    ///
    /// Unconditional write — LMDB `put` on an identical key is idempotent
    /// (last writer wins, same value either way), so this is safe to call
    /// multiple times and eliminates the TOCTOU race of a read-then-write.
    /// Silently ignores errors (best-effort migration).
    pub fn seed_legacy(&self, raw_key: &str) {
        let hash = Self::hash_key(raw_key);
        let record = TokenRecord {
            id: bytes_to_hex(&hash[..4]),
            name: "SPARROW_API_KEY".to_string(),
            role: Role::Admin,
            created_at: unix_now(),
        };
        if let Ok(value) = serde_json::to_vec(&record) {
            if let Ok(mut wtxn) = self.env.write_txn() {
                let _ = self.db.put(&mut wtxn, &hash, &value);
                let _ = wtxn.commit();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "lmdb"))]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TokenStore, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path().to_str().unwrap()).unwrap();
        (store, dir)
    }

    #[test]
    fn test_auth_disabled_when_empty() {
        let (store, _dir) = temp_store();
        assert!(!store.is_auth_required());
    }

    #[test]
    fn test_create_and_verify_token() {
        let (store, _dir) = temp_store();
        let (raw_token, record) = store.create("test-token", Role::ReadWrite).unwrap();
        assert!(raw_token.starts_with("sparrow_"));
        assert_eq!(raw_token.len(), 8 + 32); // "sparrow_" + 32 hex chars

        let verified = store.verify(&raw_token).unwrap();
        assert_eq!(verified.id, record.id);
        assert_eq!(verified.name, "test-token");
        assert_eq!(verified.role, Role::ReadWrite);
    }

    #[test]
    fn test_invalid_token_rejected() {
        let (store, _dir) = temp_store();
        // This has correct prefix but wrong length hex part
        let result = store.verify("sparrow_bad_key");
        assert!(matches!(result, Err(TokenError::InvalidKey)));
    }

    #[test]
    fn test_list_tokens() {
        let (store, _dir) = temp_store();
        store.create("token-one", Role::ReadOnly).unwrap();
        store.create("token-two", Role::Admin).unwrap();
        let tokens = store.list().unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_revoke_token() {
        let (store, _dir) = temp_store();
        let (raw_token, record) = store.create("to-revoke", Role::ReadOnly).unwrap();

        // Revoke by id — should return true.
        let deleted = store.revoke(&record.id).unwrap();
        assert!(deleted);

        // Verify now fails.
        let result = store.verify(&raw_token);
        assert!(matches!(result, Err(TokenError::Unauthorized)));

        // Revoking again returns false.
        let deleted_again = store.revoke(&record.id).unwrap();
        assert!(!deleted_again);
    }

    #[test]
    fn test_legacy_sparrow_api_key_seeded_as_admin() {
        let (store, _dir) = temp_store();
        // Legacy SPARROW_API_KEY values are arbitrary strings (no sparrow_ prefix).
        let legacy_key = "my-legacy-secret-key";
        store.seed_legacy(legacy_key);

        let record = store.verify(legacy_key).unwrap();
        assert_eq!(record.role, Role::Admin);
        assert_eq!(record.name, "SPARROW_API_KEY");
    }
}
