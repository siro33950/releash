use super::*;
use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    DecodedStoredEvent, EventCodecRegistry, LocalEventPayloadCodec,
};
use crate::domain::local_event::LocalDomainEvent;

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
        let legacy_payload = match &event {
            LocalDomainEvent::WorkspaceStateChanged(state) => CborValue::Text(
                serde_json::to_string(&StoredWorkspaceState::from(state.as_ref())).unwrap(),
            ),
            LocalDomainEvent::RepositoryGroupChanged(collapsed) => {
                CborValue::Text(collapsed.to_string())
            }
            LocalDomainEvent::WorkspaceStateInvalidated => CborValue::Null,
            _ => unreachable!(),
        };
        let legacy_bytes =
            crate::adaptor::gateway::local_event_store::canonical_cbor::encode_canonical(
                &legacy_payload,
            )
            .unwrap();
        assert_eq!(encoded.payload, legacy_bytes);
        assert_eq!(
            registry
                .decode(&encoded.event_type, 1, &legacy_bytes)
                .unwrap(),
            DecodedStoredEvent::Known(Box::new(event.clone()))
        );
        assert_eq!(
            registry
                .decode(
                    &encoded.event_type,
                    encoded.payload_version,
                    &encoded.payload
                )
                .unwrap(),
            DecodedStoredEvent::Known(Box::new(event.clone()))
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
    let codec = WorkspaceStateCodec;
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
    assert!(RepositoryGroupCodec.decode(1, &raw).is_err());
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

#[test]
fn test_pane形式_既存jsonと全種類と分割軸をdomainとの往復で保つ() {
    // Given
    for axis in ["horizontal", "vertical"] {
        let json = format!(
            r#"{{"kind":"split","id":"root","axis":"{axis}","ratio":0.7,"first":{{"kind":"pane","id":"a","tabs":[{{"id":"terminal","kind":"terminal"}}],"active_tab":"terminal"}},"second":{{"kind":"pane","id":"b","tabs":[{{"id":"workflow","kind":"workflow"}},{{"id":"file","kind":"file"}},{{"id":"session","kind":"session"}}],"active_tab":"file"}}}}"#
        );
        // When
        let dto: StoredPaneLayout = serde_json::from_str(&json).unwrap();
        let domain: crate::domain::workspace_state::value_objects::pane_layout::PaneLayout =
            dto.clone().try_into().unwrap();
        // Then
        assert_eq!(StoredPaneLayout::from(&domain), dto);
        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
    }
}

#[test]
fn test_workspace保存形式_panesのない旧jsonと現在のjsonを直接往復する() {
    // Given
    let legacy = serde_json::json!({
        "version": 1, "tabs": {"editors": [{"path":"file", "name":"File"}], "activeEditorPath":"file"},
        "layout": {"centerTab":"agent", "activeView":"git", "leftNavCollapsed":true, "rightCollapsed":false, "rightBottomCollapsed":false}
    });
    for panes in [
        None,
        Some(serde_json::json!({"kind":"pane", "id":"root", "tabs":[], "active_tab":null})),
    ] {
        let mut json = legacy.clone();
        if let Some(panes) = &panes {
            json["panes"] = panes.clone();
        }
        // When
        let stored: StoredWorkspaceState = serde_json::from_value(json.clone()).unwrap();
        let domain: WorkspaceState = stored.try_into().unwrap();
        // Then
        assert_eq!(domain.panes.is_some(), panes.is_some());
        assert_eq!(domain.tabs.editors[0].path, "file");
        assert!(domain.layout.left_nav_collapsed);
        assert_eq!(
            serde_json::to_value(StoredWorkspaceState::from(&domain)).unwrap(),
            json
        );
    }
}
