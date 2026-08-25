use serde::{Deserialize, Serialize, Serializer};

pub(crate) fn decode_nonnegative_i64_decimal(raw: &str) -> Option<i64> {
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    raw.parse::<i64>().ok()
}

pub(crate) fn decode_nonnegative_u64_decimal(raw: &str) -> Option<u64> {
    decode_nonnegative_i64_decimal(raw).map(|value| value as u64)
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingCallerAttemptDtoV1 {
    pub kind: String,
    pub caller_request_id: String,
    pub operation_id: Option<String>,
    pub resolution: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingCallerAttemptPageDtoV1 {
    pub entries: Vec<PendingCallerAttemptDtoV1>,
    pub next_cursor: Option<String>,
}

impl From<crate::usecase::application_lifecycle::operation::PendingCallerAttemptPage>
    for PendingCallerAttemptPageDtoV1
{
    fn from(
        value: crate::usecase::application_lifecycle::operation::PendingCallerAttemptPage,
    ) -> Self {
        Self {
            entries: value.entries.into_iter().map(Into::into).collect(),
            next_cursor: value.next_cursor,
        }
    }
}

impl From<crate::usecase::application_lifecycle::operation::PendingCallerAttempt>
    for PendingCallerAttemptDtoV1
{
    fn from(value: crate::usecase::application_lifecycle::operation::PendingCallerAttempt) -> Self {
        Self {
            kind: value.kind.label().to_string(),
            caller_request_id: value.caller_request_id,
            operation_id: value.operation_id,
            resolution: value.resolution.label().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeOperationFailureDtoV1 {
    pub kind: String,
    pub retryable: bool,
    pub label: String,
    pub detail: Option<String>,
    pub correlation_id: String,
}

impl From<crate::domain::local_event::SafeOperationFailure> for SafeOperationFailureDtoV1 {
    fn from(value: crate::domain::local_event::SafeOperationFailure) -> Self {
        Self {
            kind: crate::usecase::application_lifecycle::operation::record::failure_kind_label(
                value.kind,
            )
            .to_string(),
            retryable: value.retryable,
            label: value.label.value().to_string(),
            detail: value.detail.map(|detail| detail.value().to_string()),
            correlation_id: value.correlation_id,
        }
    }
}

#[derive(Serialize)]
struct ApplicationCommandErrorWire<'a> {
    #[serde(rename = "type")]
    error_type: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<&'a SafeOperationFailureDtoV1>,
}

impl<'a> ApplicationCommandErrorWire<'a> {
    fn new(error_type: &'static str, message: &'static str) -> Self {
        Self {
            error_type,
            message,
            correlation_id: None,
            failure: None,
        }
    }

    fn with_correlation_id(mut self, correlation_id: &'a str) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    fn with_failure(mut self, failure: &'a SafeOperationFailureDtoV1) -> Self {
        self.failure = Some(failure);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) enum OperationApplicationErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    ShutdownInProgress,
    Internal { correlation_id: String },
}

impl Serialize for OperationApplicationErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::InvalidRequest => ApplicationCommandErrorWire::new(
                "invalid_request",
                "Releash could not access application operation history because the request is invalid.",
            ),
            Self::PayloadConflict => ApplicationCommandErrorWire::new(
                "payload_conflict",
                "The application operation request conflicts with an earlier request. Refresh and try again.",
            ),
            Self::ShutdownInProgress => ApplicationCommandErrorWire::new(
                "shutdown_in_progress",
                "Releash could not update application operation history while shutdown is in progress. Try again after Releash restarts.",
            ),
            Self::Internal { correlation_id } => ApplicationCommandErrorWire::new(
                "internal",
                "Releash could not access application operation history. Try again.",
            )
            .with_correlation_id(correlation_id),
        };
        wire.serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ApplicationQuitErrorDtoV1 {
    InvalidRequest,
    PayloadConflict,
    CapacityExceeded,
    Internal { correlation_id: String },
}

impl Serialize for ApplicationQuitErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::InvalidRequest => ApplicationCommandErrorWire::new(
                "invalid_request",
                "Releash could not start the application quit because the request is invalid.",
            ),
            Self::PayloadConflict => ApplicationCommandErrorWire::new(
                "payload_conflict",
                "The application quit request conflicts with an earlier request. Refresh and try again.",
            ),
            Self::CapacityExceeded => ApplicationCommandErrorWire::new(
                "capacity_exceeded",
                "Releash could not start another application quit. Wait for the current operation and try again.",
            ),
            Self::Internal { correlation_id } => ApplicationCommandErrorWire::new(
                "internal",
                "Releash could not complete the application quit. Try again.",
            )
            .with_correlation_id(correlation_id),
        };
        wire.serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ApplicationQuitLookupErrorDtoV1 {
    InvalidRequest,
    NotFound,
    QueryBusy,
    DeadlineExceeded,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

impl Serialize for ApplicationQuitLookupErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::InvalidRequest => ApplicationCommandErrorWire::new(
                "invalid_request",
                "Releash could not check the application quit because the request is invalid.",
            ),
            Self::NotFound => ApplicationCommandErrorWire::new(
                "not_found",
                "The application quit operation is no longer available.",
            ),
            Self::QueryBusy => ApplicationCommandErrorWire::new(
                "query_busy",
                "Releash is still checking the application quit. Try again.",
            ),
            Self::DeadlineExceeded => ApplicationCommandErrorWire::new(
                "deadline_exceeded",
                "Checking the application quit took too long. Try again.",
            ),
            Self::StorageUnavailable { failure } => ApplicationCommandErrorWire::new(
                "storage_unavailable",
                "Releash could not access the application quit operation. Try again.",
            )
            .with_failure(failure),
            Self::Internal { correlation_id } => ApplicationCommandErrorWire::new(
                "internal",
                "Releash could not check the application quit. Try again.",
            )
            .with_correlation_id(correlation_id),
        };
        wire.serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CurrentShutdownErrorDtoV1 {
    Internal { correlation_id: String },
}

impl Serialize for CurrentShutdownErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Self::Internal { correlation_id } = self;
        ApplicationCommandErrorWire::new(
            "internal",
            "Releash could not check the current application shutdown. Try again.",
        )
        .with_correlation_id(correlation_id)
        .serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ShutdownPlanQueryErrorDtoV1 {
    InvalidRequest,
    NotFound,
    DetailsCompacted,
    CursorMismatch,
    CursorExpired,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

impl Serialize for ShutdownPlanQueryErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::InvalidRequest => ApplicationCommandErrorWire::new(
                "invalid_request",
                "Releash could not load the shutdown plan because the request is invalid.",
            ),
            Self::NotFound => ApplicationCommandErrorWire::new(
                "not_found",
                "The shutdown plan is no longer available.",
            ),
            Self::DetailsCompacted => ApplicationCommandErrorWire::new(
                "details_compacted",
                "The shutdown plan details are no longer available.",
            ),
            Self::CursorMismatch => ApplicationCommandErrorWire::new(
                "cursor_mismatch",
                "The shutdown plan changed while it was loading. Reload the plan and try again.",
            ),
            Self::CursorExpired => ApplicationCommandErrorWire::new(
                "cursor_expired",
                "The shutdown plan page expired. Reload the plan and try again.",
            ),
            Self::QueryBusy => ApplicationCommandErrorWire::new(
                "query_busy",
                "The shutdown plan is busy. Try again.",
            ),
            Self::DeadlineExceeded => ApplicationCommandErrorWire::new(
                "deadline_exceeded",
                "Loading the shutdown plan took too long. Try again.",
            ),
            Self::ResponseTooLarge => ApplicationCommandErrorWire::new(
                "response_too_large",
                "The shutdown plan is too large to load.",
            ),
            Self::StorageUnavailable { failure } => ApplicationCommandErrorWire::new(
                "storage_unavailable",
                "Releash could not access the shutdown plan. Try again.",
            )
            .with_failure(failure),
            Self::Internal { correlation_id } => ApplicationCommandErrorWire::new(
                "internal",
                "Releash could not load the shutdown plan. Try again.",
            )
            .with_correlation_id(correlation_id),
        };
        wire.serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ShutdownDetailsMutationErrorDtoV1 {
    InvalidRequest,
    Internal { correlation_id: String },
}

impl Serialize for ShutdownDetailsMutationErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::InvalidRequest => ApplicationCommandErrorWire::new(
                "invalid_request",
                "Releash could not compact the shutdown details because the request is invalid.",
            ),
            Self::Internal { correlation_id } => ApplicationCommandErrorWire::new(
                "internal",
                "Releash could not compact the shutdown details. Try again.",
            )
            .with_correlation_id(correlation_id),
        };
        wire.serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RecoveryActionCommandErrorDtoV1 {
    InvalidRequest,
    NotFound,
    StorageUnavailable { failure: SafeOperationFailureDtoV1 },
    Internal { correlation_id: String },
}

impl Serialize for RecoveryActionCommandErrorDtoV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::InvalidRequest => ApplicationCommandErrorWire::new(
                "invalid_request",
                "Releash could not resolve the shutdown target because the request is invalid.",
            ),
            Self::NotFound => ApplicationCommandErrorWire::new(
                "not_found",
                "The shutdown target action is no longer available. Reload the shutdown plan and try again.",
            ),
            Self::StorageUnavailable { failure } => ApplicationCommandErrorWire::new(
                "storage_unavailable",
                "Releash could not save the shutdown target action. Try again.",
            )
            .with_failure(failure),
            Self::Internal { correlation_id } => ApplicationCommandErrorWire::new(
                "internal",
                "Releash could not resolve the shutdown target action. Try again.",
            )
            .with_correlation_id(correlation_id),
        };
        wire.serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryActionKindDtoV1 {
    ReadAgain,
    RetrySameEffect,
    UseObservedResult,
    CancelIfSafe,
    KeepForManualResolution,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryActionIdentityDtoV1 {
    pub action_id: String,
    pub action: RecoveryActionKindDtoV1,
    pub origin_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionOutcomeDtoV1 {
    Completed {
        action_id: String,
        result: RecoveryActionCompletedResultDtoV1,
    },
    InProgress {
        action_id: String,
    },
    Rejected {
        action_id: String,
        rejection: RecoveryActionRejectionDtoV1,
    },
    ActionOutcomeUnknown {
        action_id: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryActionCompletedResultDtoV1 {
    pub outcome: String,
    pub classification: String,
    pub resource_revision: String,
    pub canonical_result_sha256: String,
    pub resource_view: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RecoveryActionRejectionDtoV1 {
    RevisionConflict { current_revision: String },
    ActionUnavailable,
    TargetRevisionChanged,
}

impl From<RecoveryActionKindDtoV1> for crate::domain::local_event::RecoveryActionKind {
    fn from(value: RecoveryActionKindDtoV1) -> Self {
        use crate::domain::local_event::RecoveryActionKind as K;
        match value {
            RecoveryActionKindDtoV1::ReadAgain => K::ReadAgain,
            RecoveryActionKindDtoV1::RetrySameEffect => K::RetrySameEffect,
            RecoveryActionKindDtoV1::UseObservedResult => K::UseObservedResult,
            RecoveryActionKindDtoV1::CancelIfSafe => K::CancelIfSafe,
            RecoveryActionKindDtoV1::KeepForManualResolution => K::KeepForManualResolution,
        }
    }
}

impl From<crate::domain::local_event::RecoveryActionKind> for RecoveryActionKindDtoV1 {
    fn from(value: crate::domain::local_event::RecoveryActionKind) -> Self {
        use crate::domain::local_event::RecoveryActionKind as K;
        match value {
            K::ReadAgain => Self::ReadAgain,
            K::RetrySameEffect => Self::RetrySameEffect,
            K::UseObservedResult => Self::UseObservedResult,
            K::CancelIfSafe => Self::CancelIfSafe,
            K::KeepForManualResolution => Self::KeepForManualResolution,
        }
    }
}

impl From<crate::usecase::application_lifecycle::operation::RecoveryActionOutcome>
    for RecoveryActionOutcomeDtoV1
{
    fn from(
        value: crate::usecase::application_lifecycle::operation::RecoveryActionOutcome,
    ) -> Self {
        use crate::usecase::application_lifecycle::operation::RecoveryActionOutcome as O;
        match value {
            O::Completed { action_id, result } => Self::Completed {
                action_id,
                result: result.into(),
            },
            O::ActionOutcomeUnknown { action_id } => Self::ActionOutcomeUnknown { action_id },
            O::InProgress { action_id } => Self::InProgress { action_id },
            O::Rejected {
                action_id,
                rejection,
            } => Self::Rejected {
                action_id,
                rejection: rejection.into(),
            },
        }
    }
}

impl From<crate::usecase::application_lifecycle::operation::RecoveryActionRejection>
    for RecoveryActionRejectionDtoV1
{
    fn from(
        value: crate::usecase::application_lifecycle::operation::RecoveryActionRejection,
    ) -> Self {
        use crate::usecase::application_lifecycle::operation::RecoveryActionRejection as R;
        match value {
            R::RevisionConflict { current_revision } => Self::RevisionConflict {
                current_revision: current_revision.to_string(),
            },
            R::ActionUnavailable => Self::ActionUnavailable,
            R::TargetRevisionChanged => Self::TargetRevisionChanged,
        }
    }
}

impl From<crate::usecase::application_lifecycle::operation::recovery::RecoveryActionCompletedResult>
    for RecoveryActionCompletedResultDtoV1
{
    fn from(
        value: crate::usecase::application_lifecycle::operation::recovery::RecoveryActionCompletedResult,
    ) -> Self {
        use crate::usecase::application_lifecycle::operation::RecoveryActionResultOutcome as O;
        Self {
            outcome: match value.outcome {
                O::Pending => "pending",
                O::Terminal => "terminal",
                O::Unchanged => "unchanged",
            }
            .to_string(),
            classification: recovery_classification_label(value.classification).to_string(),
            resource_revision: value.resource_revision.to_string(),
            canonical_result_sha256: value.canonical_result_sha256,
            resource_view: value.resource_view,
        }
    }
}

impl RecoveryActionOutcomeDtoV1 {
    pub(crate) fn from_durable_status(
        value: crate::usecase::application_lifecycle::operation::RecoveryActionStatus,
    ) -> Self {
        use crate::usecase::application_lifecycle::operation::RecoveryActionStatus as S;
        match value {
            S::Completed { action_id, result } => Self::Completed {
                action_id,
                result: result.into(),
            },
            S::InProgress { action_id } | S::ReconciliationRequired { action_id, .. } => {
                Self::InProgress { action_id }
            }
            S::OutcomeUnknown { action_id } => Self::ActionOutcomeUnknown { action_id },
        }
    }
}

fn recovery_classification_label(
    value: crate::domain::local_event::RecoveryResultClassification,
) -> &'static str {
    use crate::domain::local_event::RecoveryResultClassification as C;
    match value {
        C::Pending => "pending",
        C::Succeeded => "succeeded",
        C::ConfirmedNoEffect => "confirmed_no_effect",
        C::Ambiguous => "ambiguous",
        C::CancelledBeforeEffect => "cancelled_before_effect",
        C::Unchanged => "unchanged",
    }
}

#[cfg(test)]
#[path = "application_operation_v1_test.rs"]
mod application_operation_v1_tests;
