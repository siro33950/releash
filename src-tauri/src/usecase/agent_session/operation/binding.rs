//! Canonical caller-operation binding codec.
//!
//! This is the single owner of the byte contract shared by send, Stop,
//! application quit, and the Tauri-only session lifecycle command.

const SEND_DOMAIN: &[u8] = b"send-operation-exact-request-binding/v1";
const PERMISSION_RESPONSE_DOMAIN: &[u8] = b"permission-response-operation-exact-request-binding/v1";
const STOP_DOMAIN: &[u8] = b"stop-operation-exact-request-binding/v1";
const QUIT_DOMAIN: &[u8] = b"application-quit-exact-request-binding/v1";
const LIFECYCLE_DOMAIN: &[u8] = b"session-lifecycle-exact-request-binding/v1";

fn push_lp(output: &mut Vec<u8>, value: &[u8]) {
    let length = u32::try_from(value.len()).expect("operation binding field exceeds u32");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
}

fn outer(
    domain: &[u8],
    principal: &str,
    generation_id: &str,
    caller_id: &str,
    backend_operation_id: Option<&str>,
    inner: &[u8],
) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, domain);
    push_lp(&mut output, principal.as_bytes());
    push_lp(&mut output, generation_id.as_bytes());
    push_lp(&mut output, caller_id.as_bytes());
    if let Some(operation_id) = backend_operation_id {
        push_lp(&mut output, operation_id.as_bytes());
    }
    push_lp(&mut output, inner);
    output
}

pub fn send(
    principal: &str,
    generation_id: &str,
    operation_id: &str,
    canonical_command: &[u8],
) -> Vec<u8> {
    outer(
        SEND_DOMAIN,
        principal,
        generation_id,
        operation_id,
        None,
        canonical_command,
    )
}

pub fn permission_response(
    principal: &str,
    generation_id: &str,
    operation_id: &str,
    canonical_command: &[u8],
) -> Vec<u8> {
    outer(
        PERMISSION_RESPONSE_DOMAIN,
        principal,
        generation_id,
        operation_id,
        None,
        canonical_command,
    )
}

pub fn stop_inner(session_id: &str, turn_id: &str, expected_revision: u64) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, session_id.as_bytes());
    push_lp(&mut output, turn_id.as_bytes());
    output.extend_from_slice(&expected_revision.to_be_bytes());
    output
}

pub fn stop(
    principal: &str,
    generation_id: &str,
    request_id: &str,
    backend_operation_id: &str,
    session_id: &str,
    turn_id: &str,
    expected_revision: u64,
) -> Vec<u8> {
    outer(
        STOP_DOMAIN,
        principal,
        generation_id,
        request_id,
        Some(backend_operation_id),
        &stop_inner(session_id, turn_id, expected_revision),
    )
}

pub fn quit_inner(mode: &str, exit_code: i32) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, mode.as_bytes());
    output.extend_from_slice(&exit_code.to_be_bytes());
    output
}

pub fn application_quit(
    principal: &str,
    generation_id: &str,
    request_id: &str,
    backend_operation_id: &str,
    mode: &str,
    exit_code: i32,
) -> Vec<u8> {
    outer(
        QUIT_DOMAIN,
        principal,
        generation_id,
        request_id,
        Some(backend_operation_id),
        &quit_inner(mode, exit_code),
    )
}

pub fn session_lifecycle_inner(
    session_id: &str,
    expected_revision: u64,
    action: &str,
    backend_id: Option<&str>,
) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, session_id.as_bytes());
    output.extend_from_slice(&expected_revision.to_be_bytes());
    push_lp(&mut output, action.as_bytes());
    match backend_id {
        None => push_lp(&mut output, b"none"),
        Some(backend_id) => {
            push_lp(&mut output, b"some");
            push_lp(&mut output, backend_id.as_bytes());
        }
    }
    output
}

pub struct SessionLifecycleBinding<'a> {
    pub principal: &'a str,
    pub generation_id: &'a str,
    pub request_id: &'a str,
    pub backend_operation_id: &'a str,
    pub session_id: &'a str,
    pub expected_revision: u64,
    pub action: &'a str,
    pub backend_id: Option<&'a str>,
}

pub fn session_lifecycle(binding: SessionLifecycleBinding<'_>) -> Vec<u8> {
    outer(
        LIFECYCLE_DOMAIN,
        binding.principal,
        binding.generation_id,
        binding.request_id,
        Some(binding.backend_operation_id),
        &session_lifecycle_inner(
            binding.session_id,
            binding.expected_revision,
            binding.action,
            binding.backend_id,
        ),
    )
}

pub fn principal(principal: &str) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, b"agent-operation-principal/v1");
    push_lp(&mut output, principal.as_bytes());
    output
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
        let mut key_block = [0u8; 64];
        key_block[..key.len()].copy_from_slice(key);
        let mut inner = Sha256::new();
        inner.update(key_block.map(|byte| byte ^ 0x36));
        inner.update(message);
        let inner = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(key_block.map(|byte| byte ^ 0x5c));
        outer.update(inner);
        outer.finalize().into()
    }

    fn kat(material: Vec<u8>, expected: Vec<u8>, length: usize, digest: &str) {
        let key: Vec<u8> = (0..32).collect();
        assert_eq!(material, expected, "canonical preimage changed");
        assert_eq!(material.len(), length);
        assert_eq!(hex::encode(hmac_sha256(&key, &material)), digest);
    }

    fn lp(value: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::from((value.len() as u32).to_be_bytes());
        encoded.extend_from_slice(value);
        encoded
    }

    fn exact(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.iter().flat_map(|part| part.iter().copied()).collect()
    }

    #[test]
    fn fixed_operation_binding_known_answer_tests() {
        let send_preimage = exact(&[
            lp(SEND_DOMAIN),
            lp(b"principal_1"),
            lp(b"app_1"),
            lp(b"op_1"),
            lp(&[1, 2, 3, 4]),
        ]);
        kat(
            send("principal_1", "app_1", "op_1", &[1, 2, 3, 4]),
            send_preimage,
            83,
            "74ad9247b5f271fc4e31f4fddf7c45cf35d413b1b35202d532095b163f9545db",
        );
        let stop_inner = exact(&[
            lp(b"session_1"),
            lp(b"turn_1"),
            Vec::from(1_u64.to_be_bytes()),
        ]);
        let stop_preimage = exact(&[
            lp(STOP_DOMAIN),
            lp(b"principal_1"),
            lp(b"app_1"),
            lp(b"stop_req_1"),
            lp(b"stop_op_1"),
            lp(&stop_inner),
        ]);
        kat(
            stop(
                "principal_1",
                "app_1",
                "stop_req_1",
                "stop_op_1",
                "session_1",
                "turn_1",
                1,
            ),
            stop_preimage,
            129,
            "9aea744029168a755e77bf7fa763f84df36b2167f7b1bc7fc727e75a26590d3c",
        );
        let quit_inner = exact(&[lp(b"exit"), Vec::from(0_i32.to_be_bytes())]);
        let quit_preimage = exact(&[
            lp(QUIT_DOMAIN),
            lp(b"principal_1"),
            lp(b"app_1"),
            lp(b"quit_req_1"),
            lp(b"quit_op_1"),
            lp(&quit_inner),
        ]);
        kat(
            application_quit("principal_1", "app_1", "quit_req_1", "quit_op_1", "exit", 0),
            quit_preimage,
            112,
            "6a34bd12ce2691c1912e31d4e0f797cd51e28a67fdf5dc03714f18782e49dfda",
        );
        let inner = session_lifecycle_inner("session_1", 1, "close", None);
        let expected_inner = exact(&[
            lp(b"session_1"),
            Vec::from(1_u64.to_be_bytes()),
            lp(b"close"),
            lp(b"none"),
        ]);
        assert_eq!(inner, expected_inner);
        assert_eq!(inner.len(), 38);
        let lifecycle_preimage = exact(&[
            lp(LIFECYCLE_DOMAIN),
            lp(b"principal_1"),
            lp(b"app_1"),
            lp(b"lifecycle_req_1"),
            lp(b"lifecycle_op_1"),
            lp(&inner),
        ]);
        kat(
            session_lifecycle(SessionLifecycleBinding {
                principal: "principal_1",
                generation_id: "app_1",
                request_id: "lifecycle_req_1",
                backend_operation_id: "lifecycle_op_1",
                session_id: "session_1",
                expected_revision: 1,
                action: "close",
                backend_id: None,
            }),
            lifecycle_preimage,
            149,
            "b623c791f1a3f40579ba9713507ab507bdc844dee12d95e4408d673b17eb2217",
        );
    }
}
