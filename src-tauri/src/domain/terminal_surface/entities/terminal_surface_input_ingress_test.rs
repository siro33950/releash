use super::{
    TerminalSurfaceInput, TerminalSurfaceInputIngressError, TerminalSurfaceInputIngressRegistry,
};

fn input(sequence: u64, data: &str) -> TerminalSurfaceInput {
    TerminalSurfaceInput {
        sequence,
        data: data.to_string(),
    }
}

#[test]
fn test_ターミナル入力受付_順序乱れ入力を連番順で払い出す() {
    let mut registry = TerminalSurfaceInputIngressRegistry::with_pending_capacity(8);
    registry.activate("surface-a", "attachment-a");

    assert_eq!(
        registry.admit("surface-a", "attachment-a", 1, "second".to_string()),
        Ok(Vec::new())
    );
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 0, "first".to_string()),
        Ok(vec![input(0, "first"), input(1, "second")])
    );
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 2, "third".to_string()),
        Ok(vec![input(2, "third")])
    );
}

#[test]
fn test_ターミナル入力受付_重複入力と旧attachment入力を書き込まない() {
    let mut registry = TerminalSurfaceInputIngressRegistry::with_pending_capacity(8);
    registry.activate("surface-a", "attachment-a");
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 0, "first".to_string()),
        Ok(vec![input(0, "first")])
    );
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 0, "duplicate".to_string()),
        Ok(Vec::new())
    );

    registry.activate("surface-a", "attachment-b");
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 1, "stale".to_string()),
        Err(TerminalSurfaceInputIngressError::StaleAttachment)
    );
}

#[test]
fn test_ターミナル入力受付_保留入力の上限超過を拒否する() {
    let mut registry = TerminalSurfaceInputIngressRegistry::with_pending_capacity(2);
    registry.activate("surface-a", "attachment-a");

    assert_eq!(
        registry.admit("surface-a", "attachment-a", 2, "third".to_string()),
        Ok(Vec::new())
    );
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 1, "second".to_string()),
        Ok(Vec::new())
    );
    assert_eq!(
        registry.admit("surface-a", "attachment-a", 3, "fourth".to_string()),
        Err(TerminalSurfaceInputIngressError::PendingCapacityExceeded)
    );
}

#[test]
fn test_ターミナル入力受付_書込失敗分を新しい入力より先に再払い出しする() {
    let mut registry = TerminalSurfaceInputIngressRegistry::with_pending_capacity(8);
    registry.activate("surface-a", "attachment-a");
    let ready = registry
        .admit("surface-a", "attachment-a", 0, "first".to_string())
        .unwrap();
    registry
        .restore_failed("surface-a", "attachment-a", ready)
        .unwrap();

    assert_eq!(
        registry.admit("surface-a", "attachment-a", 1, "second".to_string()),
        Ok(vec![input(0, "first"), input(1, "second")])
    );
}

#[test]
fn test_ターミナル入力受付_失敗通知は次の成功まで一度だけ発火する() {
    let mut registry = TerminalSurfaceInputIngressRegistry::with_pending_capacity(8);
    registry.activate("surface-a", "attachment-a");

    assert!(registry.record_failure("surface-a", "attachment-a"));
    assert!(!registry.record_failure("surface-a", "attachment-a"));
    registry.record_success("surface-a", "attachment-a");
    assert!(registry.record_failure("surface-a", "attachment-a"));
}
