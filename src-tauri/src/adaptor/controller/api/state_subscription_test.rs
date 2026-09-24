use super::*;

#[test]
fn test_購読事象_対象名と日本語を含む引数を分離して配信する() {
    use crate::domain::state_subscription::{SubscriptionTarget, Version};
    // Given
    let target = SubscriptionTarget::BranchBase("/作業:repo".into(), "feature".into());
    let value = StateValue::BranchBase(Some("main".into()));
    // When
    let message = event(StateSubscriptionEvent::Item(
        target.to_string(),
        Event::Snapshot(
            Version {
                epoch: "boot".into(),
                sequence: 1,
            },
            std::sync::Arc::new(value),
        ),
    ))
    .unwrap();
    let message: wire::StateSubscriptionEvent =
        crate::adaptor::protocol::connect::to_wire(&message).unwrap();
    // Then
    assert_eq!(message.target, "branch-base");
    assert_eq!(message.args, ["/作業:repo", "feature"]);
    assert_eq!(
        SubscriptionTarget::from_parts(
            &message.target,
            &message.args.iter().map(String::as_str).collect::<Vec<_>>()
        ),
        Ok(target)
    );
    assert!(matches!(
        message.event,
        Some(wire::state_subscription_event::Event::Snapshot(_))
    ));
}
