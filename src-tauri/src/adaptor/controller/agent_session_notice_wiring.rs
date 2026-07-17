use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::presenter::agent_session_notice::TauriAgentSessionNoticePublisher;
use crate::usecase::agent_session::notice::{
    AgentSessionNoticeSessionLookup, AgentSessionNoticeUpdate, AgentSessionNoticeUsecase,
};
use crate::usecase::agent_session::notice_query_service::AgentSessionNoticeQueryService;
use crate::usecase::agent_session::notice_state::new_shared_agent_session_notice_state;
use crate::usecase::agent_session::session::{SessionState, SessionStore};

struct SessionStoreNoticeSessionLookup {
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
}

impl AgentSessionNoticeSessionLookup for SessionStoreNoticeSessionLookup {
    fn contains_session(&self, session_id: &str) -> bool {
        match self
            .session_store
            .get_session_meta(&self.data_dir, session_id)
        {
            Ok(session) => session.is_some(),
            Err(error) => {
                log::debug!("failed to validate agent session notice target {session_id}: {error}");
                false
            }
        }
    }
}

pub(crate) fn build_agent_session_notice_usecase(
    session_store: Arc<SessionStore>,
    data_dir: &Path,
) -> AgentSessionNoticeUsecase {
    let state = new_shared_agent_session_notice_state();
    AgentSessionNoticeUsecase::new(
        state.clone(),
        AgentSessionNoticeQueryService::new(state),
        Arc::new(SessionStoreNoticeSessionLookup {
            session_store,
            data_dir: data_dir.to_path_buf(),
        }),
    )
}

/// Session lifecycle の正規 state transition に notice retention invariant を結合する。
/// frontend や個別 command が remove を申告しなくても Closed / Archived への全経路で破棄する。
pub(crate) fn register_session_notice_cleanup_listener(
    session_store: &SessionStore,
    notice_usecase: Arc<AgentSessionNoticeUsecase>,
) {
    session_store.register_state_change_listener(Arc::new(move |session_id, _, new_state| {
        if matches!(new_state, SessionState::Closed | SessionState::Archived) {
            notice_usecase.update(session_id, AgentSessionNoticeUpdate::RemoveSession);
        }
    }));
}

pub(crate) fn register_agent_session_notice_publisher<R: tauri::Runtime>(
    notice_usecase: Arc<AgentSessionNoticeUsecase>,
    app: tauri::AppHandle<R>,
) {
    notice_usecase.register_publisher(Arc::new(TauriAgentSessionNoticePublisher::new(app)));
}
