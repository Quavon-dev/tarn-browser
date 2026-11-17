//! Minimum key sizes for certificates Tarn's own HTTPS clients accept (NIST, valid through 2030;
//! docs/cryptography.md). The engines' page TLS is not decided here.

pub const MIN_RSA_BITS: u32 = 2048;
pub const MIN_EC_BITS: u32 = 224;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Rsa,
    Ec,
    Ed25519,
    Other,
}

impl KeyKind {
    pub fn from_code(code: i32) -> KeyKind {
        match code {
            1 => KeyKind::Rsa,
            2 => KeyKind::Ec,
            3 => KeyKind::Ed25519,
            _ => KeyKind::Other,
        }
    }
}

pub fn strong_key(kind: KeyKind, bits: u32) -> bool {
    match kind {
        KeyKind::Rsa => bits >= MIN_RSA_BITS,
        KeyKind::Ec => bits >= MIN_EC_BITS,
        KeyKind::Ed25519 => true,
        KeyKind::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsa_needs_2048_bits() {
        assert!(!strong_key(KeyKind::Rsa, 1024));
        assert!(!strong_key(KeyKind::Rsa, 2047));
        assert!(strong_key(KeyKind::Rsa, 2048));
    }

    #[test]
    fn ec_needs_224_bits() {
        assert!(!strong_key(KeyKind::Ec, 192));
        assert!(strong_key(KeyKind::Ec, 224));
        assert!(strong_key(KeyKind::Ec, 256));
    }

    #[test]
    fn ed25519_passes_and_other_kinds_fail() {
        assert!(strong_key(KeyKind::Ed25519, 0));
        assert!(!strong_key(KeyKind::Other, 4096));
        assert_eq!(KeyKind::from_code(1), KeyKind::Rsa);
        assert_eq!(KeyKind::from_code(2), KeyKind::Ec);
        assert_eq!(KeyKind::from_code(3), KeyKind::Ed25519);
        assert_eq!(KeyKind::from_code(0), KeyKind::Other);
        assert_eq!(KeyKind::from_code(7), KeyKind::Other);
    }
}
