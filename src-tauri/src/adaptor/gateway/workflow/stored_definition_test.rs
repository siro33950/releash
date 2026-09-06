use super::*;

fn started_detail(nodes: Value) -> String {
    serde_json::json!({
        "root": {
            "workspaceIdentity": "/repo", "worktreePath": "/repo",
            "createdFrom": "desktop_ui", "request": "", "launchedAs": "workflow",
            "definition": { "name": "example", "description": "", "entry": "main", "nodes": nodes }
        }
    })
    .to_string()
}

#[test]
fn test_保存定義_未対応のsequenceがあってもcommandとsessionを取得できる() {
    // Given
    let detail = started_detail(serde_json::json!({
        "main": { "sequence": { "children": ["build", "review"], "output": "review" } },
        "build": { "command": "cargo check" },
        "review": { "session": { "provider": "codex" } }
    }));

    // When
    let NodeFact::Started(started) = decode_started(&detail).unwrap() else {
        panic!()
    };
    let root = started.root.unwrap();

    // Then
    assert!(root.definition.node_by_name("main").is_none());
    assert!(root
        .definition
        .node_by_name("build")
        .unwrap()
        .command()
        .is_some());
    assert!(root.definition.node_by_name("review").unwrap().is_session());
    assert!(root.definition_resolution.node_errors["main"].contains("output"));
    assert_eq!(root.definition.entry, "main");
}

#[test]
fn test_保存定義_特定の旧fieldに依存せず未対応項目を局所化する() {
    // Given
    let detail = started_detail(serde_json::json!({
        "main": { "command": "true" },
        "unused": { "unrecognized_kind": {} }
    }));

    // When
    let NodeFact::Started(started) = decode_started(&detail).unwrap() else {
        panic!()
    };
    let root = started.root.unwrap();

    // Then
    assert_eq!(root.definition_resolution.node_errors.len(), 1);
    assert!(root
        .definition_resolution
        .node_error(&root.definition, "main")
        .is_none());
    assert!(NodeFact::decode("started", &detail).is_err());
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
fn test_保存定義_未対応fanoutでも表示上の展開同定情報を保持する() {
    // Given
    let detail = started_detail(serde_json::json!({
        "main": { "fanout": { "children": ["cmd"], "items": "plan.items", "future_field": true } },
        "cmd": { "command": "true" }
    }));

    // When
    let NodeFact::Started(started) = decode_started(&detail).unwrap() else {
        panic!()
    };
    let root = started.root.unwrap();

    // Then
    assert!(root.definition.node_by_name("main").is_none());
    assert_eq!(
        root.definition_resolution.dynamic_fanout_names,
        std::collections::BTreeSet::from(["main".into()])
    );
}
