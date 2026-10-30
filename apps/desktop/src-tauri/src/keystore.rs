//! Encrypted Local Keystore for Solana Keypairs.
//!
//! Security Properties:
//! - Keys are encrypted at rest using AES-256-GCM.
//! - Password key derivation uses Argon2id (memory-hard, GPU-resistant).
//! - Ephemeral secret buffers are zeroized from memory immediately after use.
//! - Private keys never leave this module and are never exposed via IPC to the frontend.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("Argon2 key derivation failed: {0}")]
    KeyDerivation(String),
    #[error("Decryption failed: incorrect password or corrupted file")]
    DecryptionFailed,
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Invalid keypair bytes: {0}")]
    InvalidKeypair(String),
    #[error("Public key mismatch: expected {expected}, recovered {recovered}")]
    PubkeyMismatch { expected: String, recovered: String },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Base64 error: {0}")]
    Base64(String),
}

/// Encrypted keystore format stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedKeystore {
    pub version: u32,
    pub pubkey: String,
    pub label: String,
    pub salt_base64: String,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
}

/// Sanitized wallet metadata safe to send to the frontend UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletSummary {
    pub pubkey: String,
    pub label: String,
    pub is_default: bool,
}

/// Derive a 32-byte encryption key from a password and salt using Argon2id.
fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
    let mut key = Zeroizing::new([0u8; 32]);
    let argon2 = Argon2::default();

    argon2
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| KeystoreError::KeyDerivation(e.to_string()))?;

    Ok(key)
}

/// Encrypt a Solana keypair with a user-supplied password.
pub fn encrypt_keypair(
    keypair: &Keypair,
    label: &str,
    password: &str,
) -> Result<EncryptedKeystore, KeystoreError> {
    let mut rng = rand::thread_rng();

    // 1. Generate 32-byte random salt for Argon2id
    let mut salt = [0u8; 32];
    rng.fill_bytes(&mut salt);

    // 2. Derive 256-bit AES key
    let derived_key = derive_key(password, &salt)?;

    // 3. Generate 12-byte random nonce for AES-256-GCM
    let mut nonce_bytes = [0u8; 12];
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // 4. Encrypt the 64-byte keypair buffer
    let cipher = Aes256Gcm::new_from_slice(derived_key.as_ref())
        .map_err(|e| KeystoreError::EncryptionFailed(e.to_string()))?;

    let raw_keypair_bytes = Zeroizing::new(keypair.to_bytes());
    let ciphertext = cipher
        .encrypt(nonce, raw_keypair_bytes.as_ref())
        .map_err(|e| KeystoreError::EncryptionFailed(e.to_string()))?;

    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    Ok(EncryptedKeystore {
        version: 1,
        pubkey: keypair.pubkey().to_string(),
        label: label.to_string(),
        salt_base64: STANDARD.encode(&salt),
        nonce_base64: STANDARD.encode(&nonce_bytes),
        ciphertext_base64: STANDARD.encode(&ciphertext),
    })
}

/// Decrypt an encrypted keystore back into a signing Keypair.
pub fn decrypt_keypair(
    keystore: &EncryptedKeystore,
    password: &str,
) -> Result<Keypair, KeystoreError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let salt = STANDARD
        .decode(&keystore.salt_base64)
        .map_err(|e| KeystoreError::Base64(e.to_string()))?;
    let nonce_bytes = STANDARD
        .decode(&keystore.nonce_base64)
        .map_err(|e| KeystoreError::Base64(e.to_string()))?;
    let ciphertext = STANDARD
        .decode(&keystore.ciphertext_base64)
        .map_err(|e| KeystoreError::Base64(e.to_string()))?;

    if nonce_bytes.len() != 12 {
        return Err(KeystoreError::DecryptionFailed);
    }

    let derived_key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(derived_key.as_ref())
        .map_err(|e| KeystoreError::EncryptionFailed(e.to_string()))?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    let decrypted_bytes = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| KeystoreError::DecryptionFailed)?;

    let zeroized_decrypted = Zeroizing::new(decrypted_bytes);
    let keypair = Keypair::try_from(zeroized_decrypted.as_slice())
        .map_err(|e| KeystoreError::InvalidKeypair(e.to_string()))?;

    let recovered_pubkey = keypair.pubkey().to_string();
    if recovered_pubkey != keystore.pubkey {
        return Err(KeystoreError::PubkeyMismatch {
            expected: keystore.pubkey.clone(),
            recovered: recovered_pubkey,
        });
    }

    Ok(keypair)
}

/// Save encrypted keystore JSON to disk.
pub fn save_keystore(keystore: &EncryptedKeystore, dir: &Path) -> Result<PathBuf, KeystoreError> {
    fs::create_dir_all(dir)?;
    let filename = format!("{}.json", keystore.pubkey);
    let path = dir.join(filename);
    let json = serde_json::to_string_pretty(keystore)
        .map_err(|e| KeystoreError::EncryptionFailed(e.to_string()))?;
    fs::write(&path, json)?;
    Ok(path)
}

/// Load an encrypted keystore by public key from directory.
pub fn load_keystore(pubkey: &str, dir: &Path) -> Result<EncryptedKeystore, KeystoreError> {
    let filename = format!("{}.json", pubkey);
    let path = dir.join(filename);
    let content = fs::read_to_string(path)?;
    let keystore: EncryptedKeystore =
        serde_json::from_str(&content).map_err(|e| KeystoreError::InvalidKeypair(e.to_string()))?;
    Ok(keystore)
}

/// List all available wallet summaries in the directory.
pub fn list_wallets(dir: &Path) -> Result<Vec<WalletSummary>, KeystoreError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut wallets = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(keystore) = serde_json::from_str::<EncryptedKeystore>(&content) {
                    wallets.push(WalletSummary {
                        pubkey: keystore.pubkey,
                        label: keystore.label,
                        is_default: wallets.is_empty(),
                    });
                }
            }
        }
    }
    Ok(wallets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey().to_string();
        let password = "super-secret-password-123!";

        // Encrypt
        let keystore = encrypt_keypair(&keypair, "Primary Trading", password).unwrap();
        assert_eq!(keystore.pubkey, pubkey);

        // Decrypt with correct password
        let recovered = decrypt_keypair(&keystore, password).unwrap();
        assert_eq!(recovered.pubkey().to_string(), pubkey);

        // Decrypt with wrong password must fail
        let wrong = decrypt_keypair(&keystore, "wrong-password");
        assert!(wrong.is_err());
    }

    #[test]
    fn test_import_key_formats_roundtrip() {
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey().to_string();

        // 64-byte full keypair in base58
        let full_b58 = bs58::encode(keypair.to_bytes()).into_string();
        let decoded_64 = bs58::decode(&full_b58).into_vec().unwrap();
        let kp64 = Keypair::try_from(decoded_64.as_slice()).unwrap();
        assert_eq!(kp64.pubkey().to_string(), pubkey);

        // 32-byte seed in base58
        let seed: [u8; 32] = *keypair.secret_bytes();
        let seed_b58 = bs58::encode(seed).into_string();
        let decoded_32 = bs58::decode(&seed_b58).into_vec().unwrap();
        let seed_arr: [u8; 32] = decoded_32.as_slice().try_into().unwrap();
        let kp32 = Keypair::new_from_array(seed_arr);
        assert_eq!(kp32.pubkey().to_string(), pubkey);

        // JSON byte array
        let json_arr = serde_json::to_string(&keypair.to_bytes().to_vec()).unwrap();
        let decoded_json: Vec<u8> = serde_json::from_str(&json_arr).unwrap();
        let kp_json = Keypair::try_from(decoded_json.as_slice()).unwrap();
        assert_eq!(kp_json.pubkey().to_string(), pubkey);
    }
}
