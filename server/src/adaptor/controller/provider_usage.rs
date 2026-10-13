use futures_util::{Stream, StreamExt};
use std::sync::Arc;

pub(crate) async fn run(
    usage: Arc<crate::usecase::provider_usage::ProviderUsageUsecase>,
    lifecycle: Arc<crate::usecase::provider_lifecycle::ProviderLifecycleUsecase>,
    mut ticks: impl Stream<Item = ()> + Unpin,
) {
    while ticks.next().await.is_some() {
        match usage.poll_active_codex(lifecycle.as_ref()).await {
            Ok(failures) => {
                for error in failures {
                    log::warn!("Provider usage: {error}");
                }
            }
            Err(error) => log::warn!("Provider usage: {error}"),
        }
    }
}
