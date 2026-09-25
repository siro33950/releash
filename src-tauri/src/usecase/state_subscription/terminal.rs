use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
use crate::domain::state_subscription::SubscriptionTarget;
use crate::domain::terminal_surface::entities::TerminalSurfaceSummary;
use crate::domain::terminal_surface::gateway::{TerminalSurfaceEvent, TerminalSurfaceStateSink};
use crate::domain::terminal_surface::value_objects::output_flow_control::{
    OUTPUT_PENDING_LIMIT, OUTPUT_REPORT_UNITS,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;

fn error(e: impl std::fmt::Display) -> StateReadError {
    StateReadError {
        kind: FailureKind::Internal,
        message: e.to_string(),
    }
}

impl StateSubscriptionUsecase {
    pub async fn start_terminal(
        &self,
        client: &str,
        raw: &str,
        version: Option<&Version>,
        input_id: &str,
    ) -> Result<(), StateReadError> {
        if input_id.trim().is_empty() || input_id.len() > 128 {
            return Err(StateReadError {
                kind: FailureKind::InvalidInput,
                message: "Invalid terminal input identity".into(),
            });
        }
        let SubscriptionTarget::Terminal(owner) = SubscriptionTarget::parse(raw).map_err(error)?
        else {
            return Err(error("Not a terminal target"));
        };
        let terminal = self
            .terminal
            .clone()
            .ok_or_else(|| error("Terminal unavailable"))?;
        let usecase = self.clone();
        let client = client.to_string();
        let raw = raw.to_string();
        let version = version.cloned();
        let input_id = input_id.to_string();
        tokio::task::spawn_blocking(move || {
            let summary = terminal.get_summary(&owner).map_err(error)?;
            let mut start_result = Ok(());
            let mut needs_snapshot = true;
            terminal.with_output_order(summary.runtime_generation.value(), &mut || {
                let mut state = usecase.publisher.state.lock();
                start_result = state
                    .needs_snapshot(&raw, version.as_ref())
                    .map(|required| needs_snapshot = required);
                if start_result.is_ok() && !needs_snapshot {
                    start_result = state.start(&client, &raw, version.as_ref());
                    if start_result.is_ok() {
                        terminal.subscribe_output(
                            &owner,
                            &client,
                            &input_id,
                            state.pending_amount(&client, &raw, |value| match value {
                                StateValue::Terminal(TerminalSurfaceStreamItem::Output {
                                    data,
                                    ..
                                }) => data.encode_utf16().count(),
                                _ => 0,
                            }),
                        );
                        usecase
                            .terminal_inputs
                            .lock()
                            .insert((client.clone(), raw.clone()), input_id.clone());
                    }
                }
            });
            if start_result.is_ok() && needs_snapshot {
                terminal
                    .visit_snapshot(&owner, &mut |surface| {
                        let mut state = usecase.publisher.state.lock();
                        let snapshot_version = Version {
                            epoch: format!(
                                "{}:{}",
                                usecase.publisher.boot,
                                surface.runtime_generation.value()
                            ),
                            sequence: surface.latest_sequence(),
                        };
                        start_result = state
                            .set_delta_snapshot(
                                &raw,
                                snapshot_version,
                                StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface)),
                            )
                            .and_then(|()| state.stop(&client, &raw))
                            .and_then(|()| state.start(&client, &raw, version.as_ref()));
                        if start_result.is_ok() {
                            terminal.subscribe_output(
                                &owner,
                                &client,
                                &input_id,
                                state.pending_amount(&client, &raw, |value| match value {
                                    StateValue::Terminal(TerminalSurfaceStreamItem::Output {
                                        data,
                                        ..
                                    }) => data.encode_utf16().count(),
                                    _ => 0,
                                }),
                            );
                            usecase
                                .terminal_inputs
                                .lock()
                                .insert((client.clone(), raw.clone()), input_id.clone());
                        }
                    })
                    .map_err(error)?;
            }
            start_result.map_err(|e| StateReadError {
                kind: e.failure_kind(),
                message: e.to_string(),
            })?;
            usecase.publisher.changed.notify_waiters();
            Ok(())
        })
        .await
        .map_err(error)?
    }

    pub(super) async fn refresh_terminal(&self, raw: &str) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(owner) = SubscriptionTarget::parse(raw).map_err(error)?
        else {
            return Ok(());
        };
        let terminal = self
            .terminal
            .clone()
            .ok_or_else(|| error("Terminal unavailable"))?;
        let publisher = self.publisher.clone();
        let inputs = self.terminal_inputs.clone();
        let raw = raw.to_string();
        tokio::task::spawn_blocking(move || {
            let mut result = Ok(());
            terminal
                .visit_snapshot(&owner, &mut |surface| {
                    let version = Version {
                        epoch: format!("{}:{}", publisher.boot, surface.runtime_generation.value()),
                        sequence: surface.latest_sequence(),
                    };
                    let mut state = publisher.state.lock();
                    let reset: Vec<_> = inputs
                        .lock()
                        .keys()
                        .filter(|(client, target)| {
                            target == &raw && state.snapshot_requests(client).contains(&raw)
                        })
                        .map(|(client, _)| client.clone())
                        .collect();
                    result = state.set_delta_snapshot(
                        &raw,
                        version,
                        StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface)),
                    );
                    if result.is_ok() {
                        for client in reset {
                            terminal.reset_output(&owner, &client);
                        }
                    }
                })
                .map_err(error)?;
            result.map_err(error)
        })
        .await
        .map_err(error)?
    }

    pub(super) fn stop_terminal(&self, client: &str, raw: &str) {
        if let Some(input_id) = self
            .terminal_inputs
            .lock()
            .remove(&(client.into(), raw.into()))
        {
            if let (Some(terminal), Ok(SubscriptionTarget::Terminal(owner))) =
                (&self.terminal, SubscriptionTarget::parse(raw))
            {
                terminal.unsubscribe_output(&owner, client, &input_id);
            }
        }
    }

    pub fn terminal_processed(
        &self,
        client: &str,
        raw: &str,
        units: usize,
    ) -> Result<(), StateReadError> {
        if units != OUTPUT_REPORT_UNITS {
            return Err(StateReadError {
                kind: FailureKind::InvalidInput,
                message: "Invalid terminal processed units".into(),
            });
        }
        if !self
            .terminal_inputs
            .lock()
            .contains_key(&(client.into(), raw.into()))
        {
            return Err(StateReadError {
                kind: FailureKind::Missing,
                message: "Terminal subscription ended".into(),
            });
        }
        let SubscriptionTarget::Terminal(owner) = SubscriptionTarget::parse(raw).map_err(error)?
        else {
            return Err(error("Not a terminal target"));
        };
        self.terminal
            .as_ref()
            .ok_or_else(|| error("Terminal unavailable"))?
            .processed_output(&owner, client, units);
        Ok(())
    }
}

impl TerminalSurfaceStateSink for StateSubscriptionPublisher {
    fn initialize(&self, surface: &TerminalSurfaceSummary) {
        let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
        self.terminal_routes
            .lock()
            .insert(surface.session_key.clone(), target.clone());
        let version = Version {
            epoch: format!("{}:{}", self.boot, surface.runtime_generation.value()),
            sequence: surface.latest_sequence,
        };
        if let Err(e) =
            self.update(|state| state.register_delta(&target, version, OUTPUT_PENDING_LIMIT))
        {
            log::error!("Terminal registration failed: {e}");
        }
    }

    fn remove(&self, surface: &TerminalSurfaceSummary) -> bool {
        let mut routes = self.terminal_routes.lock();
        let Some(target) = routes.get(&surface.session_key) else {
            return false;
        };
        let mut state = self.state.lock();
        let epoch = format!("{}:{}", self.boot, surface.runtime_generation.value());
        if state
            .current_version(target)
            .is_none_or(|version| version.epoch != epoch)
        {
            return false;
        }
        if let Err(error) = state.unregister(target) {
            log::error!("Terminal removal failed: {error}");
            return false;
        }
        let mut inputs = self.terminal_inputs.lock();
        inputs.retain(|(client, raw), _| raw != target || state.is_subscribed(client, raw));
        let subscribed = inputs.keys().any(|(_, raw)| raw == target);
        routes.remove(&surface.session_key);
        self.changed.notify_waiters();
        subscribed
    }

    fn publish(&self, event: TerminalSurfaceEvent) {
        let Some(target) = self
            .terminal_routes
            .lock()
            .get(event.session_key())
            .cloned()
        else {
            return;
        };
        let mut state = self.state.lock();
        let Some(mut version) = state.current_version(&target) else {
            return;
        };
        let sequence = match &event {
            TerminalSurfaceEvent::Output { sequence, .. }
            | TerminalSurfaceEvent::Resize { sequence, .. }
            | TerminalSurfaceEvent::Exit { sequence, .. } => *sequence,
        };
        let advances_version = matches!(&event, TerminalSurfaceEvent::Output { .. });
        if sequence < version.sequence && !advances_version {
            if let Err(error) = state.require_delta_snapshot(&target) {
                log::error!("Terminal resynchronization failed: {error}");
            }
            drop(state);
            self.changed.notify_waiters();
            return;
        }
        if advances_version && sequence <= version.sequence {
            return;
        }
        let (item, units) = match event {
            TerminalSurfaceEvent::Output {
                session_key,
                data,
                sequence,
            } => {
                version.sequence = sequence;
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
            TerminalSurfaceEvent::Resize {
                session_key,
                cols,
                rows,
                sequence,
            } => {
                version.sequence = sequence;
                (
                    TerminalSurfaceStreamItem::Resize {
                        session_key,
                        cols,
                        rows,
                        sequence,
                    },
                    0,
                )
            }
            TerminalSurfaceEvent::Exit {
                session_key,
                exit_code,
                sequence,
                ..
            } => {
                version.sequence = sequence;
                (
                    TerminalSurfaceStreamItem::Exit {
                        session_key,
                        exit_code,
                        sequence,
                    },
                    0,
                )
            }
        };
        if let Err(e) = state.publish_delta(
            &target,
            version,
            StateValue::Terminal(item),
            units,
            advances_version,
        ) {
            log::error!("Terminal publication failed: {e}");
        }
        drop(state);
        self.changed.notify_waiters();
    }
}

#[cfg(test)]
#[path = "terminal_test.rs"]
mod terminal_tests;
