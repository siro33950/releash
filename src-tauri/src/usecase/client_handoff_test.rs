use super::*;

#[derive(Default)]
struct Handoff {
    calls: parking_lot::Mutex<Vec<String>>,
    failure: bool,
}
impl ClientHandoffRepository for Handoff {
    fn remember(&self, reference: &ClientHandoffReference) -> Result<(), String> {
        self.calls.lock().push(reference.command.clone());
        if self.failure {
            Err("write failed".into())
        } else {
            Ok(())
        }
    }
    fn forget(&self, id: &str) -> Result<(), String> {
        self.calls.lock().push(id.into());
        if self.failure {
            Err("delete failed".into())
        } else {
            Ok(())
        }
    }
}
impl ClientHandoffQueryService for Handoff {
    fn list(&self) -> Result<Vec<ClientHandoffSummary>, String> {
        if self.failure {
            Err("read failed".into())
        } else {
            Ok(vec![])
        }
    }
}

#[test]
fn test_引継ぎ_検証済みの変更だけ保存し不正な参照は渡さない() {
    // Given
    let gateway = Arc::new(Handoff::default());
    let service = ClientHandoffUsecase::new(gateway.clone(), gateway.clone());
    // When
    for (command, id, valid) in [
        ("get_app_settings", "read", true),
        ("update_external_editor", "write", true),
        ("update_external_editor", "../invalid", false),
    ] {
        assert_eq!(
            service
                .remember(ClientHandoffReference {
                    id: id.into(),
                    command: command.into(),
                    fingerprint: vec![1; 32],
                    ordering_target: vec![],
                })
                .is_ok(),
            valid
        );
    }
    assert!(service.forget("../invalid").is_err());
    service.forget("write").unwrap();
    // Then
    assert_eq!(*gateway.calls.lock(), ["update_external_editor", "write"]);
    assert!(service.list().unwrap().is_empty());
}

#[test]
fn test_引継ぎ_保存と削除と読取りの失敗を呼び出し元へ返す() {
    // Given
    let gateway = Arc::new(Handoff {
        failure: true,
        ..Default::default()
    });
    let service = ClientHandoffUsecase::new(gateway.clone(), gateway);
    // When / Then
    assert_eq!(
        service
            .remember(ClientHandoffReference {
                id: "write".into(),
                command: "update_external_editor".into(),
                fingerprint: vec![1; 32],
                ordering_target: vec![],
            })
            .unwrap_err()
            .to_string(),
        "write failed"
    );
    assert_eq!(
        service.forget("write").unwrap_err().to_string(),
        "delete failed"
    );
    assert_eq!(service.list().err().unwrap().to_string(), "read failed");
}
