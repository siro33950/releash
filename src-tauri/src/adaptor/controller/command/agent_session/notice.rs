use tauri::State;

use crate::adaptor::protocol::agent_session_notice::{
    AgentSessionNoticeSnapshotMessage, AgentSessionNoticeUpdateMessage,
    SessionFeedbackErrorMessage, SessionFeedbackPageMessage, SessionFeedbackRetryOutcomeMessage,
};
use crate::adaptor::protocol::agent_session_v1::decode_nonnegative_u64_decimal;
use crate::usecase::agent_session::feedback::SessionFeedbackUsecase;
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

#[tauri::command]
pub async fn list_agent_session_feedback(
    state: State<'_, std::sync::Arc<SessionFeedbackUsecase>>,
    session_id: String,
    limit: usize,
    cursor: Option<String>,
) -> Result<SessionFeedbackPageMessage, SessionFeedbackErrorMessage> {
    state
        .list(&session_id, limit, cursor)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dismiss_agent_session_feedback(
    state: State<'_, std::sync::Arc<SessionFeedbackUsecase>>,
    session_id: String,
    feedback_id: String,
    expected_revision: String,
    action_id: String,
) -> Result<(), SessionFeedbackErrorMessage> {
    let expected_revision =
        parse_revision(&expected_revision).ok_or(SessionFeedbackErrorMessage::InvalidRequest)?;
    state
        .dismiss(&session_id, &feedback_id, expected_revision, &action_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn retry_agent_session_feedback(
    state: State<'_, std::sync::Arc<SessionFeedbackUsecase>>,
    session_id: String,
    feedback_id: String,
    expected_revision: String,
    action_id: String,
) -> Result<SessionFeedbackRetryOutcomeMessage, SessionFeedbackErrorMessage> {
    let expected_revision =
        parse_revision(&expected_revision).ok_or(SessionFeedbackErrorMessage::InvalidRequest)?;
    state
        .retry_resolution(&session_id, &feedback_id, expected_revision, &action_id)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

fn parse_revision(raw: &str) -> Option<u64> {
    decode_nonnegative_u64_decimal(raw)
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
        assert_eq!(response.revision, "1");
        assert_eq!(response.notice.unwrap().message, "send failed");
    }
}
