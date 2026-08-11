const QUIT_DOMAIN: &[u8] = b"application-quit-exact-request-binding/v1";

fn push_lp(output: &mut Vec<u8>, value: &[u8]) {
    let length = u32::try_from(value.len()).expect("operation binding field exceeds u32");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
}

fn outer(
    principal: &str,
    installation_id: &str,
    caller_id: &str,
    backend_operation_id: &str,
    inner: &[u8],
) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, QUIT_DOMAIN);
    push_lp(&mut output, principal.as_bytes());
    push_lp(&mut output, installation_id.as_bytes());
    push_lp(&mut output, caller_id.as_bytes());
    push_lp(&mut output, backend_operation_id.as_bytes());
    push_lp(&mut output, inner);
    output
}

pub fn quit_inner(mode: &str, exit_code: i32) -> Vec<u8> {
    let mut output = Vec::new();
    push_lp(&mut output, mode.as_bytes());
    output.extend_from_slice(&exit_code.to_be_bytes());
    output
}

pub fn application_quit(
    principal: &str,
    installation_id: &str,
    request_id: &str,
    backend_operation_id: &str,
    mode: &str,
    exit_code: i32,
) -> Vec<u8> {
    outer(
        principal,
        installation_id,
        request_id,
        backend_operation_id,
        &quit_inner(mode, exit_code),
    )
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn application_quit_binding_keeps_its_canonical_known_answer() {
        let material =
            application_quit("principal_1", "app_1", "quit_req_1", "quit_op_1", "exit", 0);
        assert_eq!(material.len(), 112);

        let key: Vec<u8> = (0..32).collect();
        let mut key_block = [0u8; 64];
        key_block[..key.len()].copy_from_slice(&key);
        let mut inner = Sha256::new();
        inner.update(key_block.map(|byte| byte ^ 0x36));
        inner.update(&material);
        let inner = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(key_block.map(|byte| byte ^ 0x5c));
        outer.update(inner);
        assert_eq!(
            hex::encode(outer.finalize()),
            "6a34bd12ce2691c1912e31d4e0f797cd51e28a67fdf5dc03714f18782e49dfda"
        );
    }
}
