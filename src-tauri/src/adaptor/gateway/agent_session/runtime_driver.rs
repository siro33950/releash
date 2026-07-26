use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::event_log::AgentTurnFailureSignal;
use crate::usecase::agent_session::runtime::ports::{
    AgentTaskSpawner, WorkflowStallNotifier, WorkflowTurnCompleteNotifier,
};
use crate::usecase::agent_session::session::{
    PendingWorkflowTurnCompletion, PendingWorkflowTurnCompletionPage, SessionStore,
};
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification, WorkflowTurnCompleteRecoveryCommand, WorkflowTurnTokenUsage,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub(crate) struct TokioAgentTaskSpawner;

impl AgentTaskSpawner for TokioAgentTaskSpawner {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        tokio::spawn(future);
    }
}

pub(crate) struct WorkflowRuntimeAgentSessionNotifier {
    workflow_runtime: Arc<WorkflowRuntimeUsecase>,
    session_store: Arc<SessionStore>,
}

impl WorkflowRuntimeAgentSessionNotifier {
    const RECOVERY_PAGE_LIMIT: usize = 64;
    const MAX_STARTUP_RECOVERIES: usize = 4_096;

    pub(crate) fn new(
        workflow_runtime: Arc<WorkflowRuntimeUsecase>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        Self {
            workflow_runtime,
            session_store,
        }
    }

    fn recovery_command(
        entry: &PendingWorkflowTurnCompletion,
    ) -> WorkflowTurnCompleteRecoveryCommand {
        let context = &entry.workflow_context;
        WorkflowTurnCompleteRecoveryCommand {
            notification: WorkflowTurnCompleteNotification {
                chat_session_id: entry.session_id.clone(),
                exit_code: entry.input.exit_code,
                final_text_parts: entry.input.final_text_parts.clone(),
                failure_signal: entry.input.failure_signal.map(|signal| match signal {
                    AgentTurnFailureSignal::ModelRefusal => {
                        crate::usecase::workflow::ports::WorkflowTurnFailureSignal::ModelRefusal
                    }
                }),
                token_usage: entry.input.token_usage.map(|usage| WorkflowTurnTokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                }),
                interrupted: entry.input.interrupted,
            },
            turn_id: entry.input.turn_id,
            execution_id: context.execution_id.clone(),
            node_execution_id: context.node_execution_id.clone(),
            workflow_name: context.workflow_name.clone(),
            node_name: context.node_name.clone(),
            attempt: context.attempt,
            parent_node_name: context.parent_node_name.clone(),
            parent_attempt: context.parent_attempt,
            order: context.order,
        }
    }

    async fn pending_page(
        &self,
        owner: Option<String>,
        limit: usize,
        cursor: Option<crate::domain::local_event::QueryCursor>,
    ) -> Result<PendingWorkflowTurnCompletionPage, String> {
        let session_store = self.session_store.clone();
        tokio::task::spawn_blocking(move || {
            session_store.pending_workflow_turn_completion_page(
                owner.as_deref(),
                None,
                limit,
                cursor,
            )
        })
        .await
        .map_err(|_| "workflow turn-completion read worker panicked".to_string())?
    }

    async fn consume(&self, entry: PendingWorkflowTurnCompletion) -> Result<(), String> {
        let session_store = self.session_store.clone();
        tokio::task::spawn_blocking(move || session_store.complete_workflow_turn_completion(&entry))
            .await
            .map_err(|_| "workflow turn-completion consume worker panicked".to_string())?
    }

    /// Applies one exact durable handoff using canonical workflow events and
    /// removes its pending membership only after the workflow commit is
    /// observable. This path never invokes the provider-facing live notifier.
    async fn recover_and_consume(
        &self,
        entry: PendingWorkflowTurnCompletion,
    ) -> Result<(), String> {
        self.workflow_runtime
            .recover_turn_complete(Self::recovery_command(&entry))
            .await
            .map_err(|error| error.to_string())?;
        self.consume(entry).await
    }

    /// Replays only the dedicated workflow-turn-completion pending namespace.
    /// The global cap deliberately fails closed before orphan recovery instead
    /// of turning startup into an unbounded inventory scan.
    pub(crate) async fn recover_pending_turn_completions(&self) -> Result<usize, String> {
        let mut cursor = None;
        let mut recovered = 0usize;
        loop {
            let page = self
                .pending_page(None, Self::RECOVERY_PAGE_LIMIT, cursor)
                .await?;
            if recovered.saturating_add(page.entries.len()) > Self::MAX_STARTUP_RECOVERIES {
                return Err(format!(
                    "workflow turn-completion startup recovery exceeds the {} entry bound",
                    Self::MAX_STARTUP_RECOVERIES
                ));
            }
            for entry in page.entries {
                self.recover_and_consume(entry).await?;
                recovered = recovered.saturating_add(1);
            }
            match page.next_cursor {
                Some(next_cursor) if recovered < Self::MAX_STARTUP_RECOVERIES => {
                    cursor = Some(next_cursor);
                }
                Some(_) => {
                    return Err(format!(
                        "workflow turn-completion startup recovery exceeds the {} entry bound",
                        Self::MAX_STARTUP_RECOVERIES
                    ));
                }
                None => return Ok(recovered),
            }
        }
    }
}

#[async_trait::async_trait]
impl WorkflowTurnCompleteNotifier for WorkflowRuntimeAgentSessionNotifier {
    async fn turn_completed(&self, notification: WorkflowTurnCompleteNotification) {
        let page = match self
            .pending_page(Some(notification.chat_session_id.clone()), 2, None)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                log::warn!(
                    "workflow turn-complete pending handoff read failed; retaining it: {error}"
                );
                return;
            }
        };
        if page.entries.is_empty() {
            // Ordinary chat sessions intentionally have no workflow outbox.
            return;
        }
        if page.entries.len() != 1 || page.next_cursor.is_some() {
            log::warn!(
                "workflow turn-complete has multiple pending handoffs for session {}; retaining them",
                notification.chat_session_id
            );
            return;
        }
        let entry = page
            .entries
            .into_iter()
            .next()
            .expect("one workflow turn-completion entry was checked");
        let command = Self::recovery_command(&entry);
        if command.notification != notification {
            log::warn!(
                "workflow turn-complete notification does not match its durable handoff for session {}; retaining it",
                notification.chat_session_id
            );
            return;
        }

        // The live path owns normal provider activation. Recovery below is a
        // canonical commit readback: if the live transition committed, it
        // returns AlreadyApplied; if runtime state vanished, it safely applies
        // the transition with provider effects suppressed.
        if self
            .workflow_runtime
            .is_session_running(&notification.chat_session_id)
            .await
        {
            if let Err(error) = self
                .workflow_runtime
                .complete_turn(notification.clone())
                .await
            {
                log::warn!(
                    "live workflow turn-complete failed; checking canonical commit before retaining: {error}"
                );
            }
        }
        if let Err(error) = self.recover_and_consume(entry).await {
            log::warn!(
                "workflow turn-complete durable handoff failed; retaining it for startup replay: {error}"
            );
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStallNotifier for WorkflowRuntimeAgentSessionNotifier {
    async fn stall_observed(&self, notification: WorkflowStallObservedNotification) {
        if let Err(error) = self.workflow_runtime.observe_stall(notification).await {
            log::warn!("workflow stall-observed notification failed: {error}");
        }
    }

    async fn stall_cleared(
        &self,
        notification: WorkflowStallClearedNotification,
    ) -> Result<(), WorkflowError> {
        self.workflow_runtime.clear_stall(notification).await
    }
}
