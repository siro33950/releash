use super::*;

fn started_detail(nodes: Value) -> String {
    serde_json::json!({
        "root": {
            "workspaceIdentity": "/repo", "worktreePath": "/repo",
            "createdFrom": "desktop_ui", "request": "", "launchedAs": "workflow",
            "workflowName": "example",
            "definition": { "name": "example", "description": "", "entry": "main", "nodes": nodes }
        }
    })
    .to_string()
}

#[test]
fn test_保存定義_一部のnodeだけを解釈せず定義全体を拒否する() {
    // Given
    for nodes in [
        serde_json::json!({"main": {"sequence": {"children": ["build"], "output": "build"}}, "build": {"command": "true"}}),
        serde_json::json!({"main": {"command": "true"}, "unused": {"unrecognized_kind": {}}}),
        serde_json::json!({"main": {"command": "true", "completion": "approval"}}),
        serde_json::json!({"main": {"fanout": {"children": ["build"], "future_field": true}}, "build": {"command": "true"}}),
    ] {
        let detail = started_detail(nodes);
        // When / Then
        assert!(decode_started(&detail).is_err());
        assert!(definition_error(&detail)
            .unwrap()
            .unwrap()
            .contains("Workflow definition is unavailable"));
        let NodeFact::Started(started) = decode_terminal_started(&detail).unwrap() else {
            panic!()
        };
        assert!(started.root.unwrap().definition.is_none());
    }
}

#[test]
fn test_保存定義_全体を解釈できる場合はentryを保持する() {
    // Given
    let detail = started_detail(serde_json::json!({"main": {"command": "true"}}));
    // When
    let NodeFact::Started(started) = decode_started(&detail).unwrap() else {
        panic!()
    };
    // Then
    assert_eq!(started.root.unwrap().definition.unwrap().entry, "main");
    assert!(definition_error(&detail).unwrap().is_none());
}

#[test]
fn test_木の列挙_実行定義を解釈せず所属情報を取得できる() {
    // Given
    let detail = started_detail(Value::String("unsupported definition format".into()));
    // When
    let header = read_tree_header(&detail).unwrap().unwrap();
    // Then
    assert_eq!(header.workspace_identity, "/repo");
    assert_eq!(header.launched_as, ExecutionTreeLaunch::Workflow);
}

#[test]
fn test_保存定義_壊れたroot情報は定義の非互換と混同しない() {
    // Given / When / Then
    assert!(definition_error("{").is_err());
    let detail = started_detail(serde_json::json!({"main": {"command": "true"}}))
        .replace("desktop_ui", "unknown-origin");
    assert!(definition_error(&detail).is_err());
}

#[test]
fn test_終端started_読める定義も読めない定義も本文を復元しない() {
    // Given
    for nodes in [
        serde_json::json!({"main": {"command": "true"}}),
        serde_json::json!({"main": {"completion": "approval"}}),
    ] {
        let detail = started_detail(nodes);
        // When
        let NodeFact::Started(started) = decode_terminal_started(&detail).unwrap() else {
            panic!()
        };
        // Then
        let root = started.root.unwrap();
        assert_eq!(root.workflow_name, "example");
        assert!(root.definition.is_none());
    }
}

#[test]
fn test_終端started_定義本文の名前を読まず開始メタデータだけで復元する() {
    // Given
    for definition in [
        serde_json::json!({"name": "different-name", "entry": "main", "nodes": {"main": {"command": "true"}}}),
        serde_json::json!({"name": 42}),
        serde_json::json!("unsupported definition"),
        Value::Null,
    ] {
        let mut detail: Value = serde_json::from_str(&started_detail(Value::Null)).unwrap();
        detail["root"]["definition"] = definition;
        // When
        let NodeFact::Started(started) = decode_terminal_started(&detail.to_string()).unwrap()
        else {
            panic!()
        };
        // Then
        let root = started.root.unwrap();
        assert_eq!(root.workflow_name, "example");
        assert!(root.definition.is_none());
    }
}

#[test]
fn test_終端started_旧ログの名前を定義本文から補完しない() {
    // Given
    let mut detail: Value = serde_json::from_str(&started_detail(
        serde_json::json!({"main": {"command": "true"}}),
    ))
    .unwrap();
    detail["root"]
        .as_object_mut()
        .unwrap()
        .remove("workflowName");
    // When
    let NodeFact::Started(started) = decode_terminal_started(&detail.to_string()).unwrap() else {
        panic!()
    };
    // Then
    let root = started.root.unwrap();
    assert!(root.workflow_name.is_empty());
    assert!(root.definition.is_none());
    assert!(decode_started(&detail.to_string()).is_ok());
}
