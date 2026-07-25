//! Safe, bounded failure vocabulary shared by local-event-store operations.
//!
//! `SafeOperationFailure` never carries filesystem paths, secrets, raw SQL,
//! provider payloads, or unbounded raw errors; adapters must sanitize before
//! constructing one.

#![allow(dead_code)] // Closed persisted failure vocabulary includes compatibility accessors.

use std::fmt;

pub const NOTICE_LABEL_MAX_BYTES: usize = 160;
pub const NOTICE_DETAIL_MAX_BYTES: usize = 2048;

/// Closed failure kinds from the issues-1499 design "Public closed types".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOperationFailureKind {
    StorageUnavailable,
    StorageCorrupt,
    PersistFailure,
    ProtocolIncompatible,
    ProviderUnavailable,
    ExternalEffectFailed,
    OutcomeUnknown,
    DeadlineExceeded,
    CapacityExceeded,
    StopCapacityExceeded,
    ShutdownAuthorityMismatch,
    TargetRevisionChanged,
    OwnerRevisionChanged,
    RuntimeGenerationChanged,
    InvalidEffectIntent,
    PreviousShutdownReconciliationRequired,
    Internal,
}

/// Content-safe evidence about an external effect. This is deliberately
/// separate from both `SessionOperationFailureKind` and resource state: an
/// observation narrows what is known about an effect without claiming that
/// the effect failed, succeeded, or never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeEffectObservation {
    ProviderObservation {
        observation_ref: String,
        proof_sha256: [u8; 32],
    },
    ConfirmedNoEffect {
        proof_sha256: [u8; 32],
    },
    ExitCoupledOutcomeUnknown {
        shutdown_id: String,
    },
}

/// UTF-8 text truncated to a byte bound; keeps a digest of the original when
/// truncation happened so operators can correlate without leaking content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedNoticeText {
    value: String,
    truncated: bool,
    original_sha256: Option<[u8; 32]>,
}

impl BoundedNoticeText {
    fn bounded(raw: &str, max_bytes: usize, original_sha256: Option<[u8; 32]>) -> Self {
        if raw.len() <= max_bytes {
            return Self {
                value: raw.to_string(),
                truncated: false,
                original_sha256: None,
            };
        }
        const TRUNCATION_MARKER: &str = "…";
        let mut cut = max_bytes.saturating_sub(TRUNCATION_MARKER.len());
        while cut > 0 && !raw.is_char_boundary(cut) {
            cut -= 1;
        }
        Self {
            value: format!("{}{}", &raw[..cut], TRUNCATION_MARKER),
            truncated: true,
            original_sha256,
        }
    }

    /// Bounded label text (160 bytes).
    pub fn label(raw: &str) -> Self {
        Self::bounded(raw, NOTICE_LABEL_MAX_BYTES, None)
    }

    /// Bounded detail text (2048 bytes).
    pub fn detail(raw: &str) -> Self {
        Self::bounded(raw, NOTICE_DETAIL_MAX_BYTES, None)
    }

    /// Bounded detail with a digest of the untruncated original.
    pub fn detail_with_digest(raw: &str, original_sha256: [u8; 32]) -> Self {
        Self::bounded(raw, NOTICE_DETAIL_MAX_BYTES, Some(original_sha256))
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn original_sha256(&self) -> Option<&[u8; 32]> {
        self.original_sha256.as_ref()
    }
}

/// Bounded, content-safe operation failure surfaced to callers and telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeOperationFailure {
    pub kind: SessionOperationFailureKind,
    pub retryable: bool,
    pub label: Box<BoundedNoticeText>,
    pub detail: Option<Box<BoundedNoticeText>>,
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
            detail: None,
            correlation_id: correlation_id.into(),
        }
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(Box::new(BoundedNoticeText::detail(detail)));
        self
    }

    /// Private repository marker used when the SQLite writer observes that
    /// a current shutdown closed mutation admission. Public adapters map this
    /// marker to their endpoint-specific `ShutdownInProgress` variant rather
    /// than exposing it as storage unavailability.
    pub fn is_shutdown_in_progress(&self) -> bool {
        self.kind == SessionOperationFailureKind::PreviousShutdownReconciliationRequired
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
        assert!(text.truncated());
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
        let text = BoundedNoticeText::detail("ok");
        assert!(!text.truncated());
        assert_eq!(text.value(), "ok");
        assert!(text.original_sha256().is_none());
    }

    #[test]
    fn detail_truncates_to_2048_bytes_on_a_utf8_boundary() {
        let raw = "詳".repeat(1_000); // 3,000 bytes
        let text = BoundedNoticeText::detail(&raw);
        assert!(text.truncated());
        assert!(text.value().len() <= NOTICE_DETAIL_MAX_BYTES);
        assert!(text.value().ends_with('…'));
        assert!(text
            .value()
            .trim_end_matches('…')
            .chars()
            .all(|character| character == '詳'));
    }
}
