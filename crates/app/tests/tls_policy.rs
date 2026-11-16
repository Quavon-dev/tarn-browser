//! The TLS provider every shell request uses (`rustls::crypto::ring::default_provider()` in
//! `filters.rs`, `ext_sources.rs`, the AI provider and the Windows updater) refuses a server key
//! under 2048 bits RSA for the handshake signature and offers only ephemeral elliptic-curve key
//! exchange; the chain's intermediate and root keys are left to the platform verifier
//! (docs/cryptography.md).
//! The vectors are public keys (RSAPublicKey DER) and SHA-256 PKCS#1 signatures of `MESSAGE`, made
//! with `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:N` and `openssl dgst -sha256 -sign`;
//! the private keys were discarded.

use rustls::NamedGroup;
use rustls::crypto::ring::default_provider;

const MESSAGE: &[u8] = b"tarn key-length test";

const RSA_1024_KEY: &[&str] = &[
    "30818902818100b5a4de2d6bfed008b03fbbd0e355781da27ede923bfdb699462e0ee71580042ed5f51fe19a04e20ee1",
    "10eea52559c73c0ed517de203a7e077f18ac5d09d9dec9066b058dfd9935a100fd33af71e093aa49164103d61bd8dcd4",
    "5775fd52e9e6ff151074a630708b98b0077b719672e3cb752380c2336093640140b1f5482535610203010001",
];
const RSA_1024_SIG: &[&str] = &[
    "5f1e1d339603232eeba474e4d032ef93d3e2d81cc643ae25cffcbe188e543da7753d57a0d9da64de49e3935a7368f46a",
    "92c59c014fdaf5b142942f2c31928a6b77b5f4b65e9461a641d129b1d24240b58fe4c54f37bd85897d192ac8762483d3",
    "40b06cd9abad1cdbb39a53eb10b588907694d337192533c37920df8857ae715b",
];
const RSA_2048_KEY: &[&str] = &[
    "3082010a0282010100a1eedf848223017be207b3d812408faa89921eed89af5a53238fe06ae471d43147177ec1410eea",
    "36732f850ddad43768fbe68d0b28848f22f6b710238a3db7b77daba6252c1dfce796a7c5eda04d8a3d1d094c35156ebf",
    "9d5caca12c5361f12f24697d72e33a5b30fa18cd03792bb2ac0c14dce82444f67a0cc5d81231ffdab62be4605e239c7c",
    "5cc58914e14c6009bb3132bb5bc1fb839c5bff2af14c816f0e563b7d92becbecc648bec42fca7efb7cdbf9e8b5d3c749",
    "7471b7c26a84009679bf016508fbe978d9f4d6200d70899027105c83cd8657c3d062fd2ef22b6bc4f92885e28c456539",
    "adce350aaf25d38cfc5d954b9c72611dad6ba1f46ddd9b6cb70203010001",
];
const RSA_2048_SIG: &[&str] = &[
    "515fdaa90e4080afcb6bd129ef9370b198c26bc3a5c6a3dda19c7b503985b76d341a38ed79934aa0478bde2c6c98ba60",
    "1648dc44c46e8cc7cbfbfd00c0180f64851b9c207c7c2f2a2522303d89145b0bb85bacf513828ff82550c81359e1d4da",
    "a33ed3912d7e96bd36c464e011477af4e7c1394785958842eb66bfdcf2468544ca9dabf48f264d8635b9628dbfb09a87",
    "2859245b4a4348dd91cb28ed628672de44892a6b54d7417583fa36cd6bd7035efc2490089eb8403d0242f380d58f953f",
    "3c0f60fbe76a852f9c540e01748bdb0af15e64c0368ddd806627e36ac7dc546ea5e3990f8740cf4f7066d05730c49257",
    "16c34c1171463d68a5552549998b700d",
];

fn bytes(lines: &[&str]) -> Vec<u8> {
    let hex = lines.concat();
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
}

fn verifies(key: &[&str], sig: &[&str]) -> bool {
    let (key, sig) = (bytes(key), bytes(sig));
    default_provider().signature_verification_algorithms.all.iter().any(|alg| alg.verify_signature(&key, MESSAGE, &sig).is_ok())
}

#[test]
fn rsa_keys_under_2048_bits_are_refused() {
    assert!(verifies(RSA_2048_KEY, RSA_2048_SIG), "the 2048-bit control must verify");
    assert!(!verifies(RSA_1024_KEY, RSA_1024_SIG), "a 1024-bit RSA signature must not verify");
}

#[test]
fn key_exchange_is_ephemeral_elliptic_curve_only() {
    let groups: Vec<NamedGroup> = default_provider().kx_groups.iter().map(|g| g.name()).collect();
    assert!(!groups.is_empty());
    assert!(groups.iter().all(|g| [NamedGroup::X25519, NamedGroup::secp256r1, NamedGroup::secp384r1].contains(g)), "{groups:?}");
    for suite in default_provider().cipher_suites {
        let name = format!("{:?}", suite.suite());
        assert!(name.starts_with("TLS13_") || name.starts_with("TLS_ECDHE_"), "{name}");
    }
}
