#[async_trait::async_trait]
pub(crate) trait AgentSessionShutdownPort: Send + Sync {
    async fn close_all(&self) -> Result<(), String>;
}

#[async_trait::async_trait]
pub(crate) trait WorkflowCommandShutdownPort: Send + Sync {
    async fn shutdown_active_commands(&self);
}

pub(crate) trait LocalApiShutdownPort: Send + Sync {
    fn shutdown(&self);
}

pub(crate) struct ApplicationLifecycleUsecase<'a, A, W, L> {
    agent_sessions: &'a A,
    workflow_commands: &'a W,
    local_api: &'a L,
}

impl<'a, A, W, L> ApplicationLifecycleUsecase<'a, A, W, L>
where
    A: AgentSessionShutdownPort,
    W: WorkflowCommandShutdownPort,
    L: LocalApiShutdownPort,
{
    pub(crate) fn new(agent_sessions: &'a A, workflow_commands: &'a W, local_api: &'a L) -> Self {
        Self {
            agent_sessions,
            workflow_commands,
            local_api,
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        self.agent_sessions.close_all().await?;
        self.workflow_commands.shutdown_active_commands().await;
        self.local_api.shutdown();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct RecordingShutdown {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait::async_trait]
    impl WorkflowCommandShutdownPort for RecordingShutdown {
        async fn shutdown_active_commands(&self) {
            self.calls.lock().unwrap().push("shutdown_active_commands");
        }
    }

    #[async_trait::async_trait]
    impl AgentSessionShutdownPort for RecordingShutdown {
        async fn close_all(&self) -> Result<(), String> {
            self.calls.lock().unwrap().push("close_all");
            Ok(())
        }
    }

    impl LocalApiShutdownPort for RecordingShutdown {
        fn shutdown(&self) {
            self.calls.lock().unwrap().push("shutdown_local_api");
        }
    }

    struct FailingAgentShutdown {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait::async_trait]
    impl AgentSessionShutdownPort for FailingAgentShutdown {
        async fn close_all(&self) -> Result<(), String> {
            self.calls.lock().unwrap().push("close_all");
            Err("injected close_all failure".to_string())
        }
    }

    #[tokio::test]
    async fn shutdown_orders_agent_finalize_before_irreversible_services() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let shutdown = RecordingShutdown {
            calls: calls.clone(),
        };
        let usecase = ApplicationLifecycleUsecase::new(&shutdown, &shutdown, &shutdown);

        usecase.shutdown().await.unwrap();

        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                "close_all",
                "shutdown_active_commands",
                "shutdown_local_api"
            ]
        );
    }

    #[tokio::test]
    async fn agent_finalize_failure_keeps_irreversible_services_running() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let workflow_and_api = RecordingShutdown {
            calls: calls.clone(),
        };
        let agent = FailingAgentShutdown {
            calls: calls.clone(),
        };
        let usecase =
            ApplicationLifecycleUsecase::new(&agent, &workflow_and_api, &workflow_and_api);

        let error = usecase.shutdown().await.unwrap_err();

        assert_eq!(error, "injected close_all failure");
        assert_eq!(calls.lock().unwrap().as_slice(), ["close_all"]);
    }
}
