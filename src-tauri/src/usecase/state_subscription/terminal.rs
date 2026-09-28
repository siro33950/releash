use super::*;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;

fn error(e: impl std::fmt::Display) -> StateReadError {
    StateReadError::from_error(TechnicalFailure {
        nature: TechnicalFailureNature::Other,
        message: e.to_string(),
    })
}

impl StateSubscriptionUsecase {
    pub async fn start_terminal(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        if input_id.trim().is_empty() || input_id.len() > crate::common::SUBSCRIPTION_ID_MAX_BYTES {
            return Err(StateReadError {
                source: StateReadFailure::InvalidTerminalInput,
                message: "Invalid terminal input identity".into(),
            });
        }
        let SubscriptionTarget::Terminal(_) = target else {
            return Err(error("Not a terminal target"));
        };
        self.terminal
            .as_ref()
            .ok_or_else(|| error("Terminal unavailable"))?;
        self.start(client, target)
            .map_err(StateReadError::from_error)?;
        self.attach_terminal(client, target, input_id)?;
        if let Err(error) = self.publisher.start(client, target, cursor, Some(input_id)) {
            let _ = self.stop(client, target);
            return Err(error);
        }
        if let Err(error) = self.attach_terminal(client, target, input_id) {
            if let (Some(terminal), SubscriptionTarget::Terminal(owner)) = (&self.terminal, target)
            {
                terminal.unsubscribe_output(owner, client, input_id);
            }
            let _ = self.publisher.stop(client, target, &self.active_targets());
            return Err(error);
        }
        Ok(())
    }

    fn attach_terminal(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: &str,
    ) -> Result<(), StateReadError> {
        let clients = self.clients.lock();
        if !clients
            .get(client)
            .is_some_and(|targets| targets.contains(target))
        {
            return Err(StateReadError::from_error(SubscriptionError::StreamEnded));
        }
        self.terminal_inputs
            .lock()
            .insert((client.into(), target.clone()), input_id.into());
        Ok(())
    }

    pub(crate) async fn refresh_terminal(
        &self,
        target: &SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(owner) = target else {
            return Ok(());
        };
        let owner = owner.clone();
        let terminal = self
            .terminal
            .clone()
            .ok_or_else(|| error("Terminal unavailable"))?;
        let output = self.publisher.clone();
        let target = target.clone();
        let resets = self.terminal_resets.clone();
        let inputs = self.terminal_inputs.clone();
        tokio::task::spawn_blocking(move || {
            let mut result = Ok(());
            terminal
                .visit_snapshot(&owner, &mut |surface| {
                    result = output.set_terminal_snapshot(
                        &target,
                        surface.runtime_generation.value(),
                        surface.latest_sequence(),
                        StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface)),
                    );
                    if result.is_ok() {
                        let reset = resets.lock().remove(&target).unwrap_or_default();
                        let active = inputs.lock();
                        for client in reset {
                            if active.contains_key(&(client.clone(), target.clone())) {
                                terminal.reset_output(&owner, &client);
                            }
                        }
                    }
                })
                .map_err(error)?;
            result.map_err(error)
        })
        .await
        .map_err(error)?
    }

    pub(super) fn stop_terminal(&self, client: &str, target: &SubscriptionTarget) {
        if let Some(input_id) = self
            .terminal_inputs
            .lock()
            .remove(&(client.into(), target.clone()))
        {
            if let (Some(terminal), SubscriptionTarget::Terminal(owner)) = (&self.terminal, target)
            {
                terminal.unsubscribe_output(owner, client, &input_id);
            }
        }
    }

    pub fn terminal_processed(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        units: usize,
    ) -> Result<(), StateReadError> {
        if !self
            .terminal_inputs
            .lock()
            .contains_key(&(client.into(), target.clone()))
        {
            return Err(StateReadError {
                source: StateReadFailure::TerminalSubscriptionEnded,
                message: "Terminal subscription ended".into(),
            });
        }
        let SubscriptionTarget::Terminal(owner) = target else {
            return Err(error("Not a terminal target"));
        };
        self.terminal
            .as_ref()
            .ok_or_else(|| error("Terminal unavailable"))?
            .processed_output(owner, client, units);
        Ok(())
    }
}

#[cfg(test)]
#[path = "terminal_test.rs"]
mod terminal_tests;
