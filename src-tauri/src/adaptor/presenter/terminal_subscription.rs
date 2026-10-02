use crate::adaptor::presenter::state_subscription::{PublishedState, StateSubscriptionPresenter};
use crate::infrastructure::state_subscription::{
    DeltaPublication, DeltaPublicationError, StateSubscriptionRuntime, Subscriptions, Version,
};
use crate::infrastructure::terminal::output_flow_control::{
    OUTPUT_PENDING_LIMIT, OUTPUT_REPORT_UNITS,
};
use crate::usecase::state_subscription::{
    StateReadError, StateValue, SubscriptionError, SubscriptionTarget,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use crate::usecase::terminal_surface::output::{
    TerminalRegistration, TerminalSurfaceOutputEvent, TerminalSurfaceStateSink,
};
use crate::usecase::terminal_surface::subscription::TerminalSubscriptionOutput;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct TerminalSubscriptionPresenter {
    runtime: StateSubscriptionRuntime<PublishedState>,
    boot: Arc<str>,
}

impl TerminalSubscriptionPresenter {
    pub(crate) fn new(presenter: &StateSubscriptionPresenter) -> Self {
        Self {
            runtime: presenter.runtime(),
            boot: uuid::Uuid::new_v4().to_string().into(),
        }
    }
    pub(crate) fn report_units() -> usize {
        OUTPUT_REPORT_UNITS
    }
    #[cfg(test)]
    pub(crate) fn test_runtime(&self) -> &StateSubscriptionRuntime<PublishedState> {
        &self.runtime
    }
    fn version(&self, generation: u64, sequence: u64) -> Version {
        Version {
            epoch: format!("{}:{generation}", self.boot),
            sequence,
        }
    }
    fn update(
        &self,
        update: impl FnOnce(
            &mut Subscriptions<PublishedState>,
        ) -> Result<
            bool,
            crate::infrastructure::state_subscription::SubscriptionError,
        >,
    ) -> Result<(), SubscriptionError> {
        self.runtime.update(update).map_err(Into::into)
    }
}

impl TerminalSubscriptionOutput for TerminalSubscriptionPresenter {
    #[cfg(test)]
    fn start(
        &self,
        client: &str,
        _target: &SubscriptionTarget,
        input_id: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<Option<usize>, StateReadError> {
        let version = cursor.map(|(epoch, sequence)| Version {
            epoch: epoch.into(),
            sequence,
        });
        self.runtime
            .mutate(|state| {
                let result = state.activate(input_id, version.as_ref()).map(|()| {
                    (!state.awaiting_snapshot(client, input_id))
                        .then(|| state.pending_amount(client, input_id))
                });
                let changed = result.is_ok();
                (result, changed)
            })
            .map_err(|error| StateReadError::from_error(SubscriptionError::from(error)))
    }
    #[cfg(test)]
    fn stop(
        &self,
        client: &str,
        _target: &SubscriptionTarget,
        input_id: &str,
    ) -> Result<(), SubscriptionError> {
        self.update(|state| state.stop_and_release(client, input_id))
    }

    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        let snapshot = PublishedState::from(error);
        let target = target.to_string();
        self.update(|state| {
            let version = state.current_version(&target).ok_or(
                crate::infrastructure::state_subscription::SubscriptionError::UnknownTarget,
            )?;
            state.publish_delta(&target, version.clone(), snapshot.clone(), 0, false)?;
            state.set_delta_snapshot(&target, version, snapshot)
        })
    }
    fn set_snapshot(
        &self,
        target: &SubscriptionTarget,
        runtime_generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        let version = self.version(runtime_generation, sequence);
        let snapshot = crate::adaptor::presenter::state_subscription_wire::payload(&snapshot)
            .map(PublishedState::from)
            .map_err(|_| SubscriptionError::EncodingFailed)?;
        self.update(|state| state.set_delta_snapshot(&target.to_string(), version, snapshot))
    }
}

impl TerminalSurfaceStateSink for TerminalSubscriptionPresenter {
    fn initialize(
        &self,
        registration: &TerminalRegistration,
    ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
        let target = terminal_target(
            &registration.workspace_path,
            registration.session_id.as_deref(),
        )
        .map_err(|error| {
            crate::usecase::terminal_surface::error::UsecaseError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                },
            )
        })?
        .to_string();
        let version = self.version(
            registration.runtime_generation,
            registration.latest_sequence,
        );
        self.runtime
            .update(|state| state.register_delta(&target, version, OUTPUT_PENDING_LIMIT))
            .map_err(|error| {
                crate::usecase::terminal_surface::error::UsecaseError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: error.to_string(),
                    },
                )
            })
    }

    fn remove(&self, registration: &TerminalRegistration) -> bool {
        let Ok(target) = terminal_target(
            &registration.workspace_path,
            registration.session_id.as_deref(),
        ) else {
            return false;
        };
        let target = target.to_string();
        let epoch = self.version(registration.runtime_generation, 0).epoch;
        self.runtime
            .mutate(|state| state.unregister_epoch(&target, &epoch))
    }

    fn publish(&self, registration: &TerminalRegistration, event: TerminalSurfaceOutputEvent) {
        let Ok(target) = terminal_target(
            &registration.workspace_path,
            registration.session_id.as_deref(),
        ) else {
            return;
        };
        let target = target.to_string();
        let sequence = match &event {
            TerminalSurfaceOutputEvent::Output { sequence, .. }
            | TerminalSurfaceOutputEvent::Resize { sequence, .. }
            | TerminalSurfaceOutputEvent::Exit { sequence, .. } => *sequence,
        };
        let advances_version = matches!(&event, TerminalSurfaceOutputEvent::Output { .. });
        let (item, units) = match event {
            TerminalSurfaceOutputEvent::Output {
                session_key,
                data,
                sequence,
            } => {
                let units = data.encode_utf16().count();
                (
                    TerminalSurfaceStreamItem::Output {
                        session_key,
                        data,
                        sequence,
                    },
                    units,
                )
            }
            TerminalSurfaceOutputEvent::Resize {
                session_key,
                cols,
                rows,
                sequence,
            } => (
                TerminalSurfaceStreamItem::Resize {
                    session_key,
                    cols,
                    rows,
                    sequence,
                },
                0,
            ),
            TerminalSurfaceOutputEvent::Exit {
                session_key,
                exit_code,
                sequence,
                ..
            } => (
                TerminalSurfaceStreamItem::Exit {
                    session_key,
                    exit_code,
                    sequence,
                },
                0,
            ),
        };
        let payload = match crate::adaptor::presenter::state_subscription_wire::payload(
            &StateValue::Terminal(item),
        ) {
            Ok(payload) => PublishedState::from(payload),
            Err(error) => {
                log::error!("Terminal publication encoding failed: {error}");
                return;
            }
        };
        self.runtime.mutate(|state| {
            match state.apply_delta(&target, sequence, payload, units, advances_version) {
                Ok(DeltaPublication::Published) => ((), true),
                Ok(DeltaPublication::Discarded) => ((), false),
                Ok(DeltaPublication::SnapshotRequired(changed)) => ((), changed),
                Err(DeltaPublicationError::Snapshot(error)) => {
                    log::error!("Terminal resynchronization failed: {error}");
                    ((), false)
                }
                Err(DeltaPublicationError::Publication(error)) => {
                    log::error!("Terminal publication failed: {error}");
                    ((), false)
                }
            }
        });
    }
}

fn terminal_target(
    workspace_path: &str,
    session_id: Option<&str>,
) -> Result<SubscriptionTarget, SubscriptionError> {
    let mut args = vec![workspace_path];
    if let Some(session_id) = session_id {
        args.push(session_id);
    }
    SubscriptionTarget::from_parts("terminal", &args)
}

#[cfg(test)]
#[path = "terminal_subscription_test.rs"]
mod terminal_subscription_tests;
