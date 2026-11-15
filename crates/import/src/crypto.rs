use aes::Aes128;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(crate) type Key = Zeroizing<[u8; 16]>;

/// Chromium's macOS cookie key: PBKDF2-HMAC-SHA1 over the Safe Storage password.
pub(crate) fn derive(password: &[u8]) -> Key {
    let mut key = Zeroizing::new([0; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, key.as_mut());
    key
}

/// Decrypts a `v10` value (AES-128-CBC, an IV of sixteen spaces, PKCS#7). With `hashed`, the
/// plaintext must start with the SHA-256 of `host`, which binds it to that host and is stripped.
/// `None` for any other tag, bad padding, a host mismatch or a value that is not UTF-8.
pub(crate) fn decrypt(key: &[u8; 16], blob: &[u8], host: &str, hashed: bool) -> Option<Zeroizing<String>> {
    let body = blob.strip_prefix(b"v10")?;
    let mut buf = Zeroizing::new(body.to_vec());
    let plain = cbc::Decryptor::<Aes128>::new_from_slices(key, &[b' '; 16]).ok()?.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?;
    let plain = if hashed { plain.strip_prefix(Sha256::digest(host.as_bytes()).as_slice())? } else { plain };
    std::str::from_utf8(plain).ok().map(|s| Zeroizing::new(s.to_owned()))
}

#[cfg(test)]
pub(crate) fn encrypt(key: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    use cbc::cipher::BlockEncryptMut;
    let mut buf = vec![0; plain.len() + 16];
    let n = cbc::Encryptor::<Aes128>::new_from_slices(key, &[b' '; 16]).unwrap().encrypt_padded_b2b_mut::<Pkcs7>(plain, &mut buf).unwrap().len();
    buf.truncate(n);
    [b"v10".as_slice(), &buf].concat()
}

#[cfg(test)]
pub(crate) fn hashed(host: &str, value: &str) -> Vec<u8> {
    [Sha256::digest(host.as_bytes()).as_slice(), value.as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_known_key_for_a_password() {
        // Python's hashlib.pbkdf2_hmac("sha1", b"peanuts", b"saltysalt", 1003, 16).
        let expected = [0xd9, 0xa0, 0x9d, 0x49, 0x9b, 0x4e, 0x1b, 0x74, 0x61, 0xf2, 0x8e, 0x67, 0x97, 0x2c, 0x6d, 0xbd];
        assert_eq!(*derive(b"peanuts"), expected);
    }

    #[test]
    fn decrypts_a_v10_value_and_checks_the_host_hash() {
        let key = derive(b"synthetic test password");
        let plain = encrypt(&key, b"abc");
        assert_eq!(decrypt(&key, &plain, ".a.test", false).as_deref().map(String::as_str), Some("abc"));
        let bound = encrypt(&key, &hashed(".a.test", "abc"));
        assert_eq!(decrypt(&key, &bound, ".a.test", true).as_deref().map(String::as_str), Some("abc"));
        assert!(decrypt(&key, &bound, ".b.test", true).is_none(), "a value bound to another host is refused");
    }

    #[test]
    fn rejects_other_tags_bad_padding_and_a_wrong_key() {
        let key = derive(b"synthetic test password");
        let blob = encrypt(&key, b"abc");
        assert!(decrypt(&key, &[b"v20", &blob[3..]].concat(), "a.test", false).is_none());
        assert!(decrypt(&key, &blob[..blob.len() - 1], "a.test", false).is_none(), "a truncated block");
        assert!(decrypt(&key, b"v10", "a.test", false).is_none());
        let mut flipped = blob.clone();
        *flipped.last_mut().unwrap() ^= 0xff;
        assert!(decrypt(&key, &flipped, "a.test", false).is_none(), "malformed padding");
        assert!(decrypt(&derive(b"other"), &blob, "a.test", false).is_none());
    }
}
