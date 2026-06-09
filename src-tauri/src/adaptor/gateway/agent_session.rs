pub(crate) fn agent_state_to_msg(
    state: crate::usecase::agent_session::status::AgentState,
) -> crate::protocol::AgentState {
    match state {
        crate::usecase::agent_session::status::AgentState::Running => {
            crate::protocol::AgentState::Running
        }
        crate::usecase::agent_session::status::AgentState::Done => {
            crate::protocol::AgentState::Done
        }
        crate::usecase::agent_session::status::AgentState::Error => {
            crate::protocol::AgentState::Error
        }
        crate::usecase::agent_session::status::AgentState::Waiting => {
            crate::protocol::AgentState::Waiting
        }
    }
}
