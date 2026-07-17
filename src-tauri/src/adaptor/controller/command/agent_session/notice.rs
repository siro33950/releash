use tauri::State;

use crate::adaptor::protocol::agent_session_notice::{
    AgentSessionNoticeSnapshotMessage, AgentSessionNoticeUpdateMessage,
};
use crate::usecase::agent_session::notice::AgentSessionNoticeUsecase;

#[tauri::command]
pub fn update_agent_session_notice(
    state: State<'_, std::sync::Arc<AgentSessionNoticeUsecase>>,
    session_id: String,
    update: AgentSessionNoticeUpdateMessage,
) -> AgentSessionNoticeSnapshotMessage {
    state.update(&session_id, update.into()).into()
}

#[tauri::command]
pub fn get_agent_session_notice(
    state: State<'_, std::sync::Arc<AgentSessionNoticeUsecase>>,
    session_id: String,
) -> AgentSessionNoticeSnapshotMessage {
    state.get_notice(&session_id).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::notice::{
        AgentSessionNoticeOperation, AgentSessionNoticeUpdate,
    };
    use tauri::Manager;

    #[test]
    fn get_notice_command_delegates_to_managed_notice_usecase() {
        let usecase = std::sync::Arc::new(AgentSessionNoticeUsecase::default());
        usecase.update(
            "session-a",
            AgentSessionNoticeUpdate::Failure {
                operation: AgentSessionNoticeOperation::Send,
                message: "send failed".to_string(),
            },
        );
        let app = tauri::test::mock_builder()
            .manage(usecase)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock app must build");

        let response = get_agent_session_notice(
            app.state::<std::sync::Arc<AgentSessionNoticeUsecase>>(),
            "session-a".to_string(),
        );

        assert_eq!(response.session_id, "session-a");
        assert_eq!(response.revision, 1);
        assert_eq!(response.notice.unwrap().message, "send failed");
    }
}
