// ============================================================
//  crypto.rs — zero-knowledge .htbk container (BE-CRYPTO owns)
// ============================================================
//
// PURE client-side crypto: no I/O, no network. Implements:
//   - KDF: argon2id, passphrase -> 256-bit key (params from the cleartext header).
//   - AEAD: XChaCha20-Poly1305 (24-byte nonce), cleartext header authenticated as AAD.
//   - `.htbk` container encode/decode (byte-exact, TSD §4, frozen).
//   - zeroize of passphrase / derived key / plaintext buffers.
//
// Security contract (TSD §4, audited by SECURITY):
//   * unique random nonce per encode (fresh OsRng) — never reused.
//   * derived key lives only in Zeroizing<[u8;32]>.
//   * argon2 params >= OWASP floor (m=64MiB / t=3 / p=1).
//   * AEAD tag verified BEFORE any plaintext is produced (decode returns Err
//     without exposing plaintext on tamper / wrong passphrase).
//   * header tamper (AAD) fails open-verification.
//   * passphrase + derived key never appear in Debug or logs.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

// ---- frozen container constants (TSD §4) --------------------------------

const MAGIC: &[u8; 4] = b"HTBK";
const VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const AEAD_XCHACHA: u8 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
/// Offset of `ct_len`; also the number of header bytes authenticated as AAD.
const HEADER_LEN: usize = 62;

/// OWASP argon2id floor (TSD §4): m = 64 MiB (65536 KiB), t = 3, p = 1.
const DEFAULT_M_COST: u32 = 65536;
const DEFAULT_T_COST: u32 = 3;
const DEFAULT_P_COST: u32 = 1;

/// Cleartext `.htbk` header (byte-exact serialization per TSD §4).
#[derive(Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub kdf_id: u8,
    pub aead_id: u8,
    pub reserved: u8,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub ct_len: u64,
}

// Never print salt/nonce/params inadvertently in a way that leaks secrets —
// there are none here, but keep Debug minimal and explicit.
impl std::fmt::Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Header")
            .field("version", &self.version)
            .field("kdf_id", &self.kdf_id)
            .field("aead_id", &self.aead_id)
            .field("m_cost", &self.m_cost)
            .field("t_cost", &self.t_cost)
            .field("p_cost", &self.p_cost)
            .field("ct_len", &self.ct_len)
            .finish_non_exhaustive()
    }
}

impl Header {
    fn with_defaults(salt: [u8; SALT_LEN], nonce: [u8; NONCE_LEN]) -> Self {
        Header {
            version: VERSION,
            kdf_id: KDF_ARGON2ID,
            aead_id: AEAD_XCHACHA,
            reserved: 0,
            m_cost: DEFAULT_M_COST,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
            salt,
            nonce,
            ct_len: 0,
        }
    }

    /// Serialize the 62-byte cleartext header. `ct_len` is written as the
    /// final u64 (this produces the full 70-byte prefix; the AAD slice is
    /// `[0..HEADER_LEN]`).
    fn to_prefix_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER_LEN + 8);
        b.extend_from_slice(MAGIC); // 0..4
        b.push(self.version); // 4
        b.push(self.kdf_id); // 5
        b.push(self.aead_id); // 6
        b.push(self.reserved); // 7
        b.extend_from_slice(&self.m_cost.to_le_bytes()); // 8..12
        b.extend_from_slice(&self.t_cost.to_le_bytes()); // 12..16
        b.extend_from_slice(&self.p_cost.to_le_bytes()); // 16..20
        b.extend_from_slice(&(SALT_LEN as u16).to_le_bytes()); // 20..22
        b.extend_from_slice(&self.salt); // 22..38
        b.extend_from_slice(&self.nonce); // 38..62
        debug_assert_eq!(b.len(), HEADER_LEN);
        b.extend_from_slice(&self.ct_len.to_le_bytes()); // 62..70
        b
    }

    /// Parse the fixed header prefix (70 bytes) from a `.htbk` blob.
    fn parse(blob: &[u8]) -> Result<Header, CryptoError> {
        if blob.len() < HEADER_LEN + 8 {
            return Err(CryptoError::Malformed("blob shorter than header"));
        }
        if &blob[0..4] != MAGIC {
            return Err(CryptoError::Malformed("bad magic"));
        }
        let version = blob[4];
        if version != VERSION {
            return Err(CryptoError::Malformed("unsupported version"));
        }
        let kdf_id = blob[5];
        if kdf_id != KDF_ARGON2ID {
            return Err(CryptoError::Malformed("unsupported kdf_id"));
        }
        let aead_id = blob[6];
        if aead_id != AEAD_XCHACHA {
            return Err(CryptoError::Malformed("unsupported aead_id"));
        }
        let reserved = blob[7];
        let m_cost = u32::from_le_bytes(blob[8..12].try_into().unwrap());
        let t_cost = u32::from_le_bytes(blob[12..16].try_into().unwrap());
        let p_cost = u32::from_le_bytes(blob[16..20].try_into().unwrap());
        let salt_len = u16::from_le_bytes(blob[20..22].try_into().unwrap());
        if salt_len as usize != SALT_LEN {
            return Err(CryptoError::Malformed("unexpected salt_len"));
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&blob[22..38]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&blob[38..62]);
        let ct_len = u64::from_le_bytes(blob[62..70].try_into().unwrap());
        Ok(Header {
            version,
            kdf_id,
            aead_id,
            reserved,
            m_cost,
            t_cost,
            p_cost,
            salt,
            nonce,
            ct_len,
        })
    }
}

/// Crypto errors. Variants never carry key/passphrase material. `TagFailed`
/// deliberately does NOT distinguish wrong-passphrase from tampered-ciphertext
/// (both surface the same fail-safe message to the caller per TSD §4).
#[derive(Debug, PartialEq, Eq)]
pub enum CryptoError {
    /// Container bytes are structurally invalid (magic/version/length).
    Malformed(&'static str),
    /// AEAD open failed: wrong passphrase, tampered header (AAD), or tampered
    /// ciphertext. No plaintext is produced.
    TagFailed,
    /// KDF failed (invalid argon2 params, etc.).
    Kdf,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::Malformed(w) => write!(f, "malformed .htbk container: {w}"),
            CryptoError::TagFailed => write!(f, "wrong passphrase or corrupt backup"),
            CryptoError::Kdf => write!(f, "key derivation failed"),
        }
    }
}

impl std::error::Error for CryptoError {}

// ---- KDF -----------------------------------------------------------------

/// Derive a 256-bit key from a passphrase + salt using argon2id with the
/// supplied cost parameters. The returned key is wrapped in `Zeroizing` so it
/// is wiped from memory on drop. Deterministic: same (passphrase, salt, params)
/// always yields the same key.
pub fn derive_key(
    passphrase: &[u8],
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<Zeroizing<[u8; KEY_LEN]>, CryptoError> {
    let params =
        Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN)).map_err(|_| CryptoError::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(passphrase, salt, key.as_mut())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(key)
}

// ---- AEAD seal / open (header authenticated as AAD) ----------------------

/// Encrypt a plaintext manifest into a `.htbk` blob. Generates a fresh random
/// salt + nonce (OsRng) every call, derives the key with argon2id (default
/// OWASP params), and seals with XChaCha20-Poly1305. The 62-byte cleartext
/// header is authenticated as AAD. The passphrase and derived key are zeroized
/// on drop. `plaintext` is zeroized before return.
pub fn encode(passphrase: &str, plaintext: Vec<u8>) -> Result<Vec<u8>, CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    encode_with(passphrase, plaintext, salt, nonce)
}

/// Deterministic encode seam (salt + nonce injected) for the golden/round-trip
/// tests. Production callers use [`encode`], which supplies random values.
fn encode_with(
    passphrase: &str,
    plaintext: Vec<u8>,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, CryptoError> {
    // Wrap the plaintext so it is wiped once we are done with it.
    let plaintext = Zeroizing::new(plaintext);

    let mut header = Header::with_defaults(salt, nonce);
    let key = derive_key(
        passphrase.as_bytes(),
        &header.salt,
        header.m_cost,
        header.t_cost,
        header.p_cost,
    )?;

    // AAD = the 62-byte cleartext header (ct_len not yet authenticated; it is
    // reconstructed and checked on decode against the parsed length).
    let aad = {
        let full = header.to_prefix_bytes();
        full[0..HEADER_LEN].to_vec()
    };

    let cipher = XChaCha20Poly1305::new(key.as_slice().into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&header.nonce),
            Payload {
                msg: plaintext.as_slice(),
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::TagFailed)?;

    header.ct_len = ciphertext.len() as u64;

    let mut out = header.to_prefix_bytes(); // 70-byte prefix incl. ct_len
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a `.htbk` blob. Parses the cleartext header, re-derives the key with
/// the stored argon2 params, and verifies the AEAD tag over the ciphertext with
/// the 62-byte header as AAD. On any failure (wrong passphrase, tampered header,
/// tampered ciphertext) returns `Err(TagFailed)` and produces NO plaintext.
pub fn decode(passphrase: &str, blob: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let header = Header::parse(blob)?;

    let ct_start = HEADER_LEN + 8; // after the 70-byte prefix
    let ct_end = ct_start
        .checked_add(header.ct_len as usize)
        .ok_or(CryptoError::Malformed("ct_len overflow"))?;
    if blob.len() != ct_end {
        return Err(CryptoError::Malformed("ct_len mismatch"));
    }
    let ciphertext = &blob[ct_start..ct_end];

    let key = derive_key(
        passphrase.as_bytes(),
        &header.salt,
        header.m_cost,
        header.t_cost,
        header.p_cost,
    )?;

    // AAD = the exact 62-byte header prefix as it appears in the blob. Any
    // tamper there changes the AAD and the tag check fails.
    let aad = &blob[0..HEADER_LEN];

    let cipher = XChaCha20Poly1305::new(key.as_slice().into());
    // Tag verified HERE, before any plaintext is returned.
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&header.nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::TagFailed)?;

    Ok(plaintext)
}

// =========================================================================
//  TESTS (pure — no network, no I/O)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Light argon2 params for fast tests (production uses the 64 MiB floor via
    // `encode`). KDF determinism/correctness are exercised directly here.
    const TM: u32 = 256; // KiB
    const TT: u32 = 1;
    const TP: u32 = 1;

    fn sample_manifest() -> Vec<u8> {
        br#"{"schema":1,"created_at":"2026-11-03T00:00:00Z","host":"box","files":[{"path_role":"ssh_config","rel":"config","mode":384,"content_b64":"SGk="}]}"#.to_vec()
    }

    #[test]
    fn kdf_determinism_same_salt_pass_same_key() {
        let salt = [7u8; SALT_LEN];
        let k1 = derive_key(b"correct horse", &salt, TM, TT, TP).unwrap();
        let k2 = derive_key(b"correct horse", &salt, TM, TT, TP).unwrap();
        assert_eq!(k1.as_slice(), k2.as_slice());
    }

    #[test]
    fn kdf_differs_on_salt_and_passphrase() {
        let k1 = derive_key(b"pass", &[1u8; SALT_LEN], TM, TT, TP).unwrap();
        let k2 = derive_key(b"pass", &[2u8; SALT_LEN], TM, TT, TP).unwrap();
        let k3 = derive_key(b"other", &[1u8; SALT_LEN], TM, TT, TP).unwrap();
        assert_ne!(k1.as_slice(), k2.as_slice());
        assert_ne!(k1.as_slice(), k3.as_slice());
    }

    #[test]
    fn aead_roundtrip() {
        let pt = sample_manifest();
        let blob = encode_with("s3cret", pt.clone(), [1u8; SALT_LEN], [2u8; NONCE_LEN]).unwrap();
        let out = decode("s3cret", &blob).unwrap();
        assert_eq!(out, pt);
    }

    #[test]
    fn encode_random_roundtrip() {
        let pt = sample_manifest();
        let blob = encode("s3cret", pt.clone()).unwrap();
        let out = decode("s3cret", &blob).unwrap();
        assert_eq!(out, pt);
    }

    #[test]
    fn wrong_passphrase_fails_on_tag() {
        let blob =
            encode_with("right", sample_manifest(), [9u8; SALT_LEN], [3u8; NONCE_LEN]).unwrap();
        assert_eq!(decode("wrong", &blob), Err(CryptoError::TagFailed));
    }

    #[test]
    fn tampered_header_fails_aad() {
        let mut blob =
            encode_with("pw", sample_manifest(), [4u8; SALT_LEN], [5u8; NONCE_LEN]).unwrap();
        // Flip a byte inside the authenticated header region (e.g. m_cost).
        blob[8] ^= 0x01;
        assert_eq!(decode("pw", &blob), Err(CryptoError::TagFailed));
    }

    #[test]
    fn tampered_salt_fails() {
        let mut blob =
            encode_with("pw", sample_manifest(), [4u8; SALT_LEN], [5u8; NONCE_LEN]).unwrap();
        blob[22] ^= 0xFF; // first salt byte -> different key AND AAD mismatch
        assert_eq!(decode("pw", &blob), Err(CryptoError::TagFailed));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut blob =
            encode_with("pw", sample_manifest(), [6u8; SALT_LEN], [7u8; NONCE_LEN]).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x80;
        assert_eq!(decode("pw", &blob), Err(CryptoError::TagFailed));
    }

    #[test]
    fn nonce_uniqueness_across_encodes() {
        // Fresh OsRng per encode -> nonces (bytes 38..62) differ across calls.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..32 {
            let blob = encode("pw", sample_manifest()).unwrap();
            let nonce = blob[38..62].to_vec();
            assert!(seen.insert(nonce), "nonce reused across encodes");
        }
    }

    #[test]
    fn salt_uniqueness_across_encodes() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..32 {
            let blob = encode("pw", sample_manifest()).unwrap();
            let salt = blob[22..38].to_vec();
            assert!(seen.insert(salt), "salt reused across encodes");
        }
    }

    #[test]
    fn htbk_byte_layout_golden() {
        // Deterministic salt/nonce -> byte-exact header assertions (TSD §4).
        let salt = [0xABu8; SALT_LEN];
        let nonce = [0xCDu8; NONCE_LEN];
        let blob = encode_with("pw", sample_manifest(), salt, nonce).unwrap();

        // magic
        assert_eq!(&blob[0..4], b"HTBK");
        // version / kdf_id / aead_id / reserved
        assert_eq!(blob[4], 1);
        assert_eq!(blob[5], 1);
        assert_eq!(blob[6], 1);
        assert_eq!(blob[7], 0);
        // m_cost / t_cost / p_cost (LE, defaults)
        assert_eq!(u32::from_le_bytes(blob[8..12].try_into().unwrap()), 65536);
        assert_eq!(u32::from_le_bytes(blob[12..16].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(blob[16..20].try_into().unwrap()), 1);
        // salt_len
        assert_eq!(u16::from_le_bytes(blob[20..22].try_into().unwrap()), 16);
        // salt / nonce verbatim
        assert_eq!(&blob[22..38], &salt);
        assert_eq!(&blob[38..62], &nonce);
        // ct_len == remaining bytes
        let ct_len = u64::from_le_bytes(blob[62..70].try_into().unwrap());
        assert_eq!(ct_len as usize, blob.len() - 70);
        // round-trip still holds for the golden blob
        assert_eq!(decode("pw", &blob).unwrap(), sample_manifest());
    }

    #[test]
    fn malformed_magic_rejected() {
        let mut blob = encode("pw", sample_manifest()).unwrap();
        blob[0] = b'X';
        assert!(matches!(decode("pw", &blob), Err(CryptoError::Malformed(_))));
    }

    #[test]
    fn truncated_blob_rejected() {
        let blob = encode("pw", sample_manifest()).unwrap();
        let short = &blob[0..HEADER_LEN]; // header only, no ct_len/ciphertext
        assert!(matches!(decode("pw", short), Err(CryptoError::Malformed(_))));
    }

    #[test]
    fn ct_len_mismatch_rejected() {
        let mut blob = encode("pw", sample_manifest()).unwrap();
        // Shrink the claimed ct_len; actual bytes no longer match.
        blob[62] = blob[62].wrapping_add(1);
        assert_eq!(decode("pw", &blob), Err(CryptoError::Malformed("ct_len mismatch")));
    }
}
