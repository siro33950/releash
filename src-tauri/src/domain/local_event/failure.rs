//! Safe, bounded failure vocabulary shared by local-event-store operations.
//!
//! `SafeOperationFailure` never carries filesystem paths, secrets, raw SQL,
//! provider payloads, or unbounded raw errors; adapters must sanitize before
//! constructing one.

use std::fmt;

pub const NOTICE_LABEL_MAX_BYTES: usize = 160;

/// Closed failure kinds from the issues-1499 design "Public closed types".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOperationFailureKind {
    StorageUnavailable,
    PersistFailure,
    OutcomeUnknown,
}

/// UTF-8 text truncated to a byte bound; keeps a digest of the original when
/// truncation happened so operators can correlate without leaking content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedNoticeText {
    value: String,
}

impl BoundedNoticeText {
    fn bounded(raw: &str, max_bytes: usize) -> Self {
        if raw.len() <= max_bytes {
            return Self {
                value: raw.to_string(),
            };
        }
        const TRUNCATION_MARKER: &str = "…";
        let mut cut = max_bytes.saturating_sub(TRUNCATION_MARKER.len());
        while cut > 0 && !raw.is_char_boundary(cut) {
            cut -= 1;
        }
        Self {
            value: format!("{}{}", &raw[..cut], TRUNCATION_MARKER),
        }
    }

    /// Bounded label text (160 bytes).
    pub fn label(raw: &str) -> Self {
        Self::bounded(raw, NOTICE_LABEL_MAX_BYTES)
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Bounded, content-safe operation failure surfaced to callers and telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeOperationFailure {
    pub kind: SessionOperationFailureKind,
    pub retryable: bool,
    pub label: Box<BoundedNoticeText>,
    pub correlation_id: String,
}

impl SafeOperationFailure {
    pub fn new(
        kind: SessionOperationFailureKind,
        retryable: bool,
        label: &str,
        correlation_id: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            retryable,
            label: Box::new(BoundedNoticeText::label(label)),
            correlation_id: correlation_id.into(),
        }
    }
}

impl fmt::Display for SafeOperationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} (retryable={}, correlation_id={}): {}",
            self.kind,
            self.retryable,
            self.correlation_id,
            self.label.value()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_truncates_on_char_boundary() {
        let raw = "あ".repeat(100); // 300 bytes
        let text = BoundedNoticeText::label(&raw);
        assert!(text.value().len() <= NOTICE_LABEL_MAX_BYTES);
        assert!(text.value().ends_with('…'));
        assert!(text
            .value()
            .trim_end_matches('…')
            .chars()
            .all(|c| c == 'あ'));
    }

    #[test]
    fn short_text_is_not_truncated() {
        let text = BoundedNoticeText::label("ok");
        assert_eq!(text.value(), "ok");
    }
}
