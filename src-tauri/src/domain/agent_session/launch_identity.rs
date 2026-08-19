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
