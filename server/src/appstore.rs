//! What the App Store signed (docs/protocol.md, "Paid families").
//!
//! Every value Apple hands an app or this server about a purchase — a
//! transaction, a subscription's renewal info, a server notification — is a
//! JWS whose `x5c` header carries the certificates that signed it. This
//! module believes one only after checking all of it, OFFLINE:
//!
//! - the chain is exactly leaf, intermediate, root, and the root is
//!   byte-equal to Apple Root CA – G3, which the binary carries;
//! - each certificate is signed by the next, and the JWS by the leaf (ES256);
//! - the leaf carries Apple's marker `1.2.840.113635.100.6.11.1` and the
//!   intermediate `1.2.840.113635.100.6.2.1`;
//! - every certificate is valid at the payload's own `signedDate`.
//!
//! The markers are the check that matters most and the easiest to leave out.
//! Apple Root CA – G3 also signs certificates whose private keys live with
//! developers — an Apple Pay merchant identity is one — so a chain check
//! without them would accept a "subscription" anybody with a developer
//! account signed themselves.
//!
//! The policy above the signature — which app, which products, which
//! environments, whose account — is [`Policy`]'s, and the caller's.
//!
//! No call to Apple is made. Revocation (OCSP) is not checked, which is what
//! Apple's own library does when it is told not to go online.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;
use x509_cert::Certificate;
use x509_cert::der::asn1::ObjectIdentifier;
use x509_cert::der::{Decode, Encode};
use x509_cert::ext::pkix::BasicConstraints;

/// Apple Root CA – G3, DER, as Apple publishes it and as macOS ships it in
/// its system roots. SHA-256 `63343abf…3e9179` — pinned by a unit test.
pub const APPLE_ROOT_CA_G3: &[u8] = include_bytes!("appstore/AppleRootCA-G3.cer");

/// On the leaf: "this certificate signs App Store receipts".
const LEAF_MARKER: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113635.100.6.11.1");
/// On the intermediate: Apple's Worldwide Developer Relations CA.
const INTERMEDIATE_MARKER: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113635.100.6.2.1");

const ID_EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const CURVE_P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const CURVE_P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const BASIC_CONSTRAINTS: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.19");

/// Why a signed value was not believed. Each names a CHECK, never anything
/// from inside the value — it goes to the journal as `reason()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not three base64url parts, not JSON, or a field of the wrong shape.
    Malformed,
    /// The header's `alg` is not ES256.
    Algorithm,
    /// Not exactly three certificates, one that will not parse, or a root
    /// that is not the pinned one.
    Chain,
    /// A certificate not signed by the next, or the JWS not by the leaf.
    Signature,
    /// The intermediate is not a CA, or a marker extension is missing.
    Marker,
    /// A certificate not valid at the payload's `signedDate`.
    Expired,
    /// Signed by Apple, but for another app.
    Bundle,
    /// Signed by Apple, but not one of this server's products.
    Product,
    /// Signed by Apple, but in an environment this server does not take.
    Environment,
    /// Not an auto-renewable subscription.
    Kind,
    /// Bought for another account (`appAccountToken`).
    Token,
    /// A renewal info that names another subscription than its transaction.
    Mismatch,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::Malformed => "malformed",
            Refusal::Algorithm => "algorithm",
            Refusal::Chain => "chain",
            Refusal::Signature => "signature",
            Refusal::Marker => "marker extension",
            Refusal::Expired => "certificate validity",
            Refusal::Bundle => "bundle",
            Refusal::Product => "product",
            Refusal::Environment => "environment",
            Refusal::Kind => "type",
            Refusal::Token => "account token",
            Refusal::Mismatch => "renewal mismatch",
        }
    }
}

/// A JWS whose signature and chain held, with its payload and the moment
/// Apple says it signed it.
#[derive(Debug)]
pub struct Verified {
    pub payload: Value,
    pub signed: OffsetDateTime,
}

/// Verify one JWS against `root` (the DER of the trusted root —
/// [`APPLE_ROOT_CA_G3`] everywhere but tests).
pub fn verify(root: &[u8], jws: &str) -> Result<Verified, Refusal> {
    let mut parts = jws.trim().split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Refusal::Malformed);
    };
    let header: Value = decode_json(header_b64)?;
    let payload: Value = decode_json(payload_b64)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| Refusal::Malformed)?;

    if header.get("alg").and_then(Value::as_str) != Some("ES256") {
        return Err(Refusal::Algorithm);
    }
    let x5c = header
        .get("x5c")
        .and_then(Value::as_array)
        .ok_or(Refusal::Chain)?;
    if x5c.len() != 3 {
        return Err(Refusal::Chain);
    }
    let mut ders = Vec::with_capacity(3);
    for entry in x5c {
        let text = entry.as_str().ok_or(Refusal::Chain)?;
        ders.push(STANDARD.decode(text).map_err(|_| Refusal::Chain)?);
    }
    if ders[2] != root {
        return Err(Refusal::Chain);
    }
    let leaf = parse(&ders[0])?;
    let intermediate = parse(&ders[1])?;
    let root_cert = parse(&ders[2])?;

    // Apple signs the payload's `signedDate`; every certificate must have
    // been valid then. A missing or odd one is a malformed value, not a
    // reason to fall back to the wall clock.
    let signed = millis(&payload, "signedDate")?.ok_or(Refusal::Malformed)?;

    signed_by(&intermediate, &root_cert)?;
    signed_by(&leaf, &intermediate)?;
    if !is_ca(&intermediate)? || is_ca(&leaf)? {
        return Err(Refusal::Marker);
    }
    if !has_extension(&intermediate, INTERMEDIATE_MARKER) || !has_extension(&leaf, LEAF_MARKER) {
        return Err(Refusal::Marker);
    }
    for cert in [&leaf, &intermediate, &root_cert] {
        valid_at(cert, signed)?;
    }

    // ES256: a P-256 leaf, and the signature as the raw r || s pair.
    let (curve, point) = ec_key(&leaf)?;
    if curve != CURVE_P256 {
        return Err(Refusal::Signature);
    }
    let signing_input = format!("{header_b64}.{payload_b64}");
    UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point)
        .verify(signing_input.as_bytes(), &signature)
        .map_err(|_| Refusal::Signature)?;

    Ok(Verified { payload, signed })
}

fn decode_json(part: &str) -> Result<Value, Refusal> {
    let bytes = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| Refusal::Malformed)?;
    serde_json::from_slice(&bytes).map_err(|_| Refusal::Malformed)
}

/// Parse a certificate, refusing one whose re-encoding is not the same bytes:
/// the signature covers the TBS bytes exactly as Apple wrote them, and the
/// check below re-encodes them, which is the same thing only for canonical
/// DER.
fn parse(der: &[u8]) -> Result<Certificate, Refusal> {
    let cert = Certificate::from_der(der).map_err(|_| Refusal::Chain)?;
    match cert.to_der() {
        Ok(again) if again == der => Ok(cert),
        _ => Err(Refusal::Chain),
    }
}

/// The issuer's EC key: its curve and the uncompressed point.
fn ec_key(cert: &Certificate) -> Result<(ObjectIdentifier, &[u8]), Refusal> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid != ID_EC_PUBLIC_KEY {
        return Err(Refusal::Signature);
    }
    let curve: ObjectIdentifier = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|any| any.decode_as().ok())
        .ok_or(Refusal::Signature)?;
    let point = spki
        .subject_public_key
        .as_bytes()
        .ok_or(Refusal::Signature)?;
    Ok((curve, point))
}

/// Is `child` signed by `issuer`'s key? ECDSA over P-256 or P-384 with
/// SHA-256 or SHA-384 — every pairing Apple's App Store chain uses.
fn signed_by(child: &Certificate, issuer: &Certificate) -> Result<(), Refusal> {
    let (curve, point) = ec_key(issuer)?;
    let algorithm: &dyn VerificationAlgorithm = match (curve, child.signature_algorithm.oid) {
        (c, h) if c == CURVE_P256 && h == ECDSA_SHA256 => &signature::ECDSA_P256_SHA256_ASN1,
        (c, h) if c == CURVE_P256 && h == ECDSA_SHA384 => &signature::ECDSA_P256_SHA384_ASN1,
        (c, h) if c == CURVE_P384 && h == ECDSA_SHA256 => &signature::ECDSA_P384_SHA256_ASN1,
        (c, h) if c == CURVE_P384 && h == ECDSA_SHA384 => &signature::ECDSA_P384_SHA384_ASN1,
        _ => return Err(Refusal::Signature),
    };
    let tbs = child.tbs_certificate.to_der().map_err(|_| Refusal::Chain)?;
    let sig = child.signature.as_bytes().ok_or(Refusal::Signature)?;
    UnparsedPublicKey::new(algorithm, point)
        .verify(&tbs, sig)
        .map_err(|_| Refusal::Signature)
}

fn has_extension(cert: &Certificate, oid: ObjectIdentifier) -> bool {
    cert.tbs_certificate
        .extensions
        .as_ref()
        .is_some_and(|exts| exts.iter().any(|ext| ext.extn_id == oid))
}

/// Whether the certificate's basic constraints say it is a CA. Absent means
/// not; one that will not parse is refused rather than guessed at.
fn is_ca(cert: &Certificate) -> Result<bool, Refusal> {
    let Some(ext) = cert
        .tbs_certificate
        .extensions
        .as_ref()
        .and_then(|exts| exts.iter().find(|ext| ext.extn_id == BASIC_CONSTRAINTS))
    else {
        return Ok(false);
    };
    BasicConstraints::from_der(ext.extn_value.as_bytes())
        .map(|bc| bc.ca)
        .map_err(|_| Refusal::Marker)
}

fn valid_at(cert: &Certificate, at: OffsetDateTime) -> Result<(), Refusal> {
    let validity = &cert.tbs_certificate.validity;
    let seconds = at.unix_timestamp();
    let not_before = validity.not_before.to_unix_duration().as_secs() as i64;
    let not_after = validity.not_after.to_unix_duration().as_secs() as i64;
    if seconds < not_before || seconds > not_after {
        return Err(Refusal::Expired);
    }
    Ok(())
}

/// A field Apple writes as milliseconds since the epoch. `Ok(None)` when it
/// is absent or null; a value of any other shape is malformed.
fn millis(payload: &Value, key: &str) -> Result<Option<OffsetDateTime>, Refusal> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let ms = value.as_i64().ok_or(Refusal::Malformed)?;
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
                .map(Some)
                .map_err(|_| Refusal::Malformed)
        }
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> Result<Option<&'a str>, Refusal> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_str().map(Some).ok_or(Refusal::Malformed),
    }
}

fn required<'a>(payload: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    text(payload, key)?.ok_or(Refusal::Malformed)
}

/// The operator's half: which app, which products, which environments.
pub struct Policy<'a> {
    pub root: &'a [u8],
    pub bundle_id: &'a str,
    pub product_ids: &'a [String],
    pub environments: &'a [String],
}

/// A subscription transaction, believed.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    pub original_transaction_id: String,
    pub transaction_id: String,
    pub product_id: String,
    pub environment: String,
    pub expires: OffsetDateTime,
    pub revoked: Option<OffsetDateTime>,
    pub app_account_token: Option<Uuid>,
    pub signed: OffsetDateTime,
}

/// A subscription's renewal info, believed.
#[derive(Debug, Clone, PartialEq)]
pub struct Renewal {
    pub original_transaction_id: String,
    pub environment: String,
    /// `autoRenewStatus`: whether it will renew. `None` when Apple did not say.
    pub renews: Option<bool>,
    /// Apple's billing grace period, while one is running.
    pub grace_until: Option<OffsetDateTime>,
    pub signed: OffsetDateTime,
}

/// The outside of a server notification, believed. What it carries inside
/// is verified on its own, by [`Policy::transaction`] and
/// [`Policy::renewal`].
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub notification_type: String,
    pub subtype: Option<String>,
    pub bundle_id: Option<String>,
    pub environment: Option<String>,
    pub signed_transaction: Option<String>,
    pub signed_renewal: Option<String>,
}

impl Policy<'_> {
    fn environment(&self, environment: &str) -> Result<(), Refusal> {
        if self.environments.iter().any(|e| e == environment) {
            Ok(())
        } else {
            Err(Refusal::Environment)
        }
    }

    /// A signed transaction, held to this server's app, products and
    /// environments.
    pub fn transaction(&self, jws: &str) -> Result<Transaction, Refusal> {
        let Verified { payload, signed } = verify(self.root, jws)?;
        if required(&payload, "bundleId")? != self.bundle_id {
            return Err(Refusal::Bundle);
        }
        let environment = required(&payload, "environment")?;
        self.environment(environment)?;
        let product_id = required(&payload, "productId")?;
        if !self.product_ids.iter().any(|p| p == product_id) {
            return Err(Refusal::Product);
        }
        if required(&payload, "type")? != "Auto-Renewable Subscription" {
            return Err(Refusal::Kind);
        }
        let expires = millis(&payload, "expiresDate")?.ok_or(Refusal::Malformed)?;
        let app_account_token = match text(&payload, "appAccountToken")? {
            None | Some("") => None,
            Some(token) => Some(Uuid::parse_str(token).map_err(|_| Refusal::Malformed)?),
        };
        Ok(Transaction {
            original_transaction_id: required(&payload, "originalTransactionId")?.to_string(),
            transaction_id: required(&payload, "transactionId")?.to_string(),
            product_id: product_id.to_string(),
            environment: environment.to_string(),
            expires,
            revoked: millis(&payload, "revocationDate")?,
            app_account_token,
            signed,
        })
    }

    /// A signed renewal info. It carries no bundle of its own, so it is held
    /// to the environments here and to its transaction by the caller.
    pub fn renewal(&self, jws: &str) -> Result<Renewal, Refusal> {
        let Verified { payload, signed } = verify(self.root, jws)?;
        let environment = required(&payload, "environment")?;
        self.environment(environment)?;
        let renews = match payload.get("autoRenewStatus") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_i64().ok_or(Refusal::Malformed)? == 1),
        };
        Ok(Renewal {
            original_transaction_id: required(&payload, "originalTransactionId")?.to_string(),
            environment: environment.to_string(),
            renews,
            grace_until: millis(&payload, "gracePeriodExpiresDate")?,
            signed,
        })
    }

    /// A server notification's outer envelope. The bundle and environment
    /// are returned rather than enforced: a notification for another app or
    /// an environment this server does not take is ANSWERED (200) and
    /// dropped, never refused, so Apple stops sending it.
    pub fn notification(&self, signed_payload: &str) -> Result<Notification, Refusal> {
        let Verified { payload, .. } = verify(self.root, signed_payload)?;
        let data = payload.get("data").unwrap_or(&Value::Null);
        Ok(Notification {
            notification_type: required(&payload, "notificationType")?.to_string(),
            subtype: text(&payload, "subtype")?.map(str::to_string),
            bundle_id: text(data, "bundleId")?.map(str::to_string),
            environment: text(data, "environment")?.map(str::to_string),
            signed_transaction: text(data, "signedTransactionInfo")?.map(str::to_string),
            signed_renewal: text(data, "signedRenewalInfo")?.map(str::to_string),
        })
    }
}

#[cfg(test)]
#[path = "appstore/testing.rs"]
pub(crate) mod testing;

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use sha2::{Digest, Sha256};
    use time::Duration;

    const MONTHLY: &str = "me.nettrash.FamilyConnect.family.monthly";

    fn products() -> Vec<String> {
        vec![MONTHLY.to_string()]
    }

    fn environments() -> Vec<String> {
        vec!["Production".to_string(), "Sandbox".to_string()]
    }

    fn policy<'a>(
        root: &'a [u8],
        products: &'a [String],
        environments: &'a [String],
    ) -> Policy<'a> {
        Policy {
            root,
            bundle_id: "me.nettrash.FamilyConnect",
            product_ids: products,
            environments,
        }
    }

    fn in_a_month() -> OffsetDateTime {
        OffsetDateTime::now_utc() + Duration::days(30)
    }

    #[test]
    fn the_pinned_root_is_apple_root_ca_g3() {
        let digest = hex::encode(Sha256::digest(APPLE_ROOT_CA_G3));
        assert_eq!(
            digest,
            "63343abfb89a6a03ebb57e9b3f5fa7be7c4f5c756f3017b3a8c488c3653e9179"
        );
    }

    /// Apple's REAL intermediate (WWDR G6, public, from Apple's certificate
    /// authority page) against the real pinned root: the link this module
    /// checks first, with the curves and hashes Apple actually uses.
    #[test]
    fn apples_real_intermediate_is_signed_by_the_pinned_root_and_carries_the_marker() {
        let root = parse(APPLE_ROOT_CA_G3).expect("root parses");
        let g6 = parse(include_bytes!("appstore/AppleWWDRCAG6.cer")).expect("G6 parses");
        signed_by(&g6, &root).expect("G6 is signed by Apple Root CA – G3");
        signed_by(&root, &root).expect("the root is self-signed");
        assert!(is_ca(&g6).expect("constraints parse"));
        assert!(has_extension(&g6, INTERMEDIATE_MARKER));
        assert!(!has_extension(&g6, LEAF_MARKER));
        valid_at(&g6, OffsetDateTime::now_utc()).expect("G6 is valid today");
    }

    #[test]
    fn a_well_formed_transaction_is_believed() {
        let chain = Chain::new();
        let token = Uuid::new_v4();
        let jws = chain.sign(&transaction_payload(
            "2000001",
            MONTHLY,
            in_a_month(),
            Some(token),
        ));
        let (p, e) = (products(), environments());
        let txn = policy(&chain.root, &p, &e)
            .transaction(&jws)
            .expect("believed");
        assert_eq!(txn.original_transaction_id, "2000001");
        assert_eq!(txn.product_id, MONTHLY);
        assert_eq!(txn.environment, "Production");
        assert_eq!(txn.app_account_token, Some(token));
        assert_eq!(txn.revoked, None);
        assert!(txn.expires > OffsetDateTime::now_utc());
    }

    #[test]
    fn a_chain_without_the_leaf_marker_is_refused() {
        let chain = Chain::with(Flaws {
            no_leaf_marker: true,
            ..Flaws::default()
        });
        let jws = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        assert_eq!(verify(&chain.root, &jws).unwrap_err(), Refusal::Marker);
    }

    #[test]
    fn a_chain_without_the_intermediate_marker_is_refused() {
        let chain = Chain::with(Flaws {
            no_intermediate_marker: true,
            ..Flaws::default()
        });
        let jws = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        assert_eq!(verify(&chain.root, &jws).unwrap_err(), Refusal::Marker);
    }

    #[test]
    fn an_intermediate_that_is_not_a_ca_is_refused() {
        let chain = Chain::with(Flaws {
            intermediate_not_ca: true,
            ..Flaws::default()
        });
        let jws = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        assert_eq!(verify(&chain.root, &jws).unwrap_err(), Refusal::Marker);
    }

    #[test]
    fn a_leaf_expired_at_signing_is_refused() {
        let chain = Chain::with(Flaws {
            expired_leaf: true,
            ..Flaws::default()
        });
        let jws = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        assert_eq!(verify(&chain.root, &jws).unwrap_err(), Refusal::Expired);
    }

    /// The forger's move: their own chain, with Apple's real root pasted
    /// into the third slot. The root matches the pin; the signature over it
    /// cannot.
    #[test]
    fn apples_root_pasted_over_another_chain_is_refused() {
        let chain = Chain::new();
        let jws = chain.sign_with_root(
            &transaction_payload("1", MONTHLY, in_a_month(), None),
            APPLE_ROOT_CA_G3,
        );
        assert_eq!(
            verify(APPLE_ROOT_CA_G3, &jws).unwrap_err(),
            Refusal::Signature
        );
    }

    #[test]
    fn a_chain_under_another_root_is_refused() {
        let chain = Chain::new();
        let other = Chain::new();
        let jws = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        assert_eq!(verify(&other.root, &jws).unwrap_err(), Refusal::Chain);
        assert_eq!(verify(APPLE_ROOT_CA_G3, &jws).unwrap_err(), Refusal::Chain);
    }

    #[test]
    fn a_tampered_payload_is_refused() {
        let chain = Chain::new();
        let jws = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        let mut parts: Vec<&str> = jws.split('.').collect();
        let forged = transaction_payload(
            "1",
            MONTHLY,
            OffsetDateTime::now_utc() + Duration::days(3650),
            None,
        );
        let forged = URL_SAFE_NO_PAD.encode(forged.to_string());
        parts[1] = &forged;
        assert_eq!(
            verify(&chain.root, &parts.join(".")).unwrap_err(),
            Refusal::Signature
        );
    }

    #[test]
    fn junk_is_malformed_and_never_panics() {
        let chain = Chain::new();
        for junk in [
            "",
            "a.b",
            "a.b.c.d",
            "!!!.!!!.!!!",
            "e30.e30.",
            "e30.e30.AAAA",
        ] {
            assert!(verify(&chain.root, junk).is_err(), "{junk:?}");
        }
        // A header with another algorithm.
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","x5c":[]}"#);
        let body = URL_SAFE_NO_PAD.encode("{}");
        assert_eq!(
            verify(&chain.root, &format!("{header}.{body}.")).unwrap_err(),
            Refusal::Algorithm
        );
    }

    #[test]
    fn the_policy_holds_bundle_product_environment_and_type() {
        let chain = Chain::new();
        let (p, e) = (products(), environments());
        let policy = policy(&chain.root, &p, &e);
        let base = transaction_payload("1", MONTHLY, in_a_month(), None);

        let mut other_app = base.clone();
        other_app["bundleId"] = "me.nettrash.Scan".into();
        assert_eq!(
            policy.transaction(&chain.sign(&other_app)).unwrap_err(),
            Refusal::Bundle
        );

        let mut other_product = base.clone();
        other_product["productId"] = "me.nettrash.FamilyConnect.stickers".into();
        assert_eq!(
            policy.transaction(&chain.sign(&other_product)).unwrap_err(),
            Refusal::Product
        );

        let mut xcode = base.clone();
        xcode["environment"] = "Xcode".into();
        assert_eq!(
            policy.transaction(&chain.sign(&xcode)).unwrap_err(),
            Refusal::Environment
        );

        let mut consumable = base.clone();
        consumable["type"] = "Consumable".into();
        assert_eq!(
            policy.transaction(&chain.sign(&consumable)).unwrap_err(),
            Refusal::Kind
        );

        let production_only = vec!["Production".to_string()];
        let strict = Policy {
            environments: &production_only,
            ..policy
        };
        let mut sandbox = base.clone();
        sandbox["environment"] = "Sandbox".into();
        assert_eq!(
            strict.transaction(&chain.sign(&sandbox)).unwrap_err(),
            Refusal::Environment
        );
    }

    #[test]
    fn a_revocation_is_read() {
        let chain = Chain::new();
        let mut payload = transaction_payload("1", MONTHLY, in_a_month(), None);
        payload["revocationDate"] = ms(OffsetDateTime::now_utc()).into();
        payload["revocationReason"] = 0.into();
        let (p, e) = (products(), environments());
        let txn = policy(&chain.root, &p, &e)
            .transaction(&chain.sign(&payload))
            .expect("believed");
        assert!(txn.revoked.is_some());
    }

    #[test]
    fn renewal_info_is_read() {
        let chain = Chain::new();
        let grace = OffsetDateTime::now_utc() + Duration::days(16);
        let payload = serde_json::json!({
            "originalTransactionId": "1",
            "autoRenewProductId": MONTHLY,
            "productId": MONTHLY,
            "autoRenewStatus": 0,
            "isInBillingRetryPeriod": true,
            "gracePeriodExpiresDate": ms(grace),
            "signedDate": ms(OffsetDateTime::now_utc()),
            "environment": "Sandbox",
        });
        let (p, e) = (products(), environments());
        let renewal = policy(&chain.root, &p, &e)
            .renewal(&chain.sign(&payload))
            .expect("believed");
        assert_eq!(renewal.renews, Some(false));
        assert_eq!(renewal.original_transaction_id, "1");
        assert_eq!(
            renewal.grace_until.map(|g| g.unix_timestamp()),
            Some(grace.unix_timestamp())
        );
    }

    #[test]
    fn a_notification_carries_its_inner_values_unverified_until_asked() {
        let chain = Chain::new();
        let inner = chain.sign(&transaction_payload("1", MONTHLY, in_a_month(), None));
        let payload = serde_json::json!({
            "notificationType": "DID_RENEW",
            "notificationUUID": "6f1c0e2a-0000-4000-8000-000000000000",
            "version": "2.0",
            "signedDate": ms(OffsetDateTime::now_utc()),
            "data": {
                "bundleId": "me.nettrash.FamilyConnect",
                "environment": "Production",
                "signedTransactionInfo": inner,
            },
        });
        let (p, e) = (products(), environments());
        let policy = policy(&chain.root, &p, &e);
        let note = policy
            .notification(&chain.sign(&payload))
            .expect("believed");
        assert_eq!(note.notification_type, "DID_RENEW");
        assert_eq!(note.subtype, None);
        assert_eq!(note.bundle_id.as_deref(), Some("me.nettrash.FamilyConnect"));
        assert!(note.signed_renewal.is_none());
        let txn = policy
            .transaction(note.signed_transaction.as_deref().expect("inner"))
            .expect("inner believed");
        assert_eq!(txn.original_transaction_id, "1");
    }
}
