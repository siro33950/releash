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
        raw: &str,
        cursor: Option<(&str, u64)>,
        input_id: &str,
    ) -> Result<(), StateReadError> {
        if input_id.trim().is_empty() || input_id.len() > 128 {
            return Err(StateReadError {
                source: StateReadFailure::InvalidTerminalInput,
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
        let output = self.publisher.clone();
        let client = client.to_string();
        let raw = raw.to_string();
        let cursor = cursor.map(|(epoch, sequence)| (epoch.to_string(), sequence));
        let input_id = input_id.to_string();
        tokio::task::spawn_blocking(move || {
            let summary = terminal.get_summary(&owner).map_err(error)?;
            let cursor = cursor
                .as_ref()
                .map(|(epoch, sequence)| (epoch.as_str(), *sequence));
            let mut start_result = Ok(());
            let mut needs_snapshot = true;
            terminal.with_output_order(summary.runtime_generation.value(), &mut || {
                start_result = output
                    .needs_snapshot(&raw, cursor)
                    .map(|required| needs_snapshot = required);
                if start_result.is_ok() && !needs_snapshot {
                    start_result = output.start(&client, &raw, cursor);
                    if start_result.is_ok() {
                        terminal.subscribe_output(
                            &owner,
                            &client,
                            &input_id,
                            output.terminal_pending_amount(&client, &raw),
                        );
                        output.set_terminal_input(&client, &raw, &input_id);
                    }
                }
            });
            if start_result.is_ok() && needs_snapshot {
                terminal
                    .visit_snapshot(&owner, &mut |surface| {
                        start_result = output
                            .set_terminal_snapshot(
                                &raw,
                                surface.runtime_generation.value(),
                                surface.latest_sequence(),
                                StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface)),
                            )
                            .and_then(|()| output.stop(&client, &raw))
                            .and_then(|()| output.start(&client, &raw, cursor));
                        if start_result.is_ok() {
                            terminal.subscribe_output(
                                &owner,
                                &client,
                                &input_id,
                                output.terminal_pending_amount(&client, &raw),
                            );
                            output.set_terminal_input(&client, &raw, &input_id);
                        }
                    })
                    .map_err(error)?;
            }
            start_result.map_err(StateReadError::from_error)
        })
        .await
        .map_err(error)?
    }

    pub(crate) async fn refresh_terminal(&self, raw: &str) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(owner) = SubscriptionTarget::parse(raw).map_err(error)?
        else {
            return Ok(());
        };
        let terminal = self
            .terminal
            .clone()
            .ok_or_else(|| error("Terminal unavailable"))?;
        let output = self.publisher.clone();
        let raw = raw.to_string();
        tokio::task::spawn_blocking(move || {
            let mut result = Ok(());
            terminal
                .visit_snapshot(&owner, &mut |surface| {
                    let reset = output.terminal_reset_clients(&raw);
                    result = output.set_terminal_snapshot(
                        &raw,
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

    pub(super) fn stop_terminal(&self, client: &str, raw: &str) {
        if let Some(input_id) = self.publisher.remove_terminal_input(client, raw) {
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
        if units != self.publisher.terminal_report_units() {
            return Err(StateReadError {
                source: StateReadFailure::InvalidTerminalInput,
                message: "Invalid terminal processed units".into(),
            });
        }
        if !self.publisher.has_terminal_input(client, raw) {
            return Err(StateReadError {
                source: StateReadFailure::TerminalSubscriptionEnded,
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
