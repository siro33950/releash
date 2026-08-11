use super::{StandaloneLaunchRequestRegistry, COMPLETED_STANDALONE_LAUNCH_CAPACITY};

#[test]
fn test_冪等レジストリ_完了記録が容量を超えると最古のrequest_idを追い出す() {
    let mut registry = StandaloneLaunchRequestRegistry::default();
    for index in 0..=COMPLETED_STANDALONE_LAUNCH_CAPACITY {
        registry.record_completed(format!("request-{index}"), Ok(format!("agent-{index}")));
    }

    assert_eq!(registry.recall_completed("request-0"), None);
    assert_eq!(
        registry.recall_completed("request-1"),
        Some(Ok("agent-1".to_string()))
    );
    assert_eq!(
        registry.recall_completed(&format!("request-{COMPLETED_STANDALONE_LAUNCH_CAPACITY}")),
        Some(Ok(format!("agent-{COMPLETED_STANDALONE_LAUNCH_CAPACITY}")))
    );
}

#[test]
fn test_冪等レジストリ_recall済みrequest_idは追い出し順が更新され残存する() {
    let mut registry = StandaloneLaunchRequestRegistry::default();
    for index in 0..COMPLETED_STANDALONE_LAUNCH_CAPACITY {
        registry.record_completed(format!("request-{index}"), Ok(format!("agent-{index}")));
    }
    assert!(registry.recall_completed("request-0").is_some());

    registry.record_completed("request-new".to_string(), Ok("agent-new".to_string()));

    assert_eq!(
        registry.recall_completed("request-0"),
        Some(Ok("agent-0".to_string()))
    );
    assert_eq!(registry.recall_completed("request-1"), None);
    assert_eq!(
        registry.recall_completed("request-new"),
        Some(Ok("agent-new".to_string()))
    );
}
