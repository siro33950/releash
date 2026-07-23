//! Caller-supplied operation / request identity validation (R-001 / R-014).
//!
//! Identities are 1..=128 bytes of `[A-Za-z0-9._:-]`. Invalid identities are
//! `InvalidRequest` with zero state changes and zero external effects.

use std::fmt;

pub const MAX_OPERATION_IDENTITY_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationIdentityError {
    Empty,
    TooLong { max: usize },
    InvalidCharacter,
}

impl fmt::Display for OperationIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "operation identity must not be empty"),
            Self::TooLong { max } => write!(f, "operation identity exceeds {max} bytes"),
            Self::InvalidCharacter => {
                write!(
                    f,
                    "operation identity contains a character outside [A-Za-z0-9._:-]"
                )
            }
        }
    }
}

impl std::error::Error for OperationIdentityError {}

/// Validate a caller operation / request identity.
pub fn validate_operation_identity(raw: &str) -> Result<(), OperationIdentityError> {
    if raw.is_empty() {
        return Err(OperationIdentityError::Empty);
    }
    if raw.len() > MAX_OPERATION_IDENTITY_BYTES {
        return Err(OperationIdentityError::TooLong {
            max: MAX_OPERATION_IDENTITY_BYTES,
        });
    }
    for byte in raw.bytes() {
        let allowed = byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-');
        if !allowed {
            return Err(OperationIdentityError::InvalidCharacter);
        }
    }
    Ok(())
}

/// Constant-time equality over fixed 32-byte MACs (owner-only key material
/// never leaves the binding authority; comparisons must not leak timing).
pub fn constant_time_eq_32(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut diff: u8 = 0;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}
