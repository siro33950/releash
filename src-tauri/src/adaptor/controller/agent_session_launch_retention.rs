use crate::usecase::agent_session::AgentSessionLaunchUsecase;
use futures_util::StreamExt;
use std::sync::Arc;

pub(crate) const RETENTION: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) async fn run(
    usecase: Arc<AgentSessionLaunchUsecase>,
    mut activated: tokio::sync::mpsc::UnboundedReceiver<String>,
    delay: crate::infrastructure::timer::Delay,
) {
    while let Some(session) = activated.recv().await {
        let usecase = usecase.clone();
        let mut elapsed = delay();
        tokio::spawn(async move {
            if elapsed.next().await.is_some() {
                usecase.expire_workflow_launch(&session).await;
            }
        });
    }
}
