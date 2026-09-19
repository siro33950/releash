use super::*;
use crate::adaptor::gateway::client_handoff::ClientHandoffFiles;
#[test]
fn test_送信手順_期限切れは保存せず結果確認後にだけ引継ぎを破棄する() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let files = Arc::new(ClientHandoffFiles::new(dir.path().into()));
    let handoff = Arc::new(ClientHandoffUsecase::new(files.clone(), files));
    let mut service = DesktopClientUsecase::new(handoff.clone());
    let identity = OperationIdentity {
        command: "add_repo_path".into(),
        fingerprint: [1; 32],
        target: None,
    };
    for name in ["get_repo_paths", "get_performance_telemetry_enabled"] {
        service
            .transmit(
                &name.replace('_', "-"),
                OperationIdentity {
                    command: name.into(),
                    fingerprint: [2; 32],
                    target: None,
                },
                0,
                0,
            )
            .unwrap();
        service.responded(&name.replace('_', "-"), true);
    }
    service.finish_restoration().unwrap();
    // When / Then
    assert!(service.transmit("expired", identity.clone(), 1, 1).is_err());
    assert!(handoff.list().unwrap().is_empty());
    service.transmit("edit", identity, 10, 1).unwrap();
    service.responded("edit", true);
    assert_eq!(handoff.list().unwrap().len(), 1);
    service.acknowledge("edit").unwrap();
    assert!(handoff.list().unwrap().is_empty());
}
