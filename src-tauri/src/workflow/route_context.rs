//! Commit context for workflow command handlers.
//!
//! `WorkflowCommand` stays limited to route-independent state mutation vocabulary.
//! Caller route and transport details are passed as metadata; core engine code owns
//! append-only `WorkflowEvent` construction.

use crate::workflow::event::CliMutationRequestRecord;

/// CLI / 将来の外部経路から渡される mutation の付帯情報。
///
/// 各 caller route がフィールドを直接読み書きせず、accessor 経由で参照する
/// ことで「公開フィールドと同義 accessor の二重提供」状態を解消する
/// （review R2-03: カプセル化）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkflowMutationContext {
    run_id: String,
    source: WorkflowMutationSource,
    request: CliMutationRequestRecord,
    requested_at: f64,
}

impl WorkflowMutationContext {
    pub(crate) fn new(
        run_id: String,
        source: WorkflowMutationSource,
        request: CliMutationRequestRecord,
        requested_at: f64,
    ) -> Self {
        Self {
            run_id,
            source,
            request,
            requested_at,
        }
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn request_id(&self) -> &str {
        match &self.source {
            WorkflowMutationSource::CliPendingCommand { request_id } => request_id,
        }
    }

    /// `WorkflowEvent::CliMutationRequested` 構築時に所有を `run_id` /
    /// `request` ごと engine 側へ移譲するための分解関数。
    pub(crate) fn into_event_parts(self) -> (String, CliMutationRequestRecord, f64, String) {
        let request_id = match &self.source {
            WorkflowMutationSource::CliPendingCommand { request_id } => request_id.clone(),
        };
        (self.run_id, self.request, self.requested_at, request_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkflowMutationSource {
    CliPendingCommand { request_id: String },
}

#[derive(Debug, Clone)]
pub(crate) struct CommandCommitContext {
    mutation: WorkflowMutationContext,
}

impl CommandCommitContext {
    pub(crate) fn cli_pending(mutation: WorkflowMutationContext) -> Self {
        Self { mutation }
    }

    pub(crate) fn mutation(&self) -> &WorkflowMutationContext {
        &self.mutation
    }

    pub(crate) fn into_mutation(self) -> WorkflowMutationContext {
        self.mutation
    }
}
