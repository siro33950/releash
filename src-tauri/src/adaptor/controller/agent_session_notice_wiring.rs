use std::sync::Arc;

use crate::adaptor::presenter::agent_session_notice::TauriAgentSessionNoticePublisher;
use crate::usecase::agent_session::notice::AgentSessionNoticeUsecase;
use crate::usecase::agent_session::notice_query_service::AgentSessionNoticeQueryService;
use crate::usecase::agent_session::notice_state::new_shared_agent_session_notice_state;

pub(crate) fn build_agent_session_notice_usecase() -> AgentSessionNoticeUsecase {
    let state = new_shared_agent_session_notice_state();
    AgentSessionNoticeUsecase::new_for_runtime(
        state.clone(),
        AgentSessionNoticeQueryService::new(state),
    )
}

pub(crate) fn register_agent_session_notice_publisher<R: tauri::Runtime>(
    notice_usecase: Arc<AgentSessionNoticeUsecase>,
    app: tauri::AppHandle<R>,
) {
    notice_usecase.register_publisher(Arc::new(TauriAgentSessionNoticePublisher::new(app)));
}
