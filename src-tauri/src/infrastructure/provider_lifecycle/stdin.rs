use std::io::Read;

#[derive(Debug, thiserror::Error)]
pub(crate) enum BoundedReadError {
    #[error("input exceeds the {limit} byte limit")]
    LimitExceeded { limit: usize },
    #[error("input could not be read: {0}")]
    Read(#[source] std::io::Error),
}

pub(crate) fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, BoundedReadError> {
    let mut payload = Vec::with_capacity(limit.min(8 * 1024));
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut payload)
        .map_err(BoundedReadError::Read)?;
    if payload.len() > limit {
        return Err(BoundedReadError::LimitExceeded { limit });
    }
    Ok(payload)
}
