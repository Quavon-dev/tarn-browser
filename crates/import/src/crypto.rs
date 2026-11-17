use aes::Aes128;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(crate) type Key = Zeroizing<[u8; 16]>;

#[cfg(any(target_os = "macos", test))]
pub(crate) const MAC_ROUNDS: u32 = 1003;
#[cfg(any(target_os = "linux", test))]
pub(crate) const LINUX_ROUNDS: u32 = 1;
#[cfg(any(target_os = "linux", test))]
pub(crate) const LINUX_V10_PASSWORD: &[u8] = b"peanuts";

pub(crate) struct Keys {
    pub v10: Key,
    pub v11: Option<Key>,
}

/// Chromium's cookie key: PBKDF2-HMAC-SHA1 over the Safe Storage password.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) fn derive(password: &[u8], rounds: u32) -> Key {
    let mut key = Zeroizing::new([0; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", rounds, key.as_mut());
    key
}

/// Decrypts a `v10` or `v11` value (AES-128-CBC, an IV of sixteen spaces, PKCS#7) with its tag's
/// key. With `hashed`, the plaintext must start with the SHA-256 of `host`, which binds it to that
/// host and is stripped. `None` for any other tag, a tag without a key, bad padding, a host
/// mismatch or a value that is not UTF-8.
pub(crate) fn decrypt(keys: &Keys, blob: &[u8], host: &str, hashed: bool) -> Option<Zeroizing<String>> {
    let (key, body) = match blob.split_at_checked(3)? {
        (b"v10", body) => (&keys.v10, body),
        (b"v11", body) => (keys.v11.as_ref()?, body),
        _ => return None,
    };
    let key: &[u8; 16] = key;
    let mut buf = Zeroizing::new(body.to_vec());
    let plain = cbc::Decryptor::<Aes128>::new_from_slices(key, &[b' '; 16]).ok()?.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?;
    let plain = if hashed { plain.strip_prefix(Sha256::digest(host.as_bytes()).as_slice())? } else { plain };
    std::str::from_utf8(plain).ok().map(|s| Zeroizing::new(s.to_owned()))
}

#[cfg(test)]
pub(crate) fn only(key: Key) -> Keys {
    Keys { v10: key, v11: None }
}

#[cfg(test)]
pub(crate) fn encrypt(key: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    encrypt_as(b"v10", key, plain)
}

#[cfg(test)]
pub(crate) fn encrypt_as(tag: &[u8; 3], key: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    use cbc::cipher::BlockEncryptMut;
    let mut buf = vec![0; plain.len() + 16];
    let n = cbc::Encryptor::<Aes128>::new_from_slices(key, &[b' '; 16]).unwrap().encrypt_padded_b2b_mut::<Pkcs7>(plain, &mut buf).unwrap().len();
    buf.truncate(n);
    [tag.as_slice(), &buf].concat()
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
        assert_eq!(*derive(b"peanuts", MAC_ROUNDS), expected);
    }

    #[test]
    fn derives_chromiums_linux_fallback_key() {
        // Python's hashlib.pbkdf2_hmac("sha1", b"peanuts", b"saltysalt", 1, 16).
        let expected = [0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78];
        assert_eq!(*derive(LINUX_V10_PASSWORD, LINUX_ROUNDS), expected);
    }

    #[test]
    fn decrypts_an_openssl_v10_test_vector() {
        // printf abc | openssl enc -aes-128-cbc -K fd621fe5a2b402539dfa147ca9272778 -iv 20202020202020202020202020202020
        let blob = [b"v10".as_slice(), &[0x00, 0xeb, 0x6e, 0x0b, 0x42, 0xc5, 0xe8, 0xdc, 0xb7, 0xbf, 0x57, 0xc6, 0x11, 0x0a, 0xf1, 0x8d]].concat();
        let keys = only(derive(LINUX_V10_PASSWORD, LINUX_ROUNDS));
        assert_eq!(decrypt(&keys, &blob, "a.test", false).as_deref().map(String::as_str), Some("abc"));
    }

    #[test]
    fn v11_values_need_the_keyring_key() {
        let peanuts = derive(LINUX_V10_PASSWORD, LINUX_ROUNDS);
        let secret = derive(b"synthetic keyring secret", LINUX_ROUNDS);
        let v10 = encrypt_as(b"v10", &peanuts, &hashed(".a.test", "old"));
        let v11 = encrypt_as(b"v11", &secret, &hashed(".a.test", "new"));
        let without = Keys { v10: peanuts.clone(), v11: None };
        assert_eq!(decrypt(&without, &v10, ".a.test", true).as_deref().map(String::as_str), Some("old"));
        assert!(decrypt(&without, &v11, ".a.test", true).is_none(), "no keyring key, no v11 value");
        let with = Keys { v10: peanuts.clone(), v11: Some(secret) };
        assert_eq!(decrypt(&with, &v11, ".a.test", true).as_deref().map(String::as_str), Some("new"));
        assert!(decrypt(&with, &v11, ".b.test", true).is_none(), "a v11 value bound to another host is refused");
        let swapped = Keys { v10: derive(b"synthetic keyring secret", LINUX_ROUNDS), v11: Some(peanuts) };
        assert!(decrypt(&swapped, &v11, ".a.test", true).is_none(), "each tag has its own key");
    }

    #[test]
    fn decrypts_a_v10_value_and_checks_the_host_hash() {
        let key = only(derive(b"synthetic test password", MAC_ROUNDS));
        let plain = encrypt(&key.v10, b"abc");
        assert_eq!(decrypt(&key, &plain, ".a.test", false).as_deref().map(String::as_str), Some("abc"));
        let bound = encrypt(&key.v10, &hashed(".a.test", "abc"));
        assert_eq!(decrypt(&key, &bound, ".a.test", true).as_deref().map(String::as_str), Some("abc"));
        assert!(decrypt(&key, &bound, ".b.test", true).is_none(), "a value bound to another host is refused");
    }

    #[test]
    fn rejects_other_tags_bad_padding_and_a_wrong_key() {
        let key = only(derive(b"synthetic test password", MAC_ROUNDS));
        let blob = encrypt(&key.v10, b"abc");
        assert!(decrypt(&key, &[b"v20", &blob[3..]].concat(), "a.test", false).is_none());
        assert!(decrypt(&key, &blob[..blob.len() - 1], "a.test", false).is_none(), "a truncated block");
        assert!(decrypt(&key, b"v10", "a.test", false).is_none());
        let mut flipped = blob.clone();
        *flipped.last_mut().unwrap() ^= 0xff;
        assert!(decrypt(&key, &flipped, "a.test", false).is_none(), "malformed padding");
        assert!(decrypt(&key, b"v1", "a.test", false).is_none());
        assert!(decrypt(&only(derive(b"other", MAC_ROUNDS)), &blob, "a.test", false).is_none());
    }
}
