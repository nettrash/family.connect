//! A certificate chain shaped like Apple's App Store chain — a P-384 root,
//! a P-384 intermediate carrying the WWDR marker, a P-256 leaf carrying the
//! receipt marker — with keys this process holds, so tests can sign whatever
//! they need to see believed or refused (docs/protocol.md, "Paid families").
//!
//! ONE file for two crates: `src/appstore.rs`'s unit tests include it as a
//! module, and the integration suites include it from `tests/common` by
//! path, so it names no `crate::` item — only the dependencies both share.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rcgen::{
    BasicConstraints as RcBasicConstraints, CertificateParams, CustomExtension, IsCa, Issuer,
    KeyPair, PKCS_ECDSA_P256_SHA256, PKCS_ECDSA_P384_SHA384,
};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

/// What to leave out of, or get wrong in, the chain.
#[derive(Default, Clone, Copy)]
pub struct Flaws {
    pub no_leaf_marker: bool,
    pub no_intermediate_marker: bool,
    pub intermediate_not_ca: bool,
    /// Validity ends in 2001.
    pub expired_leaf: bool,
}

pub struct Chain {
    pub root: Vec<u8>,
    intermediate: Vec<u8>,
    leaf: Vec<u8>,
    leaf_key: EcdsaKeyPair,
}

fn marker(oid: &[u64]) -> CustomExtension {
    // Apple's markers carry an ASN.1 NULL; what matters is the OID.
    CustomExtension::from_oid_content(oid, vec![0x05, 0x00])
}

impl Chain {
    pub fn new() -> Self {
        Self::with(Flaws::default())
    }

    pub fn with(flaws: Flaws) -> Self {
        let root_key = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).expect("root key");
        let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
        root_params.is_ca = IsCa::Ca(RcBasicConstraints::Unconstrained);
        let root = root_params.self_signed(&root_key).expect("root");

        let intermediate_key =
            KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).expect("intermediate key");
        let mut intermediate_params = CertificateParams::new(Vec::<String>::new()).expect("params");
        if !flaws.intermediate_not_ca {
            intermediate_params.is_ca = IsCa::Ca(RcBasicConstraints::Constrained(0));
        }
        if !flaws.no_intermediate_marker {
            intermediate_params
                .custom_extensions
                .push(marker(&[1, 2, 840, 113635, 100, 6, 2, 1]));
        }
        let intermediate = intermediate_params
            .signed_by(
                &intermediate_key,
                &Issuer::from_params(&root_params, &root_key),
            )
            .expect("intermediate");

        let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("leaf key");
        let mut leaf_params = CertificateParams::new(Vec::<String>::new()).expect("params");
        if !flaws.no_leaf_marker {
            leaf_params
                .custom_extensions
                .push(marker(&[1, 2, 840, 113635, 100, 6, 11, 1]));
        }
        if flaws.expired_leaf {
            leaf_params.not_before = rcgen::date_time_ymd(2000, 1, 1);
            leaf_params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let leaf = leaf_params
            .signed_by(
                &leaf_key,
                &Issuer::from_params(&intermediate_params, &intermediate_key),
            )
            .expect("leaf");

        let leaf_key = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &leaf_key.serialize_der(),
            &SystemRandom::new(),
        )
        .expect("leaf signing key");
        Chain {
            root: root.der().to_vec(),
            intermediate: intermediate.der().to_vec(),
            leaf: leaf.der().to_vec(),
            leaf_key,
        }
    }

    /// A JWS over `payload`, signed by this chain's leaf.
    pub fn sign(&self, payload: &Value) -> String {
        self.sign_with_root(payload, &self.root)
    }

    /// The same, with `root` in the header's third slot — the way a
    /// forger would present Apple's real root over their own chain.
    pub fn sign_with_root(&self, payload: &Value, root: &[u8]) -> String {
        let header = serde_json::json!({
            "alg": "ES256",
            "x5c": [STANDARD.encode(&self.leaf), STANDARD.encode(&self.intermediate),
                    STANDARD.encode(root)],
        });
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(payload.to_string())
        );
        let sig = self
            .leaf_key
            .sign(&SystemRandom::new(), input.as_bytes())
            .expect("sign");
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.as_ref()))
    }
}

/// A transaction payload as StoreKit 2 writes one, signed now.
pub fn transaction_payload(
    original: &str,
    product: &str,
    expires: OffsetDateTime,
    token: Option<Uuid>,
) -> Value {
    let mut payload = serde_json::json!({
        "transactionId": format!("{original}-{}", expires.unix_timestamp()),
        "originalTransactionId": original,
        "bundleId": "me.nettrash.FamilyConnect",
        "productId": product,
        "purchaseDate": ms(OffsetDateTime::now_utc()),
        "expiresDate": ms(expires),
        "type": "Auto-Renewable Subscription",
        "inAppOwnershipType": "PURCHASED",
        "signedDate": ms(OffsetDateTime::now_utc()),
        "environment": "Production",
    });
    if let Some(token) = token {
        payload["appAccountToken"] = Value::String(token.to_string());
    }
    payload
}

pub fn ms(at: OffsetDateTime) -> i64 {
    (at.unix_timestamp_nanos() / 1_000_000) as i64
}
