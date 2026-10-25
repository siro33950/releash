use std::sync::Arc;

use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::agent_session::services::AgentSessionService;
use crate::usecase::agent_session::test_helpers::{
    FakeThreadRepository, RecordingTerminal, RecordingTerminalInput,
};
use crate::usecase::agent_session::AgentSessionUsecase;

pub fn test_sessions(
    repository: Arc<dyn AgentSessionRepository>,
    threads: Arc<FakeThreadRepository>,
    terminal: Arc<RecordingTerminal>,
    input: Arc<RecordingTerminalInput>,
) -> AgentSessionUsecase {
    AgentSessionUsecase::new(
        repository,
        threads,
        AgentSessionService::new(terminal, input),
    )
}
