#[cfg(feature = "desktop")]
pub(crate) struct ClientHandoffReference {
    pub id: String,
    pub command: String,
    pub fingerprint: Vec<u8>,
    pub ordering_target: Vec<u8>,
}

#[cfg(feature = "desktop")]
pub(crate) trait ClientHandoffRepository: Send + Sync {
    fn remember(&self, reference: &ClientHandoffReference) -> Result<(), String>;
    fn forget(&self, id: &str) -> Result<(), String>;
}

pub(crate) fn validate_reference(
    id: &str,
    command: &str,
    fingerprint: &[u8],
    ordering_target: &[u8],
) -> Result<(), &'static str> {
    validate_id(id)?;
    if command.is_empty()
        || command.len() > 128
        || !command
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err("Invalid client command.");
    }
    validate_fingerprint(fingerprint, ordering_target)
}

pub(crate) fn validate_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Err("Invalid client operation identity.")
    } else {
        Ok(())
    }
}
pub(crate) fn validate_fingerprint(fingerprint: &[u8], target: &[u8]) -> Result<(), &'static str> {
    if fingerprint.len() == 32 && matches!(target.len(), 0 | 32) {
        Ok(())
    } else {
        Err("Invalid operation fingerprint.")
    }
}

#[cfg(test)]
#[path = "handoff_test.rs"]
mod handoff_tests;

#[cfg(any(feature = "desktop", test))]
pub(crate) fn restored_unknown(command: &str, previous: &str, current: &str) -> bool {
    super::policy::persists_for_restart(command) && !previous.is_empty() && previous != current
}
