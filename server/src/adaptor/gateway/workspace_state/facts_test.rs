use super::*;

#[test]
fn test_ui状態の事実_保存形式をcodec経由で往復し未知の版と破損を拒む() {
    // Given
    let stored: StoredWorkspaceState = serde_json::from_value(serde_json::json!({
        "version": 1, "tabs": {"editors": [], "activeEditorPath": null},
        "layout": {"centerTab": "agent", "activeView": "git", "leftNavCollapsed": true, "rightCollapsed": false, "rightBottomCollapsed": false},
        "panes": {"kind": "pane", "id": "pane", "tabs": [{"id": "session", "kind": "session"}], "active_tab": "session"}
    })).unwrap();
    let registry = EventCodecRegistry::new();
    for event in [
        LocalDomainEvent::WorkspaceStateChanged(Box::new(stored.try_into().unwrap())),
        LocalDomainEvent::RepositoryGroupChanged(true),
        LocalDomainEvent::RepositoryGroupChanged(false),
        LocalDomainEvent::WorkspaceStateInvalidated,
    ] {
        // When
        let encoded = registry.encode(&event).unwrap();
        // Then
        assert_eq!(
            registry
                .decode(
                    &encoded.event_type,
                    encoded.payload_version,
                    &encoded.payload
                )
                .unwrap(),
            DecodedStoredEvent::Known(Box::new(event))
        );
        assert_eq!(
            registry
                .decode(&encoded.event_type, 2, &encoded.payload)
                .unwrap(),
            DecodedStoredEvent::Unknown
        );
        assert!(registry.decode(&encoded.event_type, 1, b"broken").is_err());
    }
}

#[test]
fn test_ui状態の事実_不正な配置と別種の事実を受理しない() {
    // Given
    let codec = UIStateCodec { group: false };
    let raw = CborValue::Text(serde_json::json!({
        "version": 1, "tabs": {"editors": [], "activeEditorPath": null},
        "layout": {"centerTab": "agent", "activeView": "git", "leftNavCollapsed": false, "rightCollapsed": false, "rightBottomCollapsed": false},
        "panes": {"kind": "pane", "id": "pane", "tabs": [], "active_tab": "missing"}
    }).to_string());
    // When / Then
    assert!(codec.decode(1, &raw).is_err());
    assert!(codec
        .encode(&LocalDomainEvent::RepositoryGroupChanged(true))
        .is_err());
    assert!(UIStateCodec { group: true }.decode(1, &raw).is_err());
}

#[test]
fn test_ui状態の無効化の事実_別種の事実と不正なpayloadを拒否する() {
    // Given
    let codec = WorkspaceStateInvalidationCodec;
    // When / Then
    assert!(codec
        .encode(&LocalDomainEvent::RepositoryGroupChanged(false))
        .is_err());
    assert!(codec.decode(1, &CborValue::Text("invalid".into())).is_err());
}
