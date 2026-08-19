use sha2::{Digest, Sha256};

pub(crate) fn launch_resource_id(namespace: &str, caller_request_id: &str) -> Option<String> {
    if namespace.trim().is_empty() || caller_request_id.trim().is_empty() {
        return None;
    }
    let digest = Sha256::digest(
        [
            b"agent-session-launch/v1\0".as_slice(),
            namespace.as_bytes(),
            b"\0",
            caller_request_id.as_bytes(),
        ]
        .concat(),
    );
    Some(format!("{namespace}-{}", hex::encode(&digest[..16])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_resource_id_is_deterministic_and_request_scoped() {
        let first = launch_resource_id("session", "request-1").unwrap();
        assert_eq!(launch_resource_id("session", "request-1"), Some(first));
        assert_ne!(
            launch_resource_id("session", "request-1"),
            launch_resource_id("session", "request-2")
        );
    }

    #[test]
    fn launch_resource_id_rejects_blank_identity_parts() {
        assert_eq!(launch_resource_id("   ", "request-1"), None);
        assert_eq!(launch_resource_id("session", "   "), None);
    }
}
