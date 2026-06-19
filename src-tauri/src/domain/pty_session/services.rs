use std::collections::VecDeque;

use crate::domain::pty_session::PtyKind;

pub const OUTPUT_BUFFER_CAPACITY: usize = 64 * 1024;
pub const MAX_PENDING_BYTES: usize = 16 * 1024;

pub fn parse_pty_kind(kind: Option<&str>) -> PtyKind {
    match kind {
        Some("one_shot") => PtyKind::OneShot,
        _ => PtyKind::Terminal,
    }
}

pub fn decode_utf8_chunk(raw_chunk: &[u8], pending: &mut Vec<u8>) -> Option<String> {
    pending.extend_from_slice(raw_chunk);

    let valid_up_to = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        Err(e) => e.valid_up_to(),
    };

    if valid_up_to == 0 {
        if pending.len() > MAX_PENDING_BYTES {
            pending.clear();
        }
        return None;
    }

    let decoded = std::str::from_utf8(&pending[..valid_up_to])
        .unwrap()
        .to_string();
    *pending = pending[valid_up_to..].to_vec();
    Some(decoded)
}

pub fn append_output_to_ring_buffer(ring: &mut VecDeque<u8>, output: &str, capacity: usize) {
    let bytes = output.as_bytes();
    if bytes.len() >= capacity {
        ring.clear();
        ring.extend(&bytes[bytes.len() - capacity..]);
        return;
    }

    let overflow = (ring.len() + bytes.len()).saturating_sub(capacity);
    if overflow > 0 {
        ring.drain(..overflow);
    }
    ring.extend(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kind_defaults_unknown_values_to_terminal() {
        assert_eq!(parse_pty_kind(Some("one_shot")), PtyKind::OneShot);
        assert_eq!(parse_pty_kind(Some("terminal")), PtyKind::Terminal);
        assert_eq!(parse_pty_kind(Some("unknown")), PtyKind::Terminal);
        assert_eq!(parse_pty_kind(None), PtyKind::Terminal);
    }

    #[test]
    fn decode_utf8_chunk_keeps_incomplete_sequence_pending() {
        let mut pending = Vec::new();

        assert_eq!(decode_utf8_chunk(&[0xE3, 0x81], &mut pending), None);
        assert_eq!(pending.len(), 2);
        assert_eq!(
            decode_utf8_chunk(&[0x82], &mut pending),
            Some("あ".to_string())
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn decode_utf8_chunk_drops_oversized_invalid_pending_bytes() {
        let mut pending = Vec::new();
        let invalid_bytes = vec![0xFF; MAX_PENDING_BYTES + 1];

        assert_eq!(decode_utf8_chunk(&invalid_bytes, &mut pending), None);
        assert!(pending.is_empty());
    }

    #[test]
    fn appends_output_as_fixed_capacity_ring_buffer() {
        let mut ring = VecDeque::new();

        append_output_to_ring_buffer(&mut ring, &"x".repeat(6), 10);
        append_output_to_ring_buffer(&mut ring, &"y".repeat(6), 10);

        let buffered = String::from_utf8(ring.into_iter().collect()).unwrap();
        assert_eq!(buffered, "xxxxyyyyyy");
    }

    #[test]
    fn oversized_output_keeps_tail_only() {
        let mut ring = VecDeque::new();

        append_output_to_ring_buffer(&mut ring, "abcdefghijkl", 5);

        let buffered = String::from_utf8(ring.into_iter().collect()).unwrap();
        assert_eq!(buffered, "hijkl");
    }
}
