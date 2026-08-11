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
            Self::InvalidCharacter => write!(
                f,
                "operation identity contains a character outside [A-Za-z0-9._:-]"
            ),
        }
    }
}

impl std::error::Error for OperationIdentityError {}

pub fn validate_operation_identity(raw: &str) -> Result<(), OperationIdentityError> {
    if raw.is_empty() {
        return Err(OperationIdentityError::Empty);
    }
    if raw.len() > MAX_OPERATION_IDENTITY_BYTES {
        return Err(OperationIdentityError::TooLong {
            max: MAX_OPERATION_IDENTITY_BYTES,
        });
    }
    if raw
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')))
    {
        return Err(OperationIdentityError::InvalidCharacter);
    }
    Ok(())
}

pub fn constant_time_eq_32(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
