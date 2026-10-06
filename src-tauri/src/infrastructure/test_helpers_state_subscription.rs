use crate::usecase::state_subscription::{
    StateReadError, StateSubscriptionUsecase, SubscriptionError, SubscriptionTarget,
};
pub async fn start_read(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    raw: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), StateReadError> {
    let target = SubscriptionTarget::parse(raw).map_err(StateReadError::from_error)?;
    usecase
        .deps()
        .start_subscription(client, &target, &format!("{client}:{raw}"), cursor)
        .await
}
pub async fn stop_read(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    raw: &str,
) -> Result<(), SubscriptionError> {
    usecase
        .deps()
        .stop_subscription(&format!("{client}:{raw}"))
        .await
}
