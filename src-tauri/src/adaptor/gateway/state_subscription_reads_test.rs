use super::*;
use crate::common::operation_context::{scope, Deadline, OperationContext};
use crate::domain::failure::FailureKind;

#[tokio::test]
async fn test_購読読取の境界_同期queryへ期限を引き継ぐ() {
    let fixture = crate::usecase::state_subscription::StateReadsFixture::new();
    let reads = StateSubscriptionReads(fixture.reads.clone());
    let target = SubscriptionTarget::RepositoryRoot(fixture.path.clone());
    assert!(reads.read(&target).await.is_ok());
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    let error = scope(context, reads.read(&target)).await.unwrap_err();
    assert_eq!(error.kind, FailureKind::Expired);
    assert!(reads.read(&target).await.is_ok());
}

#[tokio::test]
async fn test_購読読取の境界_非同期queryと外部更新不要の要求も同じ結果を返す() {
    let fixture = crate::usecase::state_subscription::StateReadsFixture::new();
    let reads = StateSubscriptionReads(fixture.reads.clone());
    let target = SubscriptionTarget::Failures("target".into(), 0);
    let expected = fixture.reads.read(&target).await.unwrap();
    assert_eq!(reads.read(&target).await.unwrap(), expected);
    assert!(reads.refresh_external(&target).await.is_ok());
}
