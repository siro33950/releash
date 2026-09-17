use super::*;
use crate::usecase::client_handoff::ClientHandoffUsecase;
use std::sync::Arc;

#[test]
fn test_引継ぎ_再起動後も未確定の変更の識別子だけ残し確定時に除去する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let service = {
        let gateway = Arc::new(ClientHandoffFiles::new(directory.path().into()));
        ClientHandoffUsecase::new(gateway.clone(), gateway)
    };
    // When
    for command in ["get_app_settings", "update_external_editor"] {
        service
            .remember(ClientHandoffReference {
                id: command.replace('_', "-"),
                command: command.into(),
                fingerprint: vec![1; 32],
                ordering_target: vec![],
            })
            .unwrap();
    }
    let restored = {
        let gateway = Arc::new(ClientHandoffFiles::new(directory.path().into()));
        ClientHandoffUsecase::new(gateway.clone(), gateway)
    };
    let references = restored.list().unwrap();
    // Then
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].command, "update_external_editor");
    let bytes =
        std::fs::read_to_string(directory.path().join("update-external-editor.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&bytes)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        4
    );
    // When
    restored.forget(&references[0].id).unwrap();
    restored.forget(&references[0].id).unwrap();
    // Then
    assert!(restored.list().unwrap().is_empty());
    assert!(restored.forget("../outside").is_err());
    std::fs::write(directory.path().join("broken.json"), "invalid").unwrap();
    assert!(restored.list().is_err());
}

#[test]
fn test_引継ぎ件数_上限で新規追加を拒否し既存更新と確認済み削除を許す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let gateway = ClientHandoffFiles::new(directory.path().into());
    let mut reference = ClientHandoffReference {
        id: "operation-0".into(),
        command: "update_external_editor".into(),
        fingerprint: vec![1; 32],
        ordering_target: vec![],
    };
    for index in 0..crate::domain::client_operation::policy::MAX_UNACKNOWLEDGED_OPERATIONS {
        reference.id = format!("operation-{index}");
        gateway.remember(&reference).unwrap();
    }
    // When
    let count = gateway.list().unwrap().len();
    reference.id = "overflow".into();
    let overflow = gateway.remember(&reference).unwrap_err();
    // Then
    assert_eq!(
        count,
        crate::domain::client_operation::policy::MAX_UNACKNOWLEDGED_OPERATIONS
    );
    assert_eq!(overflow, "Too many unresolved desktop operations.");
    // When
    reference.id = "operation-0".into();
    reference.fingerprint = vec![2; 32];
    gateway.remember(&reference).unwrap();
    // Then
    assert_eq!(
        gateway
            .list()
            .unwrap()
            .iter()
            .find(|r| r.id == reference.id)
            .unwrap()
            .fingerprint,
        [2; 32]
    );
    // When
    gateway.forget(&reference.id).unwrap();
    reference.id = "replacement".into();
    gateway.remember(&reference).unwrap();
    // Then
    assert_eq!(
        gateway.list().unwrap().len(),
        crate::domain::client_operation::policy::MAX_UNACKNOWLEDGED_OPERATIONS
    );
    // When
    std::fs::copy(
        directory.path().join("replacement.json"),
        directory.path().join("overflow.json"),
    )
    .unwrap();
    // Then
    assert_eq!(
        gateway.list().err().unwrap(),
        "Too many unresolved desktop operations."
    );
}
