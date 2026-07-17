use std::future::{ready, Future};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::execution_store::{
    ActiveInterruptionReservation, ExecutionStore, WorkflowExecutionMetadata,
};
use crate::adaptor::gateway::workflow::runtime_commit;

const ACTIVATION_CANCEL_PENDING: u8 = 0;
const ACTIVATION_CANCEL_COMMIT: u8 = 1;
const ACTIVATION_CANCEL_ROLLBACK: u8 = 2;

pub(super) struct RuntimeActivationGate {
    pub(super) lock: Mutex<()>,
    cancel_requested: AtomicBool,
    cancel_notify: Notify,
    cancel_acknowledged: AtomicBool,
    cancel_ack_notify: Notify,
    cancel_decision: AtomicU8,
    cancel_decision_notify: Notify,
}

pub(super) enum InterruptionRollback {
    ProjectionUnchanged,
    RestoreActiveSnapshot(Option<WorkflowExecutionMetadata>),
}

pub(super) async fn rollback_active_interruption(
    execution_store: &Arc<ExecutionStore>,
    reservation: ActiveInterruptionReservation,
    activation_gate: &RuntimeActivationGate,
    activation_was_paused: bool,
    projection: InterruptionRollback,
) -> Result<(), WorkflowEngineError> {
    let reservation_error = execution_store
        .finish_active_interruption(reservation)
        .await
        .err()
        .map(|error| format!("interruption reservation rollback failed: {error}"));
    let projection_error = match projection {
        InterruptionRollback::ProjectionUnchanged => None,
        InterruptionRollback::RestoreActiveSnapshot(snapshot) => {
            runtime_commit::restore_execution_store_active_snapshot(execution_store, snapshot)
                .await
                .err()
                .map(|error| format!("active metadata rollback failed: {error}"))
        }
    };
    if activation_was_paused {
        activation_gate.rollback_cancel();
    } else {
        activation_gate.reset_cancel();
    }
    let errors = [reservation_error, projection_error]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkflowEngineError::SessionStore(errors.join("; ")))
    }
}

impl RuntimeActivationGate {
    pub(super) fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            cancel_requested: AtomicBool::new(false),
            cancel_notify: Notify::new(),
            cancel_acknowledged: AtomicBool::new(false),
            cancel_ack_notify: Notify::new(),
            cancel_decision: AtomicU8::new(ACTIVATION_CANCEL_PENDING),
            cancel_decision_notify: Notify::new(),
        }
    }

    pub(super) fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Release);
        self.cancel_notify.notify_one();
    }

    async fn cancelled(&self) {
        loop {
            if self.cancel_requested.load(Ordering::Acquire) {
                return;
            }
            let notified = self.cancel_notify.notified();
            if self.cancel_requested.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    fn acknowledge_cancel(&self) {
        self.cancel_acknowledged.store(true, Ordering::Release);
        self.cancel_ack_notify.notify_one();
    }

    pub(super) async fn cancellation_acknowledged(&self) {
        loop {
            if self.cancel_acknowledged.load(Ordering::Acquire) {
                return;
            }
            let notified = self.cancel_ack_notify.notified();
            if self.cancel_acknowledged.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub(super) fn commit_cancel(&self) {
        self.cancel_decision
            .store(ACTIVATION_CANCEL_COMMIT, Ordering::Release);
        self.cancel_decision_notify.notify_one();
    }

    pub(super) fn rollback_cancel(&self) {
        self.cancel_decision
            .store(ACTIVATION_CANCEL_ROLLBACK, Ordering::Release);
        self.cancel_decision_notify.notify_one();
    }

    async fn cancel_decision(&self) -> u8 {
        loop {
            let decision = self.cancel_decision.load(Ordering::Acquire);
            if decision != ACTIVATION_CANCEL_PENDING {
                return decision;
            }
            let notified = self.cancel_decision_notify.notified();
            let decision = self.cancel_decision.load(Ordering::Acquire);
            if decision != ACTIVATION_CANCEL_PENDING {
                return decision;
            }
            notified.await;
        }
    }

    pub(super) fn reset_cancel(&self) {
        self.cancel_requested.store(false, Ordering::Release);
        self.cancel_acknowledged.store(false, Ordering::Release);
        self.cancel_decision
            .store(ACTIVATION_CANCEL_PENDING, Ordering::Release);
    }
}

pub(super) async fn run_runtime_activation<F, T>(
    gate: &RuntimeActivationGate,
    execution_id: &str,
    activation_kind: &str,
    future: F,
) -> Result<T, WorkflowEngineError>
where
    F: Future<Output = Result<T, WorkflowEngineError>>,
{
    run_runtime_activation_with_cancel_cleanup(
        gate,
        execution_id,
        activation_kind,
        future,
        ready(()),
    )
    .await
}

pub(super) async fn run_runtime_activation_with_cancel_cleanup<F, C, T>(
    gate: &RuntimeActivationGate,
    execution_id: &str,
    activation_kind: &str,
    future: F,
    cancel_cleanup: C,
) -> Result<T, WorkflowEngineError>
where
    F: Future<Output = Result<T, WorkflowEngineError>>,
    C: Future<Output = ()>,
{
    let mut future = Box::pin(future);
    loop {
        tokio::select! {
            biased;
            _ = gate.cancelled() => {
                gate.acknowledge_cancel();
                let error = match gate.cancel_decision().await {
                    ACTIVATION_CANCEL_COMMIT => WorkflowEngineError::InvalidState(format!(
                            "execution {execution_id} {activation_kind} activation was cancelled"
                        )),
                    ACTIVATION_CANCEL_ROLLBACK => {
                        gate.reset_cancel();
                        continue;
                    }
                    decision => WorkflowEngineError::InvalidState(format!(
                            "execution {execution_id} {activation_kind} activation received invalid cancellation decision {decision}"
                        )),
                };
                drop(future);
                cancel_cleanup.await;
                return Err(error);
            }
            result = &mut future => return result,
        }
    }
}
