use super::*;
use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
use crate::usecase::state_subscription::StateSubscriptionUsecase;
use crate::usecase::state_subscription::StateValue;
use crate::{
    infrastructure::state_subscription::Event, usecase::state_subscription::SubscriptionTarget,
};
use futures_util::StreamExt;
use std::sync::Arc;
#[tokio::test]
async fn test_一覧通知_購読へ一覧を配信する() {
    // Given
    let subscriptions = StateSubscriptionUsecase::new(
        Vec::new(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let notifier = RepoPathsNotifyGateway::new(subscriptions.publisher());
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    crate::test_support::state_subscription::start(
        &subscriptions,
        "client",
        &SubscriptionTarget::RepositoryPaths.to_string(),
        None,
    )
    .unwrap();
    for _ in 0..3 {
        stream.next().await.unwrap();
    }
    // When
    notifier.notify_changed(vec!["/repo".into()]);
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if crate::test_support::state_subscription::same(&value, &StateValue::RepositoryPaths(vec!["/repo".into()])))
    );
}
