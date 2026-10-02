use crate::usecase::agent_session::LaunchRetention;

pub(crate) const RETENTION: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) fn run(
    delay: crate::infrastructure::timer::Delay,
) -> tokio::sync::mpsc::UnboundedSender<LaunchRetention> {
    let (sender, mut activated) = tokio::sync::mpsc::unbounded_channel::<LaunchRetention>();
    tokio::spawn(async move {
        while let Some(request) = activated.recv().await {
            let elapsed = delay();
            tokio::spawn(async move {
                elapsed.await;
                request.expire().await;
            });
        }
    });
    sender
}

#[cfg(test)]
#[path = "agent_session_launch_retention_test.rs"]
mod agent_session_launch_retention_tests;
