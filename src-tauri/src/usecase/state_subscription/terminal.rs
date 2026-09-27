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
    ) -> Result<(), StateReadError> {
        if input_id.trim().is_empty() || input_id.len() > 128 {
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
            .map_err(StateReadError::from_error)
    }

    pub(crate) fn attach_terminal(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: &str,
    ) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(_) = target else {
            return Err(error("Not a terminal target"));
        };
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
        tokio::task::spawn_blocking(move || {
            let mut result = Ok(());
            terminal
                .visit_snapshot(&owner, &mut |surface| {
                    let reset = resets.lock().remove(&target).unwrap_or_default();
                    result = output.set_terminal_snapshot(
                        &target,
                        surface.runtime_generation.value(),
                        surface.latest_sequence(),
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
