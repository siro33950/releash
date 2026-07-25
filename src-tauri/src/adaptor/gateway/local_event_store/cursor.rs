//! Opaque MAC-protected pagination cursors.
//!
//! A cursor binds snapshot identity, filter, last key, boot generation, and
//! expiry under an HMAC with the store's private key. Tampering or a filter
//! change is `CursorMismatch`; restart or expiry is `CursorExpired`; a
//! partial page is never produced from a bad cursor.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::adaptor::gateway::local_event_store::hmac_sha256::{hmac_sha256, verify_hmac_sha256};
use crate::domain::local_event::LocalEventQueryError;

const CURSOR_VERSION: &str = "1";
const FIELD_SEPARATOR: char = '\u{1f}';

/// Decoded, verified cursor contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorClaims {
    pub snapshot_id: String,
    pub filter_hash: [u8; 32],
    pub last_key: String,
    pub process_instance_id: String,
    pub expires_at_ms: i64,
}

/// Hash a closed filter description into the cursor binding.
pub fn filter_hash(parts: &[&str]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().into()
}

pub fn issue_cursor(
    key: &[u8],
    snapshot_id: &str,
    filter: &[u8; 32],
    last_key: &str,
    process_instance_id: &str,
    expires_at_ms: i64,
) -> String {
    let body = [
        CURSOR_VERSION,
        snapshot_id,
        &hex::encode(filter),
        last_key,
        process_instance_id,
        &expires_at_ms.to_string(),
    ]
    .join(&FIELD_SEPARATOR.to_string());
    let mac = hmac_sha256(key, body.as_bytes());
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(body.as_bytes()),
        URL_SAFE_NO_PAD.encode(mac)
    )
}

/// Verify a cursor token. `expected_filter` must equal the filter the caller
/// is requesting now; `current_boot_id` / `now_ms` decide expiry.
pub fn verify_cursor(
    key: &[u8],
    token: &str,
    expected_filter: &[u8; 32],
    current_boot_id: &str,
    now_ms: i64,
) -> Result<CursorClaims, LocalEventQueryError> {
    let (body_b64, mac_b64) = token
        .split_once('.')
        .ok_or(LocalEventQueryError::CursorMismatch)?;
    let body = URL_SAFE_NO_PAD
        .decode(body_b64)
        .map_err(|_| LocalEventQueryError::CursorMismatch)?;
    let mac_bytes = URL_SAFE_NO_PAD
        .decode(mac_b64)
        .map_err(|_| LocalEventQueryError::CursorMismatch)?;
    let mac: [u8; 32] = mac_bytes
        .try_into()
        .map_err(|_| LocalEventQueryError::CursorMismatch)?;
    if !verify_hmac_sha256(key, &body, &mac) {
        return Err(LocalEventQueryError::CursorMismatch);
    }
    let body = String::from_utf8(body).map_err(|_| LocalEventQueryError::CursorMismatch)?;
    let fields: Vec<&str> = body.split(FIELD_SEPARATOR).collect();
    if fields.len() != 6 || fields[0] != CURSOR_VERSION {
        return Err(LocalEventQueryError::CursorMismatch);
    }
    let filter_bytes = hex::decode(fields[2]).map_err(|_| LocalEventQueryError::CursorMismatch)?;
    let filter: [u8; 32] = filter_bytes
        .try_into()
        .map_err(|_| LocalEventQueryError::CursorMismatch)?;
    if &filter != expected_filter {
        return Err(LocalEventQueryError::CursorMismatch);
    }
    let expires_at_ms: i64 = fields[5]
        .parse()
        .map_err(|_| LocalEventQueryError::CursorMismatch)?;
    if fields[4] != current_boot_id || now_ms > expires_at_ms {
        return Err(LocalEventQueryError::CursorExpired);
    }
    Ok(CursorClaims {
        snapshot_id: fields[1].to_string(),
        filter_hash: filter,
        last_key: fields[3].to_string(),
        process_instance_id: fields[4].to_string(),
        expires_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"test-cursor-key-0123456789abcdef";

    #[test]
    fn round_trip_verifies() {
        let filter = filter_hash(&["pending", "owner"]);
        let token = issue_cursor(KEY, "snap-1", &filter, "key-42", "boot-1", 10_000);
        let claims = verify_cursor(KEY, &token, &filter, "boot-1", 9_999).unwrap();
        assert_eq!(claims.snapshot_id, "snap-1");
        assert_eq!(claims.last_key, "key-42");
    }

    #[test]
    fn tampering_is_cursor_mismatch() {
        let filter = filter_hash(&["pending"]);
        let token = issue_cursor(KEY, "snap-1", &filter, "key-42", "boot-1", 10_000);
        let mut tampered = token.clone();
        let head = tampered.remove(0);
        tampered.insert(0, if head == 'A' { 'B' } else { 'A' });
        assert_eq!(
            verify_cursor(KEY, &tampered, &filter, "boot-1", 0),
            Err(LocalEventQueryError::CursorMismatch)
        );
        assert_eq!(
            verify_cursor(KEY, "not-a-cursor", &filter, "boot-1", 0),
            Err(LocalEventQueryError::CursorMismatch)
        );
    }

    #[test]
    fn different_filter_is_cursor_mismatch() {
        let filter = filter_hash(&["pending", "owner"]);
        let other = filter_hash(&["pending", "closed_session"]);
        let token = issue_cursor(KEY, "snap-1", &filter, "key-42", "boot-1", 10_000);
        assert_eq!(
            verify_cursor(KEY, &token, &other, "boot-1", 0),
            Err(LocalEventQueryError::CursorMismatch)
        );
    }

    #[test]
    fn restart_and_expiry_are_cursor_expired() {
        let filter = filter_hash(&["pending"]);
        let token = issue_cursor(KEY, "snap-1", &filter, "key-42", "boot-1", 10_000);
        assert_eq!(
            verify_cursor(KEY, &token, &filter, "boot-2", 0),
            Err(LocalEventQueryError::CursorExpired)
        );
        assert_eq!(
            verify_cursor(KEY, &token, &filter, "boot-1", 10_001),
            Err(LocalEventQueryError::CursorExpired)
        );
    }
}
