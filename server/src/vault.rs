//! Encryption at rest (#59): the keys, the sealed value, the lookups and the
//! sealed file — as pure functions over bytes.
//!
//! The design and its reasons are `docs/encryption-at-rest-2026-10-24.md`;
//! what it promises a reader of the protocol is docs/protocol.md, "Stored
//! encrypted". In one paragraph: one SERVER key lives in a file outside the
//! database and outside every backup. Every family, conversation and account
//! has a DATA key of its own, kept in the database wrapped by the server key,
//! and every stored file has one too. A value is sealed under the data key of
//! what it belongs to and names that key in its first bytes, so opening it
//! needs no context. What the server must FIND by value — a username, an invite
//! code, a push token — is looked up by an HMAC instead, because a sealed value
//! never equals another sealing of the same words.
//!
//! Nothing here reads the database or an attachment; the one file it touches is
//! the server key's own. Every failure to open is an [`OpenError`], which never
//! carries a byte of what it failed to open — a log line or a 500 built from one
//! cannot leak content.
//!
//! The formats are pinned by fixed vectors in the tests below. Changing a byte
//! of any of them makes every family's data unreadable, so a test that fails
//! there is never "updated to match".

use std::fmt;
use std::path::Path;

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

/// Every key here — the server key, a data key, a file key — is 256 bits.
pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// The first byte of every sealed value.
pub const SEALED: u8 = 0x01;
/// The first byte the upgrade puts in front of text it has not sealed yet
/// (the design's §4). A value that still starts with it is [`OpenError::NotSealed`].
pub const NOT_YET_SEALED: u8 = 0x00;
/// `SEALED`, then the data key's id.
const VALUE_HEADER_LEN: usize = 1 + 8;
/// What sealing adds to a value: header, nonce and tag.
pub const VALUE_OVERHEAD: usize = VALUE_HEADER_LEN + NONCE_LEN + TAG_LEN;
/// A data key wrapped by the server key: nonce, the sealed key, tag.
pub const WRAPPED_KEY_LEN: usize = NONCE_LEN + KEY_LEN + TAG_LEN;

/// The first bytes of every sealed file.
pub const FILE_MAGIC: &[u8; 4] = b"FCS1";
const FILE_PREFIX_LEN: usize = NONCE_LEN - 4 - 1;
/// The magic and the file's nonce prefix.
pub const FILE_HEADER_LEN: usize = FILE_MAGIC.len() + FILE_PREFIX_LEN;
/// Plaintext bytes per chunk of a sealed file. A byte range is served by
/// opening only the chunks it touches, so this is also the most a range
/// request reads that it does not send.
pub const CHUNK_LEN: usize = 64 * 1024;
const SEALED_CHUNK_LEN: usize = CHUNK_LEN + TAG_LEN;

const LABEL_WRAP: &[u8] = b"family.connect/v1/wrap";
const LABEL_LOOKUP: &[u8] = b"family.connect/v1/lookup";
const LABEL_FINGERPRINT: &[u8] = b"family.connect/v1/fingerprint";
const LABEL_DEDUP: &[u8] = b"family.connect/v1/dedup";
const AAD_KEY: &[u8] = b"family.connect/v1/key";

/// Why a value, a wrapped key or a file would not open. Deliberately says
/// nothing about the bytes themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// Text the upgrade has not sealed yet: it still starts with
    /// [`NOT_YET_SEALED`].
    NotSealed,
    /// Neither a sealed value nor a sealed file: an unknown first byte or magic.
    UnknownFormat,
    /// Too short, or not the length its own header says it must be.
    Truncated,
    /// The authentication failed: altered bytes, the wrong key, a chunk out of
    /// place or a file cut short.
    Rejected,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OpenError::NotSealed => "the value has not been sealed yet",
            OpenError::UnknownFormat => "not a sealed value or file",
            OpenError::Truncated => "a sealed value or file of the wrong length",
            OpenError::Rejected => "a sealed value or file that does not open with its key",
        })
    }
}

impl std::error::Error for OpenError {}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

fn derive(key: &[u8; KEY_LEN], label: &[u8]) -> [u8; KEY_LEN] {
    let mut out = [0u8; KEY_LEN];
    Hkdf::<Sha256>::new(None, key)
        .expand(label, &mut out)
        .expect("32 bytes is far below what HKDF-SHA256 can expand");
    out
}

fn hmac(key: &[u8; KEY_LEN], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes a key of any length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

fn cipher(key: &[u8; KEY_LEN]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(key).expect("the key is 32 bytes")
}

fn seal_raw(
    cipher: &XChaCha20Poly1305,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    msg: &[u8],
) -> Vec<u8> {
    cipher
        .encrypt(&XNonce::from(*nonce), Payload { msg, aad })
        .expect("nothing sealed here comes near the cipher's 256 GiB limit")
}

fn open_raw(
    cipher: &XChaCha20Poly1305,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    msg: &[u8],
) -> Result<Vec<u8>, OpenError> {
    cipher
        .decrypt(&XNonce::from(*nonce), Payload { msg, aad })
        .map_err(|_| OpenError::Rejected)
}

/// The server key: the one secret in `[storage] key_file`. It never seals
/// content itself — it wraps the data keys, keys the lookups, and gives the
/// fingerprint that tells a wrong key file from the right one.
#[derive(Clone)]
pub struct ServerKey {
    raw: [u8; KEY_LEN],
    wrap: [u8; KEY_LEN],
    lookup: [u8; KEY_LEN],
    fingerprint: String,
}

impl fmt::Debug for ServerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServerKey({})", self.fingerprint)
    }
}

impl ServerKey {
    fn from_raw(raw: [u8; KEY_LEN]) -> Self {
        let fingerprint = hex::encode(&derive(&raw, LABEL_FINGERPRINT)[..8]);
        Self {
            wrap: derive(&raw, LABEL_WRAP),
            lookup: derive(&raw, LABEL_LOOKUP),
            fingerprint,
            raw,
        }
    }

    /// A new key from the OS's randomness — `storage-key new` and `rotate`.
    pub fn generate() -> Self {
        Self::from_raw(random())
    }

    /// A key file's contents: 32 bytes in standard base64, surrounding
    /// whitespace ignored. The error never repeats what it was given.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let bytes = STANDARD
            .decode(text.trim())
            .ok()
            .and_then(|bytes| <[u8; KEY_LEN]>::try_from(bytes).ok());
        match bytes {
            Some(raw) => Ok(Self::from_raw(raw)),
            None => bail!("not a storage key: expected 32 bytes in base64"),
        }
    }

    /// Read the key file the server is configured with. A file every user on
    /// the machine can read is refused rather than used: it would put the one
    /// secret this design rests on in the hands of anybody with a shell.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let place = || format!("the storage key file {}", path.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path)
                .with_context(place)?
                .permissions()
                .mode();
            if mode & 0o007 != 0 {
                bail!(
                    "{} can be read or written by every user on this machine; make it 0400",
                    place()
                );
            }
        }
        let text = std::fs::read_to_string(path).with_context(place)?;
        Self::parse(&text).with_context(place)
    }

    /// Write this key to a NEW file, readable by its owner only. Refuses to
    /// replace an existing file: overwriting a key file is how a server loses
    /// every family it holds.
    pub fn create_file(&self, path: &Path) -> anyhow::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o400);
        }
        let mut file = options
            .open(path)
            .with_context(|| format!("creating the storage key file {}", path.display()))?;
        writeln!(file, "{}", self.encoded())
            .and_then(|()| file.sync_all())
            .with_context(|| format!("writing the storage key file {}", path.display()))
    }

    /// The key as a key file holds it — for `storage-key show`, and nothing
    /// else. Never logged.
    pub fn encoded(&self) -> String {
        STANDARD.encode(self.raw)
    }

    /// Sixteen hex digits that name this key without revealing it: stored in
    /// the database, so a server started with the wrong key file says so, and
    /// shown by `storage-key show` beside the key.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Wrap a data key for `storage_keys`, bound to the row id it is stored
    /// under: a wrapped key copied to another row does not unwrap there.
    pub fn wrap(&self, key_id: i64, key: &DataKey) -> Vec<u8> {
        self.wrap_with_nonce(key_id, key, &random())
    }

    fn wrap_with_nonce(&self, key_id: i64, key: &DataKey, nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
        let aad = [AAD_KEY, &key_id.to_be_bytes()].concat();
        let mut out = nonce.to_vec();
        out.extend(seal_raw(&cipher(&self.wrap), nonce, &aad, &key.0));
        out
    }

    /// The data key stored under `key_id`.
    pub fn unwrap(&self, key_id: i64, wrapped: &[u8]) -> Result<DataKey, OpenError> {
        if wrapped.len() != WRAPPED_KEY_LEN {
            return Err(OpenError::Truncated);
        }
        let (nonce, sealed) = wrapped.split_at(NONCE_LEN);
        let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| OpenError::Truncated)?;
        let aad = [AAD_KEY, &key_id.to_be_bytes()].concat();
        let raw = open_raw(&cipher(&self.wrap), nonce, &aad, sealed)?;
        <[u8; KEY_LEN]>::try_from(raw)
            .map(DataKey)
            .map_err(|_| OpenError::Truncated)
    }

    /// The lookup column's value for `value`, normalised the way the server
    /// has always matched it (see [`Lookup`]). Equal inputs give equal
    /// outputs, and nothing else can be learnt from one without this key.
    pub fn lookup(&self, what: Lookup, value: &str) -> Vec<u8> {
        let normalised = what.normalise(value);
        hmac(&self.lookup, &[what.label(), &[0u8], normalised.as_bytes()]).to_vec()
    }
}

/// What the server finds by value, before any family is known — so these are
/// keyed server-wide rather than by a family's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// Signing in matched `lower(username)`; usernames are ASCII by rule, so
    /// ASCII lower case is the same match.
    Username,
    /// Codes are drawn from an upper-case alphabet and the join upper-cases
    /// what was typed.
    InviteCode,
    /// A device's push token, as the platform issued it.
    PushToken,
    /// A device's VoIP token, as the platform issued it.
    VoipToken,
    /// An App Store transaction id — Apple's `originalTransactionId`, and
    /// the id of the latest transaction beside it (docs/protocol.md, "Paid
    /// families"). A notification names the subscription only by it.
    AppStoreTransaction,
}

impl Lookup {
    fn label(self) -> &'static [u8] {
        match self {
            Lookup::Username => b"username",
            Lookup::InviteCode => b"invite_code",
            Lookup::PushToken => b"push_token",
            Lookup::VoipToken => b"voip_token",
            Lookup::AppStoreTransaction => b"appstore_transaction",
        }
    }

    fn normalise(self, value: &str) -> String {
        match self {
            Lookup::Username => value.to_ascii_lowercase(),
            Lookup::InviteCode => value.to_uppercase(),
            Lookup::PushToken | Lookup::VoipToken | Lookup::AppStoreTransaction => {
                value.to_string()
            }
        }
    }
}

/// A data key: a family's, a conversation's, an account's or one file's.
pub struct DataKey([u8; KEY_LEN]);

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey(..)")
    }
}

impl DataKey {
    /// A new key from the OS's randomness.
    pub fn generate() -> Self {
        Self(random())
    }

    /// Seal `plaintext` under this key, which is stored as `key_id`. The id is
    /// the value's first bytes after [`SEALED`], so opening it needs no
    /// context, and it is authenticated: a value whose id is altered does not
    /// open under either key.
    pub fn seal(&self, key_id: i64, plaintext: &[u8]) -> Vec<u8> {
        self.seal_with_nonce(key_id, plaintext, &random())
    }

    fn seal_with_nonce(&self, key_id: i64, plaintext: &[u8], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
        let mut out = Vec::with_capacity(VALUE_OVERHEAD + plaintext.len());
        out.push(SEALED);
        out.extend_from_slice(&key_id.to_be_bytes());
        out.extend_from_slice(nonce);
        let sealed = seal_raw(&cipher(&self.0), nonce, &out[..VALUE_HEADER_LEN], plaintext);
        out.extend(sealed);
        out
    }

    /// Open a value this key sealed. Which key that is comes from
    /// [`key_id_of`].
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, OpenError> {
        key_id_of(sealed)?;
        let (header, rest) = sealed.split_at(VALUE_HEADER_LEN);
        let (nonce, body) = rest.split_at(NONCE_LEN);
        let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| OpenError::Truncated)?;
        open_raw(&cipher(&self.0), nonce, header, body)
    }

    /// A file's key from the bytes it was sealed as (the row's `file_key`,
    /// opened). Anything but 32 bytes is no key.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        <[u8; KEY_LEN]>::try_from(bytes).ok().map(Self)
    }

    /// The key's own bytes — ONLY to be sealed as a value under another key
    /// (a file's key, kept in its attachment row). Never logged, never sent.
    pub fn secret_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The dedup column's value for a file whose plaintext SHA-256 is
    /// `sha256`: hex, as the column has always held, but keyed by the family,
    /// so an old backup cannot be asked whether a family once had a known
    /// picture — and it goes with the family's key.
    pub fn content_hash(&self, sha256: &[u8]) -> String {
        hex::encode(hmac(&derive(&self.0, LABEL_DEDUP), &[sha256]))
    }
}

/// The id of the data key a sealed value names — what the server looks up
/// before it can open the value.
pub fn key_id_of(sealed: &[u8]) -> Result<i64, OpenError> {
    match sealed.first() {
        None => Err(OpenError::Truncated),
        Some(&NOT_YET_SEALED) => Err(OpenError::NotSealed),
        Some(&SEALED) if sealed.len() >= VALUE_OVERHEAD => {
            let id: [u8; 8] = sealed[1..VALUE_HEADER_LEN]
                .try_into()
                .map_err(|_| OpenError::Truncated)?;
            Ok(i64::from_be_bytes(id))
        }
        Some(&SEALED) => Err(OpenError::Truncated),
        Some(_) => Err(OpenError::UnknownFormat),
    }
}

/// How many chunks a file of `plain_len` bytes is sealed in. An empty file is
/// still one (empty) chunk, so that cutting a file down to its header is
/// detected like any other truncation.
fn chunk_count(plain_len: u64) -> u64 {
    plain_len.div_ceil(CHUNK_LEN as u64).max(1)
}

/// The size on disk of a file of `plain_len` bytes, once sealed.
pub fn sealed_file_len(plain_len: u64) -> u64 {
    FILE_HEADER_LEN as u64 + plain_len + chunk_count(plain_len) * TAG_LEN as u64
}

/// The plaintext size of a sealed file `sealed_len` bytes long, or `None` when
/// no sealed file can be that long.
pub fn plain_file_len(sealed_len: u64) -> Option<u64> {
    let body = sealed_len.checked_sub(FILE_HEADER_LEN as u64)?;
    let tag = TAG_LEN as u64;
    // Every chunk but the last is a full SEALED_CHUNK_LEN; the last carries
    // at least its tag.
    let chunks = body.checked_sub(tag)? / SEALED_CHUNK_LEN as u64 + 1;
    Some(body - chunks * tag)
}

fn chunk_nonce(prefix: &[u8; FILE_PREFIX_LEN], index: u32, last: bool) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..FILE_PREFIX_LEN].copy_from_slice(prefix);
    nonce[FILE_PREFIX_LEN..NONCE_LEN - 1].copy_from_slice(&index.to_be_bytes());
    nonce[NONCE_LEN - 1] = u8::from(last);
    nonce
}

/// Seals a file as it streams in: [`FILE_MAGIC`] and a random nonce prefix,
/// then the plaintext in [`CHUNK_LEN`] chunks, each its own AEAD with the nonce
/// `prefix | chunk number | last-chunk flag` — the STREAM construction age and
/// Tink use. A chunk moved, dropped or duplicated, or a file cut short at a
/// chunk boundary, then fails to open.
pub struct FileSealer {
    cipher: XChaCha20Poly1305,
    prefix: [u8; FILE_PREFIX_LEN],
    next: u32,
    pending: Vec<u8>,
}

impl FileSealer {
    /// A sealer, and the header to write before anything it returns.
    pub fn new(key: &DataKey) -> (Self, Vec<u8>) {
        Self::with_prefix(key, random())
    }

    fn with_prefix(key: &DataKey, prefix: [u8; FILE_PREFIX_LEN]) -> (Self, Vec<u8>) {
        let header = [FILE_MAGIC.as_slice(), &prefix].concat();
        let sealer = Self {
            cipher: cipher(&key.0),
            prefix,
            next: 0,
            pending: Vec::with_capacity(CHUNK_LEN),
        };
        (sealer, header)
    }

    fn seal_chunk(&mut self, plain: &[u8], last: bool) -> Vec<u8> {
        let nonce = chunk_nonce(&self.prefix, self.next, last);
        self.next = self
            .next
            .checked_add(1)
            .expect("2^32 chunks is 256 TiB, far beyond any upload limit");
        seal_raw(&self.cipher, &nonce, FILE_MAGIC, plain)
    }

    /// Take the next bytes of the file; returns the sealed bytes to write
    /// after everything returned before. A full chunk is held back until more
    /// arrives, because only then is it known not to be the last.
    pub fn push(&mut self, mut data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        while !data.is_empty() {
            if self.pending.len() == CHUNK_LEN {
                let chunk = std::mem::take(&mut self.pending);
                out.extend(self.seal_chunk(&chunk, false));
                self.pending = chunk;
                self.pending.clear();
            }
            let room = CHUNK_LEN - self.pending.len();
            let (now, later) = data.split_at(room.min(data.len()));
            self.pending.extend_from_slice(now);
            data = later;
        }
        out
    }

    /// The end of the file: the last chunk, sealed as the last.
    pub fn finish(mut self) -> Vec<u8> {
        let chunk = std::mem::take(&mut self.pending);
        self.seal_chunk(&chunk, true)
    }
}

/// Opens a sealed file a chunk at a time, so a byte range costs only the
/// chunks it touches.
pub struct FileOpener {
    cipher: XChaCha20Poly1305,
    prefix: [u8; FILE_PREFIX_LEN],
    plain_len: u64,
    chunks: u64,
}

impl FileOpener {
    /// `header` is the file's first [`FILE_HEADER_LEN`] bytes, and
    /// `sealed_len` its size on disk.
    pub fn new(key: &DataKey, header: &[u8], sealed_len: u64) -> Result<Self, OpenError> {
        if header.len() != FILE_HEADER_LEN {
            return Err(OpenError::Truncated);
        }
        let (magic, prefix) = header.split_at(FILE_MAGIC.len());
        if magic != FILE_MAGIC {
            return Err(OpenError::UnknownFormat);
        }
        let plain_len = plain_file_len(sealed_len).ok_or(OpenError::Truncated)?;
        let chunks = chunk_count(plain_len);
        if chunks > u64::from(u32::MAX) {
            return Err(OpenError::Truncated);
        }
        Ok(Self {
            cipher: cipher(&key.0),
            prefix: prefix.try_into().map_err(|_| OpenError::Truncated)?,
            plain_len,
            chunks,
        })
    }

    /// The file's plaintext size.
    pub fn plain_len(&self) -> u64 {
        self.plain_len
    }

    /// Where chunk `index` lies in the sealed file: its offset and length.
    pub fn chunk_span(&self, index: u64) -> (u64, usize) {
        let offset = FILE_HEADER_LEN as u64 + index * SEALED_CHUNK_LEN as u64;
        let plain = if index + 1 < self.chunks {
            CHUNK_LEN as u64
        } else {
            self.plain_len - index * CHUNK_LEN as u64
        };
        (offset, plain as usize + TAG_LEN)
    }

    /// The chunks that hold plaintext bytes `start..start + count`, first and
    /// last inclusive. An empty range touches the chunk `start` falls in.
    pub fn chunks_covering(&self, start: u64, count: u64) -> std::ops::RangeInclusive<u64> {
        let last_chunk = self.chunks - 1;
        let first = (start / CHUNK_LEN as u64).min(last_chunk);
        let last = (start + count.max(1) - 1) / CHUNK_LEN as u64;
        first..=last.min(last_chunk)
    }

    /// Open chunk `index`, given exactly the bytes [`Self::chunk_span`] names.
    pub fn open_chunk(&self, index: u64, sealed: &[u8]) -> Result<Vec<u8>, OpenError> {
        if index >= self.chunks || sealed.len() != self.chunk_span(index).1 {
            return Err(OpenError::Truncated);
        }
        let number = u32::try_from(index).map_err(|_| OpenError::Truncated)?;
        let nonce = chunk_nonce(&self.prefix, number, index + 1 == self.chunks);
        open_raw(&self.cipher, &nonce, FILE_MAGIC, sealed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_key(byte: u8) -> DataKey {
        DataKey([byte; KEY_LEN])
    }

    fn server_key(byte: u8) -> ServerKey {
        ServerKey::from_raw([byte; KEY_LEN])
    }

    /// Bytes that look like nothing in particular, so a chunk boundary in
    /// the wrong place shows up as a mismatch.
    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn seal_file(key: &DataKey, plain: &[u8], piece: usize) -> Vec<u8> {
        let (mut sealer, mut out) = FileSealer::new(key);
        for part in plain.chunks(piece.max(1)) {
            out.extend(sealer.push(part));
        }
        out.extend(sealer.finish());
        out
    }

    fn open_range(
        key: &DataKey,
        file: &[u8],
        start: u64,
        count: u64,
    ) -> Result<Vec<u8>, OpenError> {
        let opener = FileOpener::new(key, &file[..FILE_HEADER_LEN], file.len() as u64)?;
        let mut plain = Vec::new();
        let mut first_offset = None;
        for index in opener.chunks_covering(start, count) {
            let (offset, len) = opener.chunk_span(index);
            let offset = offset as usize;
            plain.extend(opener.open_chunk(index, &file[offset..offset + len])?);
            first_offset.get_or_insert(index * CHUNK_LEN as u64);
        }
        let skip = (start - first_offset.unwrap_or(0)) as usize;
        Ok(plain[skip..skip + count as usize].to_vec())
    }

    // --- the formats, pinned ------------------------------------------------
    //
    // Every expected value below was computed independently on 2026-10-25,
    // not copied from this code's output: HChaCha20 written out by hand
    // (checked against draft-irtf-cfrg-xchacha-03 §2.2.1), ChaCha20-Poly1305
    // from Python's `cryptography`, HKDF and HMAC from its standard library.

    #[test]
    fn a_sealed_value_is_byte_for_byte_what_it_was_on_2026_10_25() {
        let sealed = data_key(7).seal_with_nonce(42, b"Hello, family", &[9; NONCE_LEN]);
        assert_eq!(
            hex::encode(&sealed),
            "01000000000000002a090909090909090909090909090909090909090909090909f6178ee613865904957f6150113b476a4fbecb918f8a6d39da7535d01a"
        );
        assert_eq!(data_key(7).open(&sealed).expect("opens"), b"Hello, family");
    }

    #[test]
    fn a_wrapped_key_is_byte_for_byte_what_it_was_on_2026_10_25() {
        let wrapped = server_key(3).wrap_with_nonce(5, &data_key(7), &[1; NONCE_LEN]);
        assert_eq!(
            hex::encode(&wrapped),
            "010101010101010101010101010101010101010101010101a279577781e0d17ea2392032d0338ce22dd726589f55c5bc490d0523efe1014dc914103d8353fb79a67e5ae95961181c"
        );
    }

    #[test]
    fn the_derived_values_are_what_they_were_on_2026_10_25() {
        let key = server_key(3);
        assert_eq!(key.fingerprint(), "ac4c268fcd2f15d3");
        assert_eq!(
            hex::encode(key.lookup(Lookup::Username, "Anna")),
            "a8a61337c8b42e2cebd85c807f7e522c57b529f03e9f29490ca0c6e68c54ff87"
        );
        assert_eq!(
            data_key(7).content_hash(&[0xab; 32]),
            "b86e7e4095eeaf00969a6fc63115f0f7d2ccf555e76bf8f86a8cc5e754eea3eb"
        );
    }

    #[test]
    fn a_sealed_file_is_byte_for_byte_what_it_was_on_2026_10_25() {
        let (mut sealer, mut file) = FileSealer::with_prefix(&data_key(7), [2; FILE_PREFIX_LEN]);
        file.extend(sealer.push(b"a photo"));
        file.extend(sealer.finish());
        assert_eq!(
            hex::encode(&file),
            "46435331020202020202020202020202020202020202028ddb99e5bca9b68e00e8c992072d568e9539f71abd8c72"
        );
    }

    // --- values -------------------------------------------------------------

    #[test]
    fn a_value_opens_to_what_was_sealed_and_names_its_key() {
        let key = DataKey::generate();
        for plain in [&b""[..], b"x", "Привет, семья 👋".as_bytes(), &sample(5000)] {
            let sealed = key.seal(1234, plain);
            assert_eq!(sealed.len(), plain.len() + VALUE_OVERHEAD);
            assert_eq!(key_id_of(&sealed), Ok(1234));
            assert_eq!(key.open(&sealed).expect("opens"), plain);
        }
        assert_eq!(VALUE_OVERHEAD, 49, "the design and the docs say 49");
    }

    #[test]
    fn the_same_words_never_seal_the_same_way_twice() {
        let key = DataKey::generate();
        assert_ne!(key.seal(1, b"same"), key.seal(1, b"same"));
    }

    #[test]
    fn every_altered_byte_is_refused() {
        let key = data_key(7);
        let sealed = key.seal(42, b"Hello, family");
        for at in 0..sealed.len() {
            let mut altered = sealed.clone();
            altered[at] ^= 0x01;
            let opened = key.open(&altered);
            assert!(opened.is_err(), "byte {at} altered still opened");
        }
    }

    #[test]
    fn the_wrong_key_and_a_changed_key_id_are_refused() {
        let sealed = data_key(7).seal(42, b"Hello, family");
        assert_eq!(data_key(8).open(&sealed), Err(OpenError::Rejected));
        let mut renamed = sealed.clone();
        renamed[VALUE_HEADER_LEN - 1] ^= 0x01;
        assert_eq!(key_id_of(&renamed), Ok(43));
        assert_eq!(data_key(7).open(&renamed), Err(OpenError::Rejected));
    }

    #[test]
    fn what_is_not_a_sealed_value_says_which_way_it_is_not() {
        let key = data_key(7);
        assert_eq!(key.open(b""), Err(OpenError::Truncated));
        assert_eq!(key.open(b"\x00plain words"), Err(OpenError::NotSealed));
        assert_eq!(key.open(b"plain words"), Err(OpenError::UnknownFormat));
        assert_eq!(
            key.open(&[SEALED; VALUE_OVERHEAD - 1]),
            Err(OpenError::Truncated)
        );
        for error in [
            OpenError::NotSealed,
            OpenError::UnknownFormat,
            OpenError::Truncated,
            OpenError::Rejected,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    // --- the server key -----------------------------------------------------

    #[test]
    fn a_key_file_round_trips_and_is_never_echoed() {
        let key = ServerKey::generate();
        let again = ServerKey::parse(&format!("  {}\n", key.encoded())).expect("parses");
        assert_eq!(again.fingerprint(), key.fingerprint());
        assert_eq!(key.fingerprint().len(), 16);
        assert!(ServerKey::parse("").is_err());
        for bad in ["not base64 at all!", &STANDARD.encode([1u8; 31])] {
            let error = ServerKey::parse(bad).expect_err("refused").to_string();
            assert!(
                !error.contains(bad),
                "the error repeated its input: {error}"
            );
        }
        let shown = format!("{key:?}");
        assert!(!shown.contains(&key.encoded()) && shown.contains(key.fingerprint()));
        assert_eq!(format!("{:?}", data_key(7)), "DataKey(..)");
    }

    #[test]
    fn a_new_key_file_is_private_and_never_overwritten() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("storage.key");
        let key = ServerKey::generate();
        key.create_file(&path).expect("created");
        assert_eq!(
            ServerKey::load(&path).expect("loads").fingerprint(),
            key.fingerprint()
        );
        assert!(
            ServerKey::generate().create_file(&path).is_err(),
            "overwrote a key file"
        );
        assert_eq!(
            ServerKey::load(&path).expect("still loads").fingerprint(),
            key.fingerprint()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, 0o400);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_everybody_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("storage.key");
        std::fs::write(&path, ServerKey::generate().encoded()).expect("written");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let error = format!("{:#}", ServerKey::load(&path).expect_err("refused"));
        assert!(error.contains("every user"), "{error}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("chmod");
        ServerKey::load(&path).expect("a group-readable file, like a systemd credential, loads");
        let missing = format!(
            "{:#}",
            ServerKey::load(&dir.path().join("nope")).expect_err("no file")
        );
        assert!(missing.contains("nope"), "{missing}");
    }

    #[test]
    fn a_wrapped_key_unwraps_only_under_its_own_id_and_server_key() {
        let server = server_key(3);
        let key = DataKey::generate();
        let wrapped = server.wrap(5, &key);
        assert_eq!(wrapped.len(), WRAPPED_KEY_LEN);
        let unwrapped = server.unwrap(5, &wrapped).expect("unwraps");
        let sealed = key.seal(5, b"words");
        assert_eq!(unwrapped.open(&sealed).expect("same key"), b"words");
        assert_eq!(
            server.unwrap(6, &wrapped).map(|_| ()),
            Err(OpenError::Rejected)
        );
        assert_eq!(
            server_key(4).unwrap(5, &wrapped).map(|_| ()),
            Err(OpenError::Rejected)
        );
        assert_eq!(
            server.unwrap(5, &wrapped[1..]).map(|_| ()),
            Err(OpenError::Truncated)
        );
    }

    #[test]
    fn lookups_match_the_way_the_server_always_matched() {
        let key = server_key(3);
        assert_eq!(
            key.lookup(Lookup::Username, "Anna.K_1"),
            key.lookup(Lookup::Username, "anna.k_1")
        );
        assert_eq!(
            key.lookup(Lookup::InviteCode, "abcd2345"),
            key.lookup(Lookup::InviteCode, "ABCD2345")
        );
        assert_ne!(
            key.lookup(Lookup::PushToken, "abc"),
            key.lookup(Lookup::PushToken, "ABC"),
            "a token is matched exactly"
        );
        assert_ne!(
            key.lookup(Lookup::PushToken, "same"),
            key.lookup(Lookup::VoipToken, "same"),
            "each lookup is its own domain"
        );
        assert_ne!(
            key.lookup(Lookup::Username, "anna"),
            server_key(4).lookup(Lookup::Username, "anna")
        );
        assert_eq!(key.lookup(Lookup::Username, "anna").len(), 32);
    }

    #[test]
    fn dedup_is_keyed_by_the_family() {
        let sha = [0x11; 32];
        assert_eq!(
            data_key(7).content_hash(&sha),
            data_key(7).content_hash(&sha)
        );
        assert_ne!(
            data_key(7).content_hash(&sha),
            data_key(8).content_hash(&sha)
        );
        assert_ne!(
            data_key(7).content_hash(&sha),
            data_key(7).content_hash(&[0x12; 32])
        );
    }

    // --- files --------------------------------------------------------------

    const LENGTHS: [usize; 9] = [
        0,
        1,
        CHUNK_LEN - 1,
        CHUNK_LEN,
        CHUNK_LEN + 1,
        2 * CHUNK_LEN,
        3 * CHUNK_LEN - 7,
        3 * CHUNK_LEN,
        3 * CHUNK_LEN + 5,
    ];

    #[test]
    fn a_file_opens_to_what_was_sealed_however_it_arrived() {
        let key = DataKey::generate();
        for len in LENGTHS {
            let plain = sample(len);
            for piece in [1usize, 1000, CHUNK_LEN, CHUNK_LEN + 3, 5 * CHUNK_LEN] {
                if piece == 1 && len > CHUNK_LEN + 1 {
                    continue;
                }
                let file = seal_file(&key, &plain, piece);
                assert_eq!(file.len() as u64, sealed_file_len(len as u64), "len {len}");
                assert_eq!(plain_file_len(file.len() as u64), Some(len as u64));
                assert_eq!(
                    open_range(&key, &file, 0, len as u64).expect("opens"),
                    plain,
                    "len {len}, pieces of {piece}"
                );
            }
        }
    }

    #[test]
    fn any_range_opens_only_the_chunks_it_touches() {
        let key = DataKey::generate();
        let len = 3 * CHUNK_LEN + 5;
        let plain = sample(len);
        let file = seal_file(&key, &plain, 4096);
        let c = CHUNK_LEN as u64;
        let cases = [
            (0, 1),
            (0, len as u64),
            (len as u64 - 1, 1),
            (c - 1, 2),
            (c, 1),
            (c - 1, c + 2),
            (3 * c, 5),
            (3 * c + 4, 1),
            (12345, 70000),
        ];
        let opener =
            FileOpener::new(&key, &file[..FILE_HEADER_LEN], file.len() as u64).expect("opener");
        assert_eq!(opener.plain_len(), len as u64);
        for (start, count) in cases {
            let range = opener.chunks_covering(start, count);
            assert_eq!(*range.start(), start / c, "first chunk for {start}+{count}");
            assert_eq!(
                *range.end(),
                (start + count - 1) / c,
                "last chunk for {start}+{count}"
            );
            let s = start as usize;
            assert_eq!(
                open_range(&key, &file, start, count).expect("opens"),
                &plain[s..s + count as usize],
                "range {start}+{count}"
            );
        }
    }

    #[test]
    fn a_file_cut_short_reordered_or_altered_is_refused() {
        let key = DataKey::generate();
        let plain = sample(3 * CHUNK_LEN + 5);
        let file = seal_file(&key, &plain, CHUNK_LEN);
        let whole = |bytes: &[u8]| {
            open_range(
                &key,
                bytes,
                0,
                plain_file_len(bytes.len() as u64).unwrap_or(0),
            )
        };

        // Cut at a chunk boundary: what is now the last chunk was sealed as
        // a middle one.
        let cut = &file[..FILE_HEADER_LEN + 3 * SEALED_CHUNK_LEN];
        assert_eq!(whole(cut), Err(OpenError::Rejected));
        // Cut to the header alone.
        assert_eq!(whole(&file[..FILE_HEADER_LEN]), Err(OpenError::Truncated));
        // Two middle chunks swapped.
        let mut swapped = file.clone();
        let (a, b) = (FILE_HEADER_LEN, FILE_HEADER_LEN + SEALED_CHUNK_LEN);
        let first = file[a..b].to_vec();
        swapped.copy_within(b..b + SEALED_CHUNK_LEN, a);
        swapped[b..b + SEALED_CHUNK_LEN].copy_from_slice(&first);
        assert_eq!(whole(&swapped), Err(OpenError::Rejected));
        // One byte altered in the middle of the last chunk, and in the header.
        let mut altered = file.clone();
        let at = file.len() - 3;
        altered[at] ^= 0x01;
        assert_eq!(whole(&altered), Err(OpenError::Rejected));
        let mut renamed = file.clone();
        renamed[FILE_MAGIC.len()] ^= 0x01;
        assert_eq!(whole(&renamed), Err(OpenError::Rejected));
        let mut not_ours = file.clone();
        not_ours[0] = b'X';
        assert_eq!(whole(&not_ours), Err(OpenError::UnknownFormat));
        // The wrong key.
        assert_eq!(
            open_range(&DataKey::generate(), &file, 0, 1),
            Err(OpenError::Rejected)
        );
        // An empty file is a sealed empty chunk, not nothing.
        let empty = seal_file(&key, b"", 1);
        assert_eq!(empty.len(), FILE_HEADER_LEN + TAG_LEN);
        assert_eq!(whole(&empty), Ok(Vec::new()));
    }

    #[test]
    fn no_length_is_claimed_for_bytes_no_sealed_file_could_be() {
        assert_eq!(plain_file_len(0), None);
        assert_eq!(plain_file_len(FILE_HEADER_LEN as u64), None);
        assert_eq!(plain_file_len((FILE_HEADER_LEN + TAG_LEN - 1) as u64), None);
        assert_eq!(plain_file_len((FILE_HEADER_LEN + TAG_LEN) as u64), Some(0));
        for len in LENGTHS {
            assert_eq!(
                plain_file_len(sealed_file_len(len as u64)),
                Some(len as u64)
            );
        }
    }
}
