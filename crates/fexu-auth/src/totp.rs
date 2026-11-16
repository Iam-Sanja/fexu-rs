//! RFC 6238 TOTP with SHA-1, recovery codes, and short-lived proofs.
//!
//! SHA-1 is used only to HMAC the counter because authenticator apps expect it.
//! The 160-bit secret is encrypted in the database.

use crate::error::AuthError;
use crate::service::constant_eq;
use aes_gcm::aead::{Aead, KeyInit as AeadKeyInit, Payload};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::Sha256;
use uuid::Uuid;

type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const RECOVERY_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Key used to derive legacy TOTP secrets and recovery hashes.
/// This is HMAC-SHA256 over the CSRF secret, not the current MFA key.
pub fn legacy_mfa_key(csrf_secret: &[u8; 32]) -> [u8; 32] {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(csrf_secret).expect("mfa key length");
    mac.update(b"fexu-mfa-v1");
    mac.finalize().into_bytes().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealFormat {
    /// `v2.` plus 24 hexadecimal characters. Decryptable only using `FEXU_MFA_KEY`.
    Current,
    /// Exactly 24 hexadecimal characters. Decryptable only with the key derived from the CSRF secret.
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryFormat {
    /// `v2.` plus 64 hexadecimal characters. HMAC, not ciphertext.
    Current,
    /// Exactly 64 hexadecimal characters. HMAC with the legacy key, not ciphertext.
    Legacy,
}

pub fn seal_format(nonce: &str) -> Result<SealFormat, AuthError> {
    if let Some(rest) = nonce.strip_prefix("v2.") {
        if is_hex(rest, 24) {
            return Ok(SealFormat::Current);
        }
        return Err(AuthError::Validation("mfa secret could not be read".into()));
    }
    if is_hex(nonce, 24) {
        Ok(SealFormat::Legacy)
    } else {
        Err(AuthError::Validation("mfa secret could not be read".into()))
    }
}

pub fn recovery_format(value: &str) -> Option<RecoveryFormat> {
    if let Some(rest) = value.strip_prefix("v2.") {
        return is_hex(rest, 64).then_some(RecoveryFormat::Current);
    }
    is_hex(value, 64).then_some(RecoveryFormat::Legacy)
}

pub fn current_nonce(nonce_hex: &str) -> String {
    format!("v2.{nonce_hex}")
}

pub fn open_stored(
    mfa_key: &[u8; 32],
    legacy_key: &[u8; 32],
    user_id: Uuid,
    nonce: &str,
    ciphertext: &str,
) -> Result<Vec<u8>, AuthError> {
    match seal_format(nonce)? {
        SealFormat::Current => {
            let Some(nonce_hex) = nonce.strip_prefix("v2.") else {
                return Err(AuthError::Validation("mfa secret could not be read".into()));
            };
            open(mfa_key, user_id, nonce_hex, ciphertext)
        }
        SealFormat::Legacy => open(legacy_key, user_id, nonce, ciphertext),
    }
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.chars().all(|char| char.is_ascii_hexdigit())
}

pub fn random_secret() -> Vec<u8> {
    let mut bytes = vec![0u8; 20];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

pub fn seal(
    key: &[u8; 32],
    user_id: Uuid,
    plaintext: &[u8],
) -> Result<(String, String), AuthError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| AuthError::Validation("mfa key is invalid".into()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: user_id.as_bytes(),
            },
        )
        .map_err(|_| AuthError::Validation("mfa secret could not be stored".into()))?;
    Ok((hex(nonce.as_slice()), hex(&ciphertext)))
}

pub fn open(
    key: &[u8; 32],
    user_id: Uuid,
    nonce_hex: &str,
    cipher_hex: &str,
) -> Result<Vec<u8>, AuthError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| AuthError::Validation("mfa key is invalid".into()))?;
    let nonce = decode_hex(nonce_hex)?;
    let ciphertext = decode_hex(cipher_hex)?;
    cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: user_id.as_bytes(),
            },
        )
        .map_err(|_| AuthError::Validation("mfa secret could not be read".into()))
}

pub fn totp_code(secret: &[u8], unix: i64, digits: u32) -> Result<String, AuthError> {
    if !(6..=8).contains(&digits) {
        return Err(AuthError::Validation("totp digits are invalid".into()));
    }
    let counter = unix.div_euclid(30) as u64;
    let mut mac = <HmacSha1 as KeyInit>::new_from_slice(secret)
        .map_err(|_| AuthError::Validation("totp secret is invalid".into()))?;
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    let modulus = 10u32.pow(digits);
    Ok(format!(
        "{:0width$}",
        binary % modulus,
        width = digits as usize
    ))
}

/// Checks the current window and rejects an already consumed time step.
pub fn accept_totp(
    secret: &[u8],
    code: &str,
    unix: i64,
    last_step: Option<i64>,
) -> Result<i64, AuthError> {
    let current = unix.div_euclid(30);
    for step in [current - 1, current, current + 1] {
        if last_step.is_some_and(|used| step <= used) {
            continue;
        }
        let candidate = totp_code(secret, step.saturating_mul(30), 6)?;
        if constant_eq(&candidate, code.trim()) {
            return Ok(step);
        }
    }
    Err(AuthError::InvalidCredentials)
}

pub fn base32(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            output.push(ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        output.push(ALPHABET[index] as char);
    }
    output
}

pub fn otpauth_uri(email: &str, secret: &[u8]) -> String {
    format!(
        "otpauth://totp/Fexu:{}?secret={}&issuer=Fexu&algorithm=SHA1&digits=6&period=30",
        encode_component(email),
        base32(secret)
    )
}

pub fn recovery_codes() -> Vec<String> {
    (0..8).map(|_| recovery_code()).collect()
}

pub fn normalize_recovery(code: &str) -> Result<String, AuthError> {
    let normalized: String = code
        .chars()
        .filter(|char| *char != '-' && !char.is_whitespace())
        .map(|char| char.to_ascii_uppercase())
        .collect();
    if normalized.len() == 10
        && normalized
            .bytes()
            .all(|byte| RECOVERY_ALPHABET.contains(&byte))
    {
        Ok(normalized)
    } else {
        Err(AuthError::InvalidCredentials)
    }
}

pub fn recovery_hash(key: &[u8; 32], user_id: Uuid, normalized: &str) -> String {
    keyed_recovery(key, b"fexu-recovery-v1", user_id, normalized)
}

pub fn current_recovery_record(key: &[u8; 32], user_id: Uuid, normalized: &str) -> String {
    format!(
        "v2.{}",
        keyed_recovery(key, b"fexu-recovery-v2", user_id, normalized)
    )
}

fn keyed_recovery(key: &[u8; 32], label: &[u8], user_id: Uuid, normalized: &str) -> String {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key).expect("recovery key");
    mac.update(label);
    mac.update(user_id.as_bytes());
    mac.update(normalized.as_bytes());
    hex(&mac.finalize().into_bytes())
}

pub fn reauth_proof(key: &[u8; 32], user_id: Uuid, session_id: Uuid, expires_at: i64) -> String {
    format!(
        "v2.{expires_at}.{}",
        reauth_mac(key, b"fexu-reauth-v2", user_id, session_id, expires_at)
    )
}

#[cfg(test)]
pub fn legacy_reauth_proof(
    key: &[u8; 32],
    user_id: Uuid,
    session_id: Uuid,
    expires_at: i64,
) -> String {
    format!(
        "v1.{expires_at}.{}",
        reauth_mac(key, b"fexu-reauth-v1", user_id, session_id, expires_at)
    )
}

pub fn reauth_valid(
    mfa_key: &[u8; 32],
    legacy_key: &[u8; 32],
    user_id: Uuid,
    session_id: Uuid,
    proof: &str,
    now: i64,
) -> bool {
    let mut parts = proof.split('.');
    let version = parts.next().unwrap_or("");
    let expires = parts.next().unwrap_or("");
    let mac = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return false;
    }
    let key = match version {
        "v2" => mfa_key,
        "v1" => legacy_key,
        _ => return false,
    };
    let label: &[u8] = if version == "v2" {
        b"fexu-reauth-v2"
    } else {
        b"fexu-reauth-v1"
    };
    let Ok(expires_at) = expires.parse::<i64>() else {
        return false;
    };
    if expires_at <= now || expires_at > now.saturating_add(300) {
        return false;
    }
    constant_eq(
        &reauth_mac(key, label, user_id, session_id, expires_at),
        mac,
    )
}

fn reauth_mac(
    key: &[u8; 32],
    label: &[u8],
    user_id: Uuid,
    session_id: Uuid,
    expires_at: i64,
) -> String {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key).expect("reauth key");
    mac.update(label);
    mac.update(user_id.as_bytes());
    mac.update(session_id.as_bytes());
    mac.update(expires_at.to_string().as_bytes());
    hex(&mac.finalize().into_bytes())
}

fn recovery_code() -> String {
    let mut chars = String::new();
    while chars.len() < 10 {
        let mut byte = [0u8; 1];
        OsRng.fill_bytes(&mut byte);
        let index = (byte[0] & 31) as usize;
        chars.push(RECOVERY_ALPHABET[index] as char);
    }
    format!("{}-{}", &chars[..5], &chars[5..])
}

fn encode_component(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(value: &str) -> Result<Vec<u8>, AuthError> {
    if !value.len().is_multiple_of(2) || !value.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(AuthError::Validation("mfa secret could not be read".into()));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| AuthError::Validation("mfa secret could not be read".into()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        accept_totp, legacy_mfa_key, legacy_reauth_proof, open, open_stored, recovery_format,
        recovery_hash, seal, seal_format, totp_code, HmacSha1, RecoveryFormat, SealFormat,
    };
    use aes_gcm::aead::KeyInit as AeadKeyInit;
    use aes_gcm::Aes256Gcm;
    use argon2::password_hash::rand_core::OsRng;
    use hmac::{KeyInit, Mac};
    use uuid::Uuid;

    #[test]
    fn rfc6238_sha1_vectors() {
        let secret = b"12345678901234567890";
        let cases = [
            (59_i64, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ];
        for (unix, code) in cases {
            assert_eq!(totp_code(secret, unix, 8).unwrap(), code);
        }
    }

    #[test]
    fn sealed_secret_is_bound_to_the_user_and_codes_do_not_repeat() {
        let mut csrf_material = [0u8; 32];
        getrandom::getrandom(&mut csrf_material).expect("csrf material");
        let mfa: [u8; 32] = Aes256Gcm::generate_key(&mut OsRng).into();
        let legacy = legacy_mfa_key(&csrf_material);
        let user = Uuid::new_v4();
        let secret = b"12345678901234567890";
        let (nonce, ciphertext) = seal(&legacy, user, secret).unwrap();
        assert!(!ciphertext.contains("12345678901234567890"));
        assert_eq!(open(&legacy, user, &nonce, &ciphertext).unwrap(), secret);
        assert!(open(&legacy, Uuid::new_v4(), &nonce, &ciphertext).is_err());
        assert_eq!(seal_format(&nonce).unwrap(), SealFormat::Legacy);
        assert!(seal_format(&super::current_nonce(&nonce)).is_ok());
        let code = totp_code(secret, 1_700_000_000, 6).unwrap();
        let step = accept_totp(secret, &code, 1_700_000_000, None).unwrap();
        assert!(accept_totp(secret, &code, 1_700_000_000, Some(step)).is_err());
        let hash = recovery_hash(&legacy, user, "ABCDEFGHJK");
        assert_eq!(recovery_format(&hash), Some(RecoveryFormat::Legacy));
        assert!(seal_format(&hash).is_err());
        assert_ne!(hash, "ABCDEFGHJK");
        let current = super::current_recovery_record(&mfa, user, "ABCDEFGHJK");
        assert_eq!(recovery_format(&current), Some(RecoveryFormat::Current));
        assert!(seal_format(&current).is_err());
        assert_ne!(current, hash);
        assert!(recovery_format("$argon2id$v=19$m=19456,t=2,p=1$salt$hash").is_none());
        let stored = open_stored(&mfa, &legacy, user, &nonce, &ciphertext).unwrap();
        assert_eq!(stored, secret);
        let proof = super::reauth_proof(&mfa, user, user, 1_300);
        assert!(super::reauth_valid(
            &mfa, &legacy, user, user, &proof, 1_000
        ));
        assert!(!super::reauth_valid(
            &mfa,
            &legacy,
            user,
            Uuid::new_v4(),
            &proof,
            1_000
        ));
        let legacy_proof = legacy_reauth_proof(&legacy, user, user, 1_300);
        assert!(super::reauth_valid(
            &mfa,
            &legacy,
            user,
            user,
            &legacy_proof,
            1_000
        ));
        let other_legacy = legacy_mfa_key(b"rotated-csrf-secret-32-bytes!!!!");
        assert!(!super::reauth_valid(
            &mfa,
            &other_legacy,
            user,
            user,
            &legacy_proof,
            1_000
        ));
        assert!(super::reauth_valid(
            &mfa,
            &other_legacy,
            user,
            user,
            &proof,
            1_000
        ));
        let expired = super::reauth_proof(&mfa, user, user, 1_000);
        assert!(!super::reauth_valid(
            &mfa, &legacy, user, user, &expired, 1_000
        ));
        let too_long = super::reauth_proof(&mfa, user, user, 1_301);
        assert!(!super::reauth_valid(
            &mfa, &legacy, user, user, &too_long, 1_000
        ));
        assert!(!proof.contains("fexu-dev"));
        assert!(!legacy_proof.contains("fexu-dev"));
    }

    fn hmac_sha1_hex(key: &[u8], data: &[u8]) -> String {
        let mut mac = <HmacSha1 as KeyInit>::new_from_slice(key).unwrap();
        mac.update(data);
        super::hex(&mac.finalize().into_bytes())
    }

    #[test]
    fn rfc2202_hmac_sha1_vectors() {
        let key4 = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
        ];
        let repeated_dd = [0xdd; 50];
        let repeated_cd = [0xcd; 50];
        let cases: &[(&str, &[u8], &[u8])] = &[
            (
                "b617318655057264e28bc0b6fb378c8ef146be00",
                &[0x0b; 20],
                b"Hi There",
            ),
            (
                "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79",
                b"Jefe",
                b"what do ya want for nothing?",
            ),
            (
                "125d7342b9ac11cd91a39af48aa17b4f63f175d3",
                &[0xaa; 20],
                &repeated_dd,
            ),
            (
                "4c9007f4026250c6bc8414f9bf50c86c2d7235da",
                &key4,
                &repeated_cd,
            ),
            (
                "4c1a03424b55e07fe7f27be1d58bb9324a9a5a04",
                &[0x0c; 20],
                b"Test With Truncation",
            ),
            (
                "aa4ae5e15272d00e95705637ce8a3b55ed402112",
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First",
            ),
            (
                "e8e99d0f45237d786d6bbaa7965c7808bbff1a91",
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key and Larger Than One Block-Size Data",
            ),
        ];
        for (expected, key, data) in cases {
            assert_eq!(hmac_sha1_hex(key, data), *expected);
        }
    }

    #[test]
    fn rfc6238_sha1_six_digits_keep_leading_zeros() {
        let secret = b"12345678901234567890";
        let cases = [
            (59_i64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ];
        for (unix, code) in cases {
            assert_eq!(totp_code(secret, unix, 6).unwrap(), code);
            assert_eq!(code.len(), 6);
        }
    }

    #[test]
    fn totp_window_accepts_neighbors_and_rejects_replay() {
        let secret = b"12345678901234567890";
        let now = 59_i64;
        assert_eq!(accept_totp(secret, "755224", now, None).unwrap(), 0);
        assert_eq!(accept_totp(secret, "287082", now, None).unwrap(), 1);
        assert_eq!(accept_totp(secret, "359152", now, None).unwrap(), 2);
        assert!(accept_totp(secret, "969429", now, None).is_err());
        assert!(accept_totp(secret, "287082", now, Some(1)).is_err());
        assert!(accept_totp(secret, "755224", now, Some(1)).is_err());
        assert_eq!(accept_totp(secret, "359152", now, Some(1)).unwrap(), 2);
    }

    #[test]
    fn legacy_recovery_and_reauth_hmac_keep_published_bytes() {
        let csrf = [0x11u8; 32];
        let key = [0x42u8; 32];
        let user = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let session = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        assert_eq!(
            super::hex(&legacy_mfa_key(&csrf)),
            "0ccf407db9da1845c8b6f4400c55e64cb007528509cac32a0e2036cb707c4b47"
        );
        assert_eq!(
            recovery_hash(&key, user, "ABCDEFGHJK"),
            "2de6d22df5227b70a7307c654a4aea77b48f73210877ace426ea93e5af2f6ac3"
        );
        assert_eq!(
            super::current_recovery_record(&key, user, "ABCDEFGHJK"),
            "v2.196376076485f82a9f2a3e04a055f88186200e203a3b1017399dd3e28672fe63"
        );
        let current = super::reauth_proof(&key, user, session, 1_300);
        let legacy = legacy_reauth_proof(&key, user, session, 1_300);
        assert_eq!(
            current,
            "v2.1300.c396df286ae5b1112b8bae16102e68896228b9b26d440901927f93f9032bf255"
        );
        assert_eq!(
            legacy,
            "v1.1300.be59e03b3e3165c9aacf011ea8d6e56753fe4aa99c90b1b038e2cc8451f6139b"
        );
        assert!(super::reauth_valid(
            &key, &key, user, session, &current, 1_000
        ));
        assert!(super::reauth_valid(
            &key, &key, user, session, &legacy, 1_000
        ));
        let other = [0x24u8; 32];
        assert!(!super::reauth_valid(
            &other, &key, user, session, &current, 1_000
        ));
        assert!(!super::reauth_valid(
            &key, &other, user, session, &legacy, 1_000
        ));
    }
}
