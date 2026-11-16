//! HMAC-signed download links. The caller supplies the key; it is not embedded in the token.

use crate::StorageError;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;

type MacSha256 = Hmac<Sha256>;

pub fn sign_download(
    key: &[u8],
    owner: Uuid,
    id: Uuid,
    expires_at: i64,
) -> Result<String, StorageError> {
    if key.len() < 32 || expires_at <= 0 {
        return Err(StorageError::Failed);
    }
    let body = format!("v1.{owner}.{id}.{expires_at}");
    let mac = mac(key, body.as_bytes())?;
    Ok(format!("{body}.{mac}"))
}

pub fn open_download(key: &[u8], token: &str, now: i64) -> Result<(Uuid, Uuid), StorageError> {
    if key.len() < 32 {
        return Err(StorageError::Failed);
    }
    let mut parts = token.split('.');
    let version = parts.next().unwrap_or("");
    let owner = parts.next().unwrap_or("");
    let id = parts.next().unwrap_or("");
    let expires = parts.next().unwrap_or("");
    let presented = parts.next().unwrap_or("");
    if version != "v1" || parts.next().is_some() {
        return Err(StorageError::NotFound);
    }
    let expires_at: i64 = expires.parse().map_err(|_| StorageError::NotFound)?;
    let owner_id = Uuid::parse_str(owner).map_err(|_| StorageError::NotFound)?;
    let file_id = Uuid::parse_str(id).map_err(|_| StorageError::NotFound)?;
    let body = format!("v1.{owner}.{id}.{expires}");
    let expected = mac(key, body.as_bytes())?;
    if !constant_eq(presented.as_bytes(), expected.as_bytes()) {
        return Err(StorageError::NotFound);
    }
    if expires_at <= now {
        return Err(StorageError::Expired);
    }
    Ok((owner_id, file_id))
}

fn mac(key: &[u8], body: &[u8]) -> Result<String, StorageError> {
    let mut mac = <MacSha256 as KeyInit>::new_from_slice(key).map_err(|_| StorageError::Failed)?;
    mac.update(body);
    Ok(hex(&mac.finalize().into_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::{open_download, sign_download};
    use uuid::Uuid;

    #[test]
    fn signed_download_rejects_tampering_and_hides_the_key() {
        let key = b"0123456789abcdef0123456789abcdef";
        let owner = Uuid::new_v4();
        let id = Uuid::new_v4();
        let token = sign_download(key, owner, id, 1_700).expect("sign");
        assert!(!token.contains("0123456789abcdef"));
        assert_eq!(
            open_download(key, &token, 1_699).expect("open"),
            (owner, id)
        );
        assert_eq!(
            open_download(key, &token, 1_700),
            Err(crate::StorageError::Expired)
        );
        let mut forged = token.clone().into_bytes();
        let last = forged.len() - 1;
        forged[last] = if forged[last] == b'0' { b'1' } else { b'0' };
        let forged = String::from_utf8(forged).expect("hex");
        assert_eq!(
            open_download(key, &forged, 1_699),
            Err(crate::StorageError::NotFound)
        );
        assert_eq!(
            open_download(key, &forged, 1_700),
            Err(crate::StorageError::NotFound)
        );
        let other = b"abcdef0123456789abcdef0123456789";
        assert!(open_download(other, &token, 1_699).is_err());
        assert!(sign_download(b"short", owner, id, 1_700).is_err());
    }

    fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
        use hmac::Mac;
        let mut mac = <super::MacSha256 as hmac::KeyInit>::new_from_slice(key).unwrap();
        mac.update(data);
        super::hex(&mac.finalize().into_bytes())
    }

    #[test]
    fn rfc4231_hmac_sha256_vectors() {
        let key4 = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
        ];
        let repeated_dd = [0xdd; 50];
        let repeated_cd = [0xcd; 50];
        let cases: &[(&str, &[u8], &[u8])] = &[
            (
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
                &[0x0b; 20],
                b"Hi There",
            ),
            (
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
                b"Jefe",
                b"what do ya want for nothing?",
            ),
            (
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
                &[0xaa; 20],
                &repeated_dd,
            ),
            (
                "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
                &key4,
                &repeated_cd,
            ),
            (
                "a3b6167473100ee06e0c796c2955552bfa6f7c0a6a8aef8b93f860aab0cd20c5",
                &[0x0c; 20],
                b"Test With Truncation",
            ),
            (
                "6953025ed96f0c09f80a96f78e6538dbe2e7b820e3dd970e7ddd39091b32352f",
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First",
            ),
            (
                "6355ac22e890d0a3c8481a5ca4825bc884d3e7a1ff98a2fc2ac7d8e064c3b2e6",
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key and Larger Than One Block-Size Data",
            ),
        ];
        for (expected, key, data) in cases {
            assert_eq!(hmac_sha256_hex(key, data), *expected);
        }
    }

    #[test]
    fn signed_download_matches_the_fixed_hmac_sha256_token() {
        let key = b"0123456789abcdef0123456789abcdef";
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let token = sign_download(key, owner, id, 1_700).unwrap();
        assert_eq!(
            token,
            "v1.00000000-0000-0000-0000-000000000001.00000000-0000-0000-0000-000000000002.1700.5841c164e10a3e65d2f93b2fbdcfa4c052f531900e18b827f92b81d09b47239c"
        );
        assert_eq!(token.len(), "v1.".len() + 36 + 1 + 36 + 1 + 4 + 1 + 64);
        assert_eq!(open_download(key, &token, 1_699).unwrap(), (owner, id));
        assert_eq!(
            open_download(key, &token, 1_700),
            Err(crate::StorageError::Expired)
        );
        let mut forged = token.clone();
        let replacement = if token.ends_with('0') { "1" } else { "0" };
        forged.replace_range(token.len() - 1.., replacement);
        assert_eq!(
            open_download(key, &forged, 1_699),
            Err(crate::StorageError::NotFound)
        );
        assert!(open_download(b"abcdef0123456789abcdef0123456789", &token, 1_699).is_err());
    }
}
