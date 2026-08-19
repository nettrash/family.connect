//! Token and invite-code generation — small pure functions, heavily tested.
//!
//! Session tokens are opaque 256-bit random values; the database stores only
//! their SHA-256, so a leaked `sessions` table cannot be replayed. Invite
//! codes use a 30-character alphabet with the easily-confused glyphs
//! (0/O, 1/I/L, U/V ambiguity — U dropped) removed, because humans read
//! these aloud across a dinner table.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Alphabet for invite codes: digits and uppercase letters minus 0, 1, I, L,
/// O, U — 30 symbols, so one random byte in `0..240` maps uniformly via `% 30`.
pub const INVITE_ALPHABET: &[u8; 30] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Invite codes are 8 symbols: 30^8 ≈ 6.6 * 10^11 combinations — far beyond
/// online guessing at family scale, still short enough to read aloud.
pub const INVITE_CODE_LEN: usize = 8;

/// Generate an opaque session token: 32 bytes from the OS CSPRNG, encoded as
/// URL-safe unpadded base64 (always 43 characters).
pub fn gen_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hex-encoded SHA-256 of a token — the only form ever stored server-side.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Generate an invite code via rejection sampling: bytes >= 240 are thrown
/// away so `byte % 30` stays unbiased (240 is the largest multiple of 30
/// that fits in a byte).
pub fn gen_invite_code() -> String {
    let mut code = String::with_capacity(INVITE_CODE_LEN);
    let mut buf = [0u8; 16];
    while code.len() < INVITE_CODE_LEN {
        rand::rngs::OsRng.fill_bytes(&mut buf);
        for &byte in &buf {
            if byte >= 240 {
                continue;
            }
            code.push(INVITE_ALPHABET[(byte % 30) as usize] as char);
            if code.len() == INVITE_CODE_LEN {
                break;
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_tokens_are_43_url_safe_characters_decoding_to_32_bytes() {
        let token = gen_session_token();
        assert_eq!(token.len(), 43);
        let decoded = URL_SAFE_NO_PAD
            .decode(&token)
            .expect("token must be valid unpadded url-safe base64");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn consecutive_session_tokens_differ() {
        assert_ne!(gen_session_token(), gen_session_token());
    }

    #[test]
    fn hash_token_matches_the_known_sha256_test_vector() {
        // FIPS 180-2 test vector: SHA-256("abc").
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn different_tokens_hash_differently() {
        assert_ne!(hash_token("a"), hash_token("b"));
    }

    #[test]
    fn invite_codes_are_8_characters_drawn_from_the_alphabet() {
        for _ in 0..64 {
            let code = gen_invite_code();
            assert_eq!(code.len(), INVITE_CODE_LEN);
            for c in code.bytes() {
                assert!(
                    INVITE_ALPHABET.contains(&c),
                    "invite code contains {c:?} outside the alphabet"
                );
            }
        }
    }

    #[test]
    fn consecutive_invite_codes_differ() {
        // 30^8 combinations — a collision here means the RNG is broken.
        assert_ne!(gen_invite_code(), gen_invite_code());
    }

    #[test]
    fn the_invite_alphabet_has_no_easily_confused_glyphs() {
        for banned in [b'0', b'1', b'I', b'L', b'O', b'U'] {
            assert!(!INVITE_ALPHABET.contains(&banned));
        }
    }
}
