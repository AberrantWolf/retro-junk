//! Catalog identifiers that come from the bytes, not from the name.
//!
//! A title is a label. It gets corrected — an XML entity that leaked into a
//! DAT, a transliteration someone improved, a typo — and every time it does,
//! anything keyed on it points at nothing. This module mints keys that do not
//! move when the label does.
//!
//! There are two kinds of thing here, and they are keyed differently on
//! purpose:
//!
//! - A **medium** (one cartridge, one disc) *is* its bytes, so its id is
//!   folded from the digests of those bytes. Two catalog entries with the same
//!   complete ordered track set are the same medium and get the same id — so
//!   they collide on a `PRIMARY KEY` instead of quietly becoming twins.
//! - A **work** and a **release** are groupings with no content of their own.
//!   Folding a release from its media would change its id the day the DAT adds
//!   a second disc, and folding a work would change it the day the DAT adds a
//!   region — which is the exact failure this module exists to remove. So
//!   those get a random id minted once and found again by their natural key.
//!
//! Nothing here touches a database or the filesystem, so the importer, a
//! verifier, and a test can all reach the same answer independently.

use sha2::{Digest, Sha256};

/// The kind prefix a rendered id carries.
///
/// The prefix names the kind and never the title, so the whole printed string
/// is stable: safe to paste into a mod configuration, a bug report, or a
/// script, and still meaningful a year later.
pub const MEDIA_PREFIX: &str = "med_";
pub const RELEASE_PREFIX: &str = "rel_";
pub const WORK_PREFIX: &str = "wrk_";
pub const COMPANY_PREFIX: &str = "com_";

/// Crockford base32: no `I`, `L`, `O` or `U`, so a rendered id cannot be
/// misread aloud or mistyped into a different valid id.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// How much of the SHA-256 becomes the id: 80 bits, which is 16 base32
/// characters exactly.
///
/// At a million catalog entries the chance of any two folding to the same
/// value is about 4 in 10^13. These are `PRIMARY KEY` columns, so if it ever
/// did happen the insert would fail loudly rather than merging two games.
const ID_BYTES: usize = 10;

/// The SHA-1 of zero bytes. Every empty file has it, so it identifies nothing
/// and must never become an id.
const EMPTY_SHA1: &str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";

/// One hashed piece of a medium: a track of a disc, or the whole file for
/// something the catalog stores as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentPart {
    pub size: u64,
    pub sha1: String,
}

impl ContentPart {
    #[must_use]
    pub fn new(size: u64, sha1: impl Into<String>) -> Self {
        Self {
            size,
            sha1: sha1.into(),
        }
    }
}

/// Why a set of digests cannot name anything.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentIdError {
    #[error("no content to identify: the digest list is empty")]
    NoParts,
    #[error("part {index} has no SHA-1, so it cannot be identified by content")]
    MissingDigest { index: usize },
    #[error("part {index} is empty, and every empty file has the same digest")]
    EmptyContent { index: usize },
}

/// The id for a medium, folded from its complete ordered track set.
///
/// Order matters: track 2 and track 3 swapped is a different disc, and the
/// caller passes them in the order the catalog lists them. A medium the
/// catalog stores as a single file passes one part — its own size and SHA-1 —
/// which is the same thing said with a shorter list.
///
/// Fails rather than guessing when a part is missing its digest or is empty,
/// because both cases would hand out an id that names more than one thing.
pub fn media_id(parts: &[ContentPart]) -> Result<String, ContentIdError> {
    if parts.is_empty() {
        return Err(ContentIdError::NoParts);
    }
    for (index, part) in parts.iter().enumerate() {
        let sha1 = part.sha1.trim();
        if sha1.is_empty() {
            return Err(ContentIdError::MissingDigest { index });
        }
        if part.size == 0 || sha1.eq_ignore_ascii_case(EMPTY_SHA1) {
            return Err(ContentIdError::EmptyContent { index });
        }
    }
    let encoded = parts
        .iter()
        .map(|part| format!("{}:{}", part.sha1.trim().to_ascii_lowercase(), part.size))
        .collect::<Vec<_>>();
    Ok(fold(MEDIA_PREFIX, "rj-media-v1", &encoded))
}

/// The id for a medium stored as one file.
pub fn media_id_from_file(size: u64, sha1: &str) -> Result<String, ContentIdError> {
    media_id(&[ContentPart::new(size, sha1)])
}

/// A fresh work id, minted once and then found again by canonical name and
/// platform. See the module comment for why a work is not folded from content.
#[must_use]
pub fn new_work_id() -> String {
    minted(WORK_PREFIX, "rj-work-v1")
}

/// A fresh release id, minted once and then found again by its natural key
/// (work, platform, region, revision, variant).
#[must_use]
pub fn new_release_id() -> String {
    minted(RELEASE_PREFIX, "rj-release-v1")
}

/// A fresh company id, minted once and then found again by name or alias.
///
/// A publisher's name is free text from an enrichment source, and slugging it
/// to make a key folded every company with a non-ASCII name into one row.
#[must_use]
pub fn new_company_id() -> String {
    minted(COMPANY_PREFIX, "rj-company-v1")
}

/// Whether a string looks like an id this module minted.
///
/// Only for display code deciding whether a value is worth showing raw. It is
/// deliberately not a validity check: nothing should route on it.
#[must_use]
pub fn is_content_id(value: &str) -> bool {
    let Some(body) = [MEDIA_PREFIX, RELEASE_PREFIX, WORK_PREFIX, COMPANY_PREFIX]
        .into_iter()
        .find_map(|prefix| value.strip_prefix(prefix))
    else {
        return false;
    };
    body.len() == encoded_len()
        && body
            .bytes()
            .all(|byte| ALPHABET.contains(&byte.to_ascii_uppercase()))
}

/// A unique seed, folded so every kind of id renders identically.
///
/// The seed is a version-7 UUID: unique because it combines the current
/// millisecond with 74 random bits, so two ids minted in the same process, the
/// same millisecond, or on two machines are still different. Folding it
/// through the same function as a content id means a work id and a media id
/// are the same shape and carry the same collision odds — the caller never has
/// to remember which kind is which length.
fn minted(prefix: &str, domain: &str) -> String {
    let seed = uuid::Uuid::now_v7();
    fold(prefix, domain, &[seed.to_string()])
}

/// SHA-256 over a canonical encoding, truncated and rendered.
///
/// `domain` keeps the kinds apart: a release holding exactly one medium must
/// not fold to the same value as that medium, or the two would fight over one
/// primary key. `0x00` separates the parts — it cannot occur in a hex digest,
/// a decimal size, or a rendered id, so no arrangement of parts can be read as
/// a different arrangement.
fn fold(prefix: &str, domain: &str, parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0u8]);
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    format!("{prefix}{}", base32(&digest[..ID_BYTES]))
}

/// How many base32 characters `ID_BYTES` becomes.
const fn encoded_len() -> usize {
    ID_BYTES * 8 / 5
}

/// Crockford base32, most significant bit first. `ID_BYTES` is a multiple of
/// 5 bytes, so the bits divide evenly and there is no padding to describe.
fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 8 / 5);
    let mut buffer: u16 = 0;
    let mut bits: u32 = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(char::from(ALPHABET[index]));
        }
    }
    out
}
