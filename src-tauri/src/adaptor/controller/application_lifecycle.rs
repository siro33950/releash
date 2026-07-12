use std::sync::Arc;

#[async_trait::async_trait]
trait AgentSessionShutdown {
    async fn close_all(&self);
}

#[async_trait::async_trait]
impl AgentSessionShutdown for crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase {
    async fn close_all(&self) {
        crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase::close_all(self).await;
    }
}

#[async_trait::async_trait]
trait WorkflowCommandShutdown {
    async fn shutdown_active_commands(&self);
}

#[async_trait::async_trait]
impl WorkflowCommandShutdown for crate::usecase::workflow::WorkflowRuntimeUsecase {
    async fn shutdown_active_commands(&self) {
        crate::usecase::workflow::WorkflowRuntimeUsecase::shutdown_active_commands(self).await;
    }
}

pub(crate) fn request_application_quit_with_runtime(
    app: tauri::AppHandle,
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
) {
    tauri::async_runtime::spawn(async move {
        shutdown_application_services(workflow_runtime.as_ref(), runtime.as_ref()).await;
        app.exit(0);
    });
}

async fn shutdown_application_services<W, A>(workflow_runtime: &W, runtime: &A)
where
    W: WorkflowCommandShutdown + Sync,
    A: AgentSessionShutdown + Sync,
{
    workflow_runtime.shutdown_active_commands().await;
    // Kill all agent sessions before stopping the server.
    runtime.close_all().await;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;

    #[derive(Clone)]
    struct RecordingShutdown {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait::async_trait]
    impl WorkflowCommandShutdown for RecordingShutdown {
        async fn shutdown_active_commands(&self) {
            self.calls.lock().await.push("shutdown_active_commands");
        }
    }

    #[async_trait::async_trait]
    impl AgentSessionShutdown for RecordingShutdown {
        async fn close_all(&self) {
            self.calls.lock().await.push("close_all");
        }
    }

    #[tokio::test]
    async fn application_quit_shuts_down_workflow_commands_before_agent_sessions() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let workflow_runtime = RecordingShutdown {
            calls: calls.clone(),
        };
        let runtime = RecordingShutdown {
            calls: calls.clone(),
        };

        shutdown_application_services(&workflow_runtime, &runtime).await;

        assert_eq!(
            calls.lock().await.as_slice(),
            ["shutdown_active_commands", "close_all"]
        );
    }
}
