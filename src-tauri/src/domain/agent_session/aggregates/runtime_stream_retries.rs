use std::collections::VecDeque;

pub trait StreamRetryIdentity {
    fn message_id(&self) -> &str;
    fn sequence(&self) -> u64;
}

/// Process-local aggregate for streaming delivery retries.
///
/// The payload remains an opaque boundary value. This aggregate owns which
/// retry is canonical, replacement by message identity, ordering, and
/// acknowledgement of the queue head.
#[derive(Debug)]
pub struct RuntimeStreamRetries<T> {
    coalesced: Option<T>,
    authoritative: VecDeque<T>,
}

impl<T> Default for RuntimeStreamRetries<T> {
    fn default() -> Self {
        Self {
            coalesced: None,
            authoritative: VecDeque::new(),
        }
    }
}

impl<T: StreamRetryIdentity> RuntimeStreamRetries<T> {
    /// Reset turn-local coalescing while retaining authoritative snapshots
    /// that still have to reach the notifier.
    pub fn reset_regular(&mut self) {
        self.coalesced = None;
    }

    pub fn has_coalesced(&self) -> bool {
        self.coalesced.is_some()
    }

    pub fn take_coalesced(&mut self) -> Option<T> {
        self.coalesced.take()
    }

    pub fn replace_coalesced(&mut self, retry: Option<T>) {
        self.coalesced = retry;
    }

    pub fn clear_coalesced(&mut self) {
        self.coalesced = None;
    }

    pub fn prepare_authoritative(&mut self, message_id: &str) {
        self.coalesced = None;
        self.authoritative
            .retain(|retry| retry.message_id() != message_id);
    }

    pub fn authoritative_is_empty(&self) -> bool {
        self.authoritative.is_empty()
    }

    pub fn upsert_authoritative(&mut self, retry: T) {
        if let Some(existing) = self
            .authoritative
            .iter_mut()
            .find(|existing| existing.message_id() == retry.message_id())
        {
            *existing = retry;
        } else {
            self.authoritative.push_back(retry);
        }
    }

    pub fn authoritative_front(&self) -> Option<&T> {
        self.authoritative.front()
    }

    pub fn acknowledge_authoritative_front(&mut self, message_id: &str, sequence: u64) -> bool {
        let matches = self
            .authoritative
            .front()
            .is_some_and(|retry| retry.message_id() == message_id && retry.sequence() == sequence);
        if matches {
            self.authoritative.pop_front();
        }
        matches
    }

    pub fn clear_authoritative(&mut self) {
        self.authoritative.clear();
    }

    #[cfg(test)]
    pub fn authoritative_len(&self) -> usize {
        self.authoritative.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Retry(&'static str, u64);

    impl StreamRetryIdentity for Retry {
        fn message_id(&self) -> &str {
            self.0
        }

        fn sequence(&self) -> u64 {
            self.1
        }
    }

    #[test]
    fn authoritative_retries_are_replaced_and_acknowledged_by_identity() {
        let mut retries = RuntimeStreamRetries::default();
        retries.replace_coalesced(Some(Retry("coalesced", 1)));
        retries.upsert_authoritative(Retry("a", 1));
        retries.upsert_authoritative(Retry("b", 2));
        retries.upsert_authoritative(Retry("a", 3));
        retries.prepare_authoritative("b");

        assert!(!retries.has_coalesced());
        assert_eq!(retries.authoritative_front(), Some(&Retry("a", 3)));
        assert!(!retries.acknowledge_authoritative_front("a", 2));
        assert!(retries.acknowledge_authoritative_front("a", 3));
        assert!(retries.authoritative_is_empty());
    }

    #[test]
    fn regular_turn_reset_preserves_authoritative_delivery() {
        let mut retries = RuntimeStreamRetries::default();
        retries.replace_coalesced(Some(Retry("regular", 1)));
        retries.upsert_authoritative(Retry("terminal", 2));

        retries.reset_regular();

        assert!(!retries.has_coalesced());
        assert_eq!(retries.authoritative_front(), Some(&Retry("terminal", 2)));
    }
}
