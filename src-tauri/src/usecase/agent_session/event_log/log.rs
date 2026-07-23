#[cfg(test)]
use std::cell::Cell;

use super::events::AgentSessionEvent;
#[cfg(test)]
use super::events::{InterruptReason, PromptInput, TurnId};
#[cfg(test)]
use super::finalization::finalize_turn;
#[cfg(test)]
use super::part_events::{append_part_events, PartEventMode};
use super::projector::{project, SessionReadModel};
#[cfg(test)]
use crate::usecase::agent_session::session::MessagePart;

#[derive(Debug, Clone, Default)]
pub struct TurnEventLog {
    events: Vec<AgentSessionEvent>,
    #[cfg(test)]
    project_call_count: Cell<usize>,
}

impl PartialEq for TurnEventLog {
    fn eq(&self, other: &Self) -> bool {
        self.events == other.events
    }
}

impl TurnEventLog {
    pub fn from_events(events: Vec<AgentSessionEvent>) -> Self {
        Self {
            events,
            #[cfg(test)]
            project_call_count: Cell::new(0),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)] // issues-1301 G-3: retained for event-log scenario tests.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    #[cfg(test)]
    pub fn append(&mut self, event: AgentSessionEvent) {
        self.events.push(event);
    }

    #[cfg(test)]
    pub fn append_part_events(
        &mut self,
        turn_id: TurnId,
        message_id: &str,
        parts: &[MessagePart],
        mode: PartEventMode,
    ) -> usize {
        append_part_events(&mut self.events, turn_id, message_id, parts, mode)
    }

    #[cfg(test)]
    pub fn begin_turn(
        &mut self,
        turn_id: TurnId,
        prompt_message_id: String,
        assistant_message_id: String,
        prompt: PromptInput,
        at: f64,
    ) {
        self.append(AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: prompt_message_id,
            assistant_message_id: Some(assistant_message_id),
            prompt,
            at,
        });
    }

    pub fn project(&self) -> SessionReadModel {
        #[cfg(test)]
        self.project_call_count
            .set(self.project_call_count.get().saturating_add(1));
        project(&self.events)
    }

    pub fn queue_paused_at(&self) -> Option<f64> {
        self.project().queue_paused_at
    }

    #[cfg(test)]
    #[allow(dead_code)] // issues-1301 G-3: retained for projector performance/regression tests.
    pub fn project_call_count(&self) -> usize {
        self.project_call_count.get()
    }

    #[cfg(test)]
    #[allow(dead_code)] // issues-1301 G-3: retained for projector performance/regression tests.
    pub fn reset_project_call_count(&self) {
        self.project_call_count.set(0);
    }

    #[cfg(test)]
    #[allow(dead_code)] // issues-1301 G-3: retained for event-log finalization scenario tests.
    pub fn finalize(
        &mut self,
        turn_id: TurnId,
        reason: InterruptReason,
        error: Option<String>,
        exit_code: i64,
    ) {
        finalize_turn(&mut self.events, turn_id, reason, error, exit_code);
    }
}
