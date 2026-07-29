#[derive(Debug, Default)]
pub struct RuntimeStreamSequence {
    current: u64,
}

impl RuntimeStreamSequence {
    pub fn current(&self) -> u64 {
        self.current
    }

    pub fn next(&self) -> u64 {
        self.current.saturating_add(1)
    }

    pub fn advance(&mut self) -> u64 {
        self.current = self.next();
        self.current
    }

    pub fn observe_emitted(&mut self, sequence: u64) {
        self.current = self.current.max(sequence);
    }

    pub fn reset(&mut self) {
        self.current = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_sequence_is_monotonic_and_saturates() {
        let mut sequence = RuntimeStreamSequence::default();
        assert_eq!(sequence.next(), 1);
        assert_eq!(sequence.advance(), 1);
        sequence.observe_emitted(7);
        sequence.observe_emitted(3);
        assert_eq!(sequence.current(), 7);
        sequence.observe_emitted(u64::MAX);
        assert_eq!(sequence.advance(), u64::MAX);
        sequence.reset();
        assert_eq!(sequence.current(), 0);
    }
}
