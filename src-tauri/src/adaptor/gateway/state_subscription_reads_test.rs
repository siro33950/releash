use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use crate::common::operation_context::{scope, Deadline, OperationContext};
use connectrpc::ErrorCode;

#[tokio::test]
async fn test_購読読取の境界_同期queryへ期限を引き継ぐ() {
    let fixture = crate::test_support::state_subscription::StateReadsFixture::new();
    let reads = StateSubscriptionReads(fixture.reads.clone());
    let target = SubscriptionTarget::RepositoryRoot(fixture.path.clone());
    assert!(reads.read(&target).await.is_ok());
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    let error = scope(context, reads.read(&target)).await.unwrap_err();
    assert_eq!(error.connect_code(), ErrorCode::DeadlineExceeded);
    assert!(reads.read(&target).await.is_ok());
}

#[tokio::test]
async fn test_購読読取の境界_非同期queryと外部更新不要の要求も同じ結果を返す() {
    let fixture = crate::test_support::state_subscription::StateReadsFixture::new();
    let reads = StateSubscriptionReads(fixture.reads.clone());
    let target = SubscriptionTarget::ProviderHookHealth;
    let expected = fixture.reads.read(&target).await.unwrap();
    assert_eq!(reads.read(&target).await.unwrap(), expected);
    assert!(reads.refresh_external(&target).await.is_ok());
}

#[tokio::test]
async fn test_購読読取の境界_review対象とcomment置き場を内側へ委譲する() {
    let fixture = crate::test_support::state_subscription::StateReadsFixture::new();
    let reads = StateSubscriptionReads(fixture.reads.clone());
    let target = SubscriptionTarget::ReviewThreads("repository".into());
    let expected = fixture.reads.read(&target).await.unwrap();
    assert_eq!(reads.read(&target).await.unwrap(), expected);
    assert_eq!(
        reads.review_comments_dir(),
        fixture.reads.review_comments_dir()
    );
    assert!(!reads.review_comments_dir().is_empty());
}
