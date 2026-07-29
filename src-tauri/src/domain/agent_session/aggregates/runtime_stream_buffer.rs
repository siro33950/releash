use crate::domain::agent_session::entities::{merge_part, MessagePart, PermissionResponse};
use crate::domain::agent_session::services::{
    add_streaming_byte_size, parts_can_stream_as_append_delta, patch_permission_response,
    streaming_parts_byte_size,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamApplyPlan {
    pub candidate_parts: Vec<MessagePart>,
    pub requires_snapshot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingStreamFacts {
    pub has_pending: bool,
    pub part_count: usize,
    pub byte_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFlushBatch {
    pub snapshot: bool,
    pub parts: Vec<MessagePart>,
}

/// Process-local aggregate for one turn's canonical and pending stream state.
///
/// Persistence and notifier I/O stay in the usecase. This aggregate owns
/// snapshot/delta selection and every transition of the pending buffer.
#[derive(Debug, Default)]
pub struct RuntimeStreamBuffer {
    canonical_parts: Vec<MessagePart>,
    persisted_parts: Vec<MessagePart>,
    pending_parts: Vec<MessagePart>,
    pending_bytes: usize,
    snapshot_pending: bool,
}

impl RuntimeStreamBuffer {
    pub fn reset(&mut self) {
        self.canonical_parts.clear();
        self.persisted_parts.clear();
        self.pending_parts.clear();
        self.pending_bytes = 0;
        self.snapshot_pending = false;
    }

    pub fn canonical_parts(&self) -> &[MessagePart] {
        &self.canonical_parts
    }

    pub fn persisted_parts(&self) -> &[MessagePart] {
        &self.persisted_parts
    }

    pub fn prepare_apply(
        &self,
        delta: &[MessagePart],
        immediate: bool,
        current_sequence: u64,
    ) -> StreamApplyPlan {
        let mut candidate_parts = self.canonical_parts.clone();
        for part in delta {
            merge_part(&mut candidate_parts, part.clone());
        }
        StreamApplyPlan {
            candidate_parts,
            requires_snapshot: immediate
                || current_sequence == 0
                || self.persisted_parts.is_empty()
                || !parts_can_stream_as_append_delta(delta),
        }
    }

    pub fn commit_persisted(
        &mut self,
        candidate_parts: Vec<MessagePart>,
        persisted_parts: Vec<MessagePart>,
        delta: &[MessagePart],
        requires_snapshot: bool,
    ) {
        self.canonical_parts = candidate_parts;
        self.persisted_parts = persisted_parts;
        if requires_snapshot {
            self.fallback_to_snapshot();
        } else if !self.snapshot_pending {
            self.pending_bytes =
                add_streaming_byte_size(self.pending_bytes, streaming_parts_byte_size(delta));
            self.pending_parts.extend_from_slice(delta);
        }
    }

    pub fn pending_facts(&self) -> PendingStreamFacts {
        if self.snapshot_pending {
            PendingStreamFacts {
                has_pending: true,
                part_count: self.persisted_parts.len(),
                byte_size: streaming_parts_byte_size(&self.canonical_parts),
            }
        } else {
            PendingStreamFacts {
                has_pending: !self.pending_parts.is_empty(),
                part_count: self.pending_parts.len(),
                byte_size: self.pending_bytes,
            }
        }
    }

    pub fn take_flush(&mut self, current_sequence: u64) -> Option<StreamFlushBatch> {
        if !self.snapshot_pending && self.pending_parts.is_empty() {
            return None;
        }
        let snapshot = self.snapshot_pending || current_sequence == 0;
        let parts = if snapshot {
            self.persisted_parts.clone()
        } else {
            std::mem::take(&mut self.pending_parts)
        };
        self.pending_bytes = 0;
        self.snapshot_pending = false;
        Some(StreamFlushBatch { snapshot, parts })
    }

    pub fn quarantine_after_persist_failure(&mut self) {
        self.snapshot_pending = true;
        self.pending_parts.clear();
        self.pending_bytes = 0;
    }

    pub fn stop_delivery(&mut self) {
        self.snapshot_pending = false;
        self.pending_parts.clear();
        self.pending_bytes = 0;
    }

    pub fn fallback_to_snapshot(&mut self) {
        self.snapshot_pending = true;
        self.pending_parts.clear();
        self.pending_bytes = 0;
    }

    pub fn patch_permission_response(&mut self, response: &PermissionResponse) -> bool {
        if !patch_permission_response(&mut self.canonical_parts, response) {
            return false;
        }
        self.persisted_parts = self.canonical_parts.clone();
        true
    }

    #[cfg(test)]
    pub fn restore_for_test(
        &mut self,
        canonical_parts: Vec<MessagePart>,
        persisted_parts: Vec<MessagePart>,
        snapshot_pending: bool,
    ) {
        self.canonical_parts = canonical_parts;
        self.persisted_parts = persisted_parts;
        self.pending_parts.clear();
        self.pending_bytes = 0;
        self.snapshot_pending = snapshot_pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: &str) -> MessagePart {
        MessagePart::Text {
            content: content.to_string(),
            parent_tool_use_id: None,
        }
    }

    #[test]
    fn snapshot_and_delta_buffer_transitions_are_closed_in_the_aggregate() {
        let mut buffer = RuntimeStreamBuffer::default();
        let first = buffer.prepare_apply(&[text("first")], false, 0);
        assert!(first.requires_snapshot);
        buffer.commit_persisted(
            first.candidate_parts.clone(),
            first.candidate_parts,
            &[text("first")],
            first.requires_snapshot,
        );
        assert_eq!(
            buffer.take_flush(0),
            Some(StreamFlushBatch {
                snapshot: true,
                parts: vec![text("first")],
            })
        );

        let second = buffer.prepare_apply(&[text(" second")], false, 1);
        assert!(!second.requires_snapshot);
        buffer.commit_persisted(
            second.candidate_parts.clone(),
            second.candidate_parts,
            &[text(" second")],
            second.requires_snapshot,
        );
        assert_eq!(
            buffer.take_flush(1),
            Some(StreamFlushBatch {
                snapshot: false,
                parts: vec![text(" second")],
            })
        );
    }

    #[test]
    fn persistence_failure_quarantines_the_next_delivery_as_a_snapshot() {
        let mut buffer = RuntimeStreamBuffer::default();
        let plan = buffer.prepare_apply(&[text("value")], false, 0);
        buffer.commit_persisted(
            plan.candidate_parts.clone(),
            plan.candidate_parts,
            &[text("value")],
            plan.requires_snapshot,
        );
        buffer.quarantine_after_persist_failure();
        assert_eq!(
            buffer.take_flush(1),
            Some(StreamFlushBatch {
                snapshot: true,
                parts: vec![text("value")],
            })
        );
    }
}
