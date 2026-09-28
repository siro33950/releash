#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InvalidateReason {
    pub shutdown: bool,
}

impl InvalidateReason {
    pub fn change() -> Self {
        Self::default()
    }

    pub fn shutdown() -> Self {
        Self { shutdown: true }
    }

    pub fn merge(&mut self, other: Self) {
        self.shutdown |= other.shutdown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_merge_keeps_shutdown() {
        let mut reason = InvalidateReason::change();
        reason.merge(InvalidateReason::shutdown());
        reason.merge(InvalidateReason::change());

        assert!(reason.shutdown);
    }
}
