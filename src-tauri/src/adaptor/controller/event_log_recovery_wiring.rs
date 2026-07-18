use std::sync::{Arc, Weak};

use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::SessionStore;

pub(crate) fn register_event_log_recovery_listener(
    session_store: Arc<SessionStore>,
    runtime_usecase: &Arc<AgentSessionRuntimeUsecase>,
) {
    let runtime_usecase = Arc::downgrade(runtime_usecase);
    register_event_log_recovery_listener_with_weak(session_store, runtime_usecase);
}

fn register_event_log_recovery_listener_with_weak(
    session_store: Arc<SessionStore>,
    runtime_usecase: Weak<AgentSessionRuntimeUsecase>,
) {
    session_store.register_event_log_recovery_listener(Arc::new(move |session_id| {
        if let Some(runtime_usecase) = runtime_usecase.upgrade() {
            runtime_usecase.report_event_log_recovered(session_id);
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_listener_does_not_retain_runtime_usecase() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (runtime_usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let runtime_usecase_weak = Arc::downgrade(&runtime_usecase);

        drop(runtime_usecase);

        assert!(runtime_usecase_weak.upgrade().is_none());
        assert_eq!(Arc::strong_count(&session_store), 1);
    }
}
