//! Test-only key material, shared by the config and push unit tests.
//!
//! Compiled only under `cfg(test)` — nothing here ships. P-256 generation is
//! cheap and done per call; RSA-2048 (which ring insists on for RS256) is
//! expensive even with the `[profile.dev.package.rsa]` opt-level override,
//! so one key is generated per test binary and reused.

use std::sync::OnceLock;

use p256::pkcs8::EncodePublicKey;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};

/// A fresh P-256 key pair as `(private PKCS#8 PEM, public SPKI PEM)` — the
/// private half is byte-for-byte the shape of an Apple `.p8` file.
pub fn ec_p256_key_pair_pems() -> (String, String) {
    let key = p256::SecretKey::random(&mut rand::rngs::OsRng);
    let private_pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("serializing the test P-256 private key")
        .to_string();
    let public_pem = key
        .public_key()
        .to_public_key_pem(LineEnding::LF)
        .expect("serializing the test P-256 public key");
    (private_pem, public_pem)
}

/// The per-test-binary RSA-2048 key pair as
/// `(private PKCS#8 PEM, public SPKI PEM)`.
pub fn rsa_2048_key_pair_pems() -> (&'static str, &'static str) {
    static PAIR: OnceLock<(String, String)> = OnceLock::new();
    let (private_pem, public_pem) = PAIR.get_or_init(|| {
        let key = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048)
            .expect("generating the test RSA key");
        let private_pem = key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("serializing the test RSA private key")
            .to_string();
        let public_pem = rsa::RsaPublicKey::from(&key)
            .to_public_key_pem(LineEnding::LF)
            .expect("serializing the test RSA public key");
        (private_pem, public_pem)
    });
    (private_pem, public_pem)
}

/// A fake Google service-account JSON document carrying the test RSA key —
/// only the fields the FCM transport reads, plus the `type` marker real
/// files have.
pub fn service_account_json(project_id: &str, client_email: &str, token_uri: &str) -> String {
    let (private_pem, _) = rsa_2048_key_pair_pems();
    serde_json::json!({
        "type": "service_account",
        "project_id": project_id,
        "private_key_id": "test-key-id",
        "private_key": private_pem,
        "client_email": client_email,
        "token_uri": token_uri,
    })
    .to_string()
}
