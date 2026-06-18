//! Commit context for workflow runtime mutation handlers.
//!
//! Caller route and transport details are passed as metadata; core engine code owns
//! append-only `WorkflowEvent` construction.

use crate::adaptor::gateway::workflow::event::CliMutationRequestRecord;

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

/// CLI pending dispatcher / compatibility dispatch boundary 経由で渡される commit metadata。
///
/// `CliPending` は CliMutationRequested を伴う Approve / Reject / Abort 系の文脈。
/// `SubmitOutput` は OutputSubmitted 単体で記録される [08] structured output 提出系で、
/// CliMutationRequested は emit しない（spec [08] 振る舞い定義 Rule 1 / Rule 4:
/// SubmitOutput 単独で `request_id` / `submitted_at` を伴う OutputSubmitted を記録）。
///
/// 5-3 / 5-4 修正: いずれの variant も engine 拒否時に `CliMutationRejected` event
/// を補助履歴として記録する。SubmitOutput variant は payload 本体を持たないが、
/// `step_name` と `contract` を保持して rejected event の `request` フィールドを
/// 構築できるようにする（payload 全体は容量肥大を避けるため含めない）。
#[derive(Debug, Clone)]
pub(crate) enum CommandCommitContext {
    CliPending {
        mutation: WorkflowMutationContext,
    },
    SubmitOutput {
        request_id: String,
        submitted_at: f64,
        step_name: String,
        contract: String,
    },
}

impl CommandCommitContext {
    pub(crate) fn cli_pending(mutation: WorkflowMutationContext) -> Self {
        Self::CliPending { mutation }
    }

    pub(crate) fn submit_output(
        request_id: String,
        submitted_at: f64,
        step_name: String,
        contract: String,
    ) -> Self {
        Self::SubmitOutput {
            request_id,
            submitted_at,
            step_name,
            contract,
        }
    }

    /// `CliPending` バリアントに含まれる `WorkflowMutationContext` への参照を返す。
    /// `SubmitOutput` バリアントには CliMutationRequested 用の mutation context は無い。
    pub(crate) fn cli_pending_mutation(&self) -> Option<&WorkflowMutationContext> {
        match self {
            Self::CliPending { mutation } => Some(mutation),
            Self::SubmitOutput { .. } => None,
        }
    }

    pub(crate) fn into_cli_pending_mutation(self) -> Option<WorkflowMutationContext> {
        match self {
            Self::CliPending { mutation } => Some(mutation),
            Self::SubmitOutput { .. } => None,
        }
    }

    /// SubmitOutput context から CliMutationRejected event 用の構成要素を取り出す。
    /// `(request_id, requested_at, step_name, contract)` を返す（5-3 修正）。
    pub(crate) fn submit_output_rejection_parts(&self) -> Option<(String, f64, String, String)> {
        match self {
            Self::SubmitOutput {
                request_id,
                submitted_at,
                step_name,
                contract,
            } => Some((
                request_id.clone(),
                *submitted_at,
                step_name.clone(),
                contract.clone(),
            )),
            Self::CliPending { .. } => None,
        }
    }
}
