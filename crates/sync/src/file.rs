use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::{Snapshot, SyncError};

const MAGIC: &[u8; 8] = b"TARNSYNC";
const VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;
pub const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 8 + 2 + 1 + 12 + SALT_LEN + NONCE_LEN;
const TAG_LEN: usize = 16;
pub const MAX_FILE_LEN: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    pub const DEFAULT: Self = Self { memory_kib: 128 * 1024, iterations: 3, parallelism: 1 };
    pub const MIN: Self = Self { memory_kib: 64 * 1024, iterations: 3, parallelism: 1 };
    /// Low enough that a planted header cannot make the passphrase prompt exhaust memory or stall.
    pub const MAX: Self = Self { memory_kib: 256 * 1024, iterations: 8, parallelism: 4 };

    fn check(self, min: Self) -> Result<(), SyncError> {
        let max = Self::MAX;
        if self.memory_kib < min.memory_kib || self.iterations < min.iterations || self.parallelism < min.parallelism {
            Err(SyncError::WeakKdf)
        } else if self.memory_kib > max.memory_kib || self.iterations > max.iterations || self.parallelism > max.parallelism {
            Err(SyncError::CostlyKdf)
        } else {
            Ok(())
        }
    }
}

/// The key of one sync set: Argon2id over the passphrase and the set's salt, derived once per
/// session so the passphrase can be dropped and opening a file costs no key derivation.
/// Zeroized on drop.
pub struct SyncKey {
    cipher: XChaCha20Poly1305,
    salt: [u8; SALT_LEN],
    params: KdfParams,
}

impl SyncKey {
    pub fn derive(passphrase: &str, salt: [u8; SALT_LEN], params: KdfParams) -> Result<Self, SyncError> {
        Self::derive_with(passphrase, salt, params, KdfParams::MIN)
    }

    fn derive_with(passphrase: &str, salt: [u8; SALT_LEN], params: KdfParams, min: KdfParams) -> Result<Self, SyncError> {
        params.check(min)?;
        let argon_params = Params::new(params.memory_kib, params.iterations, params.parallelism, Some(32)).map_err(|_| SyncError::WeakKdf)?;
        let mut key = Zeroizing::new([0u8; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params)
            .hash_password_into(passphrase.as_bytes(), &salt, key.as_mut())
            .map_err(|_| SyncError::WeakKdf)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| SyncError::WeakKdf)?;
        Ok(Self { cipher, salt, params })
    }

    /// Salt for a new sync set. Devices joining a set take it from any existing file ([`header_kdf`]).
    pub fn new_salt() -> Result<[u8; SALT_LEN], SyncError> {
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|_| SyncError::Rng)?;
        Ok(salt)
    }
}

struct Header<'a> {
    bytes: &'a [u8],
    params: KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
}

fn parse(bytes: &[u8]) -> Result<(Header<'_>, &[u8]), SyncError> {
    if bytes.len() > MAX_FILE_LEN {
        return Err(SyncError::TooLarge);
    }
    if bytes.len() < HEADER_LEN + TAG_LEN {
        return Err(SyncError::Truncated);
    }
    let (header, ciphertext) = bytes.split_at(HEADER_LEN);
    if &header[..8] != MAGIC {
        return Err(SyncError::BadMagic);
    }
    if u16::from_le_bytes([header[8], header[9]]) != VERSION || header[10] != KDF_ARGON2ID {
        return Err(SyncError::UnsupportedVersion);
    }
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().expect("4 bytes"));
    let params = KdfParams { memory_kib: word(11), iterations: word(15), parallelism: word(19) };
    let salt = header[23..23 + SALT_LEN].try_into().expect("salt length");
    let nonce = header[23 + SALT_LEN..].try_into().expect("nonce length");
    Ok((Header { bytes: header, params, salt, nonce }, ciphertext))
}

/// KDF parameters and salt from a file's unauthenticated header, for joining an existing sync set.
/// Adopt them only after [`open_with_key`] with the key derived from them succeeds on that file.
pub fn header_kdf(bytes: &[u8]) -> Result<(KdfParams, [u8; SALT_LEN]), SyncError> {
    let (header, _) = parse(bytes)?;
    header.params.check(KdfParams::MIN)?;
    Ok((header.params, header.salt))
}

/// Encrypts `snapshot` under `key` with a fresh random nonce.
pub fn seal_with_key(snapshot: &Snapshot, key: &SyncKey) -> Result<Vec<u8>, SyncError> {
    let plaintext = Zeroizing::new(serde_json::to_vec(snapshot).map_err(|_| SyncError::Malformed)?);
    if HEADER_LEN + plaintext.len() + TAG_LEN > MAX_FILE_LEN {
        return Err(SyncError::TooLarge);
    }
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| SyncError::Rng)?;

    let mut out = Vec::with_capacity(HEADER_LEN + plaintext.len() + TAG_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.push(KDF_ARGON2ID);
    for v in [key.params.memory_kib, key.params.iterations, key.params.parallelism] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&key.salt);
    out.extend_from_slice(&nonce);
    let ciphertext = key.cipher.encrypt(&XNonce::from(nonce), Payload { msg: &plaintext, aad: &out }).map_err(|_| SyncError::TooLarge)?;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts a file written under `key`. A file from another sync set or passphrase fails with
/// [`SyncError::KeyMismatch`] without any key derivation. The result is untrusted until passed
/// through [`crate::validate`].
pub fn open_with_key(bytes: &[u8], key: &SyncKey) -> Result<Snapshot, SyncError> {
    let (header, ciphertext) = parse(bytes)?;
    if header.params != key.params || header.salt != key.salt {
        return Err(SyncError::KeyMismatch);
    }
    let plaintext =
        Zeroizing::new(key.cipher.decrypt(&XNonce::from(header.nonce), Payload { msg: ciphertext, aad: header.bytes }).map_err(|_| SyncError::Decrypt)?);
    serde_json::from_slice(&plaintext).map_err(|_| SyncError::Malformed)
}

/// One-shot [`seal_with_key`] under a new sync set (fresh salt).
pub fn seal(snapshot: &Snapshot, passphrase: &str, params: KdfParams) -> Result<Vec<u8>, SyncError> {
    seal_with_key(snapshot, &SyncKey::derive(passphrase, SyncKey::new_salt()?, params)?)
}

/// One-shot [`open_with_key`]: derives the key from the file's header. Parameters outside
/// [`KdfParams::MIN`]..=[`KdfParams::MAX`] are rejected before derivation.
pub fn open(bytes: &[u8], passphrase: &str) -> Result<Snapshot, SyncError> {
    let (params, salt) = header_kdf(bytes)?;
    open_with_key(bytes, &SyncKey::derive(passphrase, salt, params)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST: KdfParams = KdfParams { memory_kib: 64, iterations: 1, parallelism: 1 };

    fn key(passphrase: &str) -> SyncKey {
        SyncKey::derive_with(passphrase, [7; SALT_LEN], TEST, TEST).unwrap()
    }

    fn sealed() -> Vec<u8> {
        seal_with_key(&crate::tests::sample(), &key("correct horse")).unwrap()
    }

    #[test]
    fn roundtrip() {
        assert_eq!(open_with_key(&sealed(), &key("correct horse")).unwrap(), crate::tests::sample());
    }

    #[test]
    fn fresh_nonce_per_seal() {
        assert_ne!(sealed()[23 + SALT_LEN..HEADER_LEN], sealed()[23 + SALT_LEN..HEADER_LEN]);
    }

    #[test]
    fn wrong_passphrase() {
        assert_eq!(open_with_key(&sealed(), &key("wrong horse")), Err(SyncError::Decrypt));
    }

    #[test]
    fn every_tampered_byte_fails() {
        let file = sealed();
        let key = key("correct horse");
        for i in 0..file.len() {
            let mut bad = file.clone();
            bad[i] ^= 1;
            assert!(open_with_key(&bad, &key).is_err(), "byte {i}");
        }
    }

    #[test]
    fn other_salt_or_params_is_key_mismatch() {
        let other = SyncKey::derive_with("correct horse", [8; SALT_LEN], TEST, TEST).unwrap();
        assert_eq!(open_with_key(&sealed(), &other), Err(SyncError::KeyMismatch));
        let mut file = sealed();
        file[15] = 2;
        assert_eq!(open_with_key(&file, &key("correct horse")), Err(SyncError::KeyMismatch));
    }

    #[test]
    fn planted_header_is_not_adoptable() {
        let mut file = sealed();
        file[23] ^= 1;
        let (params, salt) = (TEST, file[23..23 + SALT_LEN].try_into().unwrap());
        let joined = SyncKey::derive_with("correct horse", salt, params, TEST).unwrap();
        assert_eq!(open_with_key(&file, &joined), Err(SyncError::Decrypt));
    }

    #[test]
    fn truncated() {
        let file = sealed();
        let key = key("correct horse");
        assert_eq!(open_with_key(&file[..HEADER_LEN], &key), Err(SyncError::Truncated));
        assert_eq!(open_with_key(&file[..file.len() - 1], &key), Err(SyncError::Decrypt));
    }

    #[test]
    fn oversized() {
        let mut file = sealed();
        file.resize(MAX_FILE_LEN + 1, 0);
        assert_eq!(open(&file, "correct horse"), Err(SyncError::TooLarge));
        let huge = crate::Snapshot { device: "x".repeat(MAX_FILE_LEN), ..crate::tests::sample() };
        assert_eq!(seal_with_key(&huge, &key("correct horse")), Err(SyncError::TooLarge));
    }

    #[test]
    fn bad_magic_and_version() {
        let key = key("correct horse");
        let mut file = sealed();
        file[0] = b'X';
        assert_eq!(open_with_key(&file, &key), Err(SyncError::BadMagic));
        let mut file = sealed();
        file[8] = 2;
        assert_eq!(open_with_key(&file, &key), Err(SyncError::UnsupportedVersion));
        let mut file = sealed();
        file[10] = 2;
        assert_eq!(open_with_key(&file, &key), Err(SyncError::UnsupportedVersion));
    }

    #[test]
    fn weak_params_rejected() {
        assert_eq!(open(&sealed(), "correct horse"), Err(SyncError::WeakKdf));
        assert_eq!(header_kdf(&sealed()), Err(SyncError::WeakKdf));
        assert!(matches!(SyncKey::derive("x", [0; SALT_LEN], TEST), Err(SyncError::WeakKdf)));
        assert!(matches!(seal(&crate::tests::sample(), "x", TEST), Err(SyncError::WeakKdf)));
    }

    #[test]
    fn costly_params_rejected_before_derivation() {
        let mut file = sealed();
        file[11..15].copy_from_slice(&(KdfParams::MAX.memory_kib + 1).to_le_bytes());
        file[15..19].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(open(&file, "correct horse"), Err(SyncError::CostlyKdf));
        let mut file = sealed();
        file[11..15].copy_from_slice(&KdfParams::MIN.memory_kib.to_le_bytes());
        file[15..19].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(open(&file, "correct horse"), Err(SyncError::CostlyKdf));
    }

    #[test]
    #[ignore = "runs Argon2id with production parameters"]
    fn one_shot_roundtrip_with_default_params() {
        let file = seal(&crate::tests::sample(), "correct horse", KdfParams::DEFAULT).unwrap();
        assert_eq!(open(&file, "correct horse").unwrap(), crate::tests::sample());
    }
}
