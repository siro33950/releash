use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::usecase::application_lifecycle::{
    AgentSessionShutdownPort, ApplicationLifecycleUsecase, LocalApiShutdownPort,
    WorkflowCommandShutdownPort,
};

#[async_trait::async_trait]
impl AgentSessionShutdownPort
    for crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase
{
    async fn close_all(&self) -> Result<(), String> {
        crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase::close_all(self)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait::async_trait]
impl WorkflowCommandShutdownPort for crate::usecase::workflow::WorkflowRuntimeUsecase {
    async fn shutdown_active_commands(&self) {
        crate::usecase::workflow::WorkflowRuntimeUsecase::shutdown_active_commands(self).await;
    }
}

impl<F> LocalApiShutdownPort for F
where
    F: Fn() + Send + Sync,
{
    fn shutdown(&self) {
        self();
    }
}

pub(crate) fn request_application_quit_with_runtime<F>(
    app: tauri::AppHandle,
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    shutdown_local_api: F,
) where
    F: Fn() + Send + Sync + 'static,
{
    tauri::async_runtime::spawn(async move {
        let lifecycle = ApplicationLifecycleUsecase::new(
            runtime.as_ref(),
            workflow_runtime.as_ref(),
            &shutdown_local_api,
        );
        shutdown_application(&lifecycle, || app.exit(0)).await;
    });
}

async fn shutdown_application<A, W, L, F>(
    lifecycle: &ApplicationLifecycleUsecase<'_, A, W, L>,
    exit: F,
) where
    A: AgentSessionShutdownPort,
    W: WorkflowCommandShutdownPort,
    L: LocalApiShutdownPort,
    F: FnOnce(),
{
    match lifecycle.shutdown().await {
        Ok(()) => exit(),
        Err(error) => {
            crate::infrastructure::platform::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
            log::error!("application shutdown aborted: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct FailingShutdown;

    #[async_trait::async_trait]
    impl AgentSessionShutdownPort for FailingShutdown {
        async fn close_all(&self) -> Result<(), String> {
            Err("injected shutdown failure".to_string())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowCommandShutdownPort for FailingShutdown {
        async fn shutdown_active_commands(&self) {}
    }

    impl LocalApiShutdownPort for FailingShutdown {
        fn shutdown(&self) {}
    }

    #[tokio::test]
    async fn shutdown_failure_restores_exit_protection_and_allows_tray_retry() {
        let shutdown = FailingShutdown;
        let lifecycle = ApplicationLifecycleUsecase::new(&shutdown, &shutdown, &shutdown);
        let exited = AtomicBool::new(false);
        {
            let _guard = crate::infrastructure::platform::tray::QUIT_REQUESTED_TEST_LOCK
                .lock()
                .unwrap();
            crate::infrastructure::platform::tray::mark_quit_requested();
        }

        shutdown_application(&lifecycle, || exited.store(true, Ordering::SeqCst)).await;

        let _guard = crate::infrastructure::platform::tray::QUIT_REQUESTED_TEST_LOCK
            .lock()
            .unwrap();
        assert!(!exited.load(Ordering::SeqCst));
        assert!(!crate::infrastructure::platform::tray::QUIT_REQUESTED.load(Ordering::SeqCst));
        assert!(crate::infrastructure::platform::window_lifecycle::should_prevent_exit());

        crate::infrastructure::platform::tray::mark_quit_requested();
        assert!(!crate::infrastructure::platform::window_lifecycle::should_prevent_exit());
        crate::infrastructure::platform::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
    }
}
