use releashd::test_support::integration::workflow::{
    list_builtin_workflows, ExecutionStatus, WorkflowDefinitionFileQueryService,
    WorkflowDefinitionQueryService, WorkflowExecutionSummary, WorkflowSourceFormat,
};

fn running(name: &str) -> WorkflowExecutionSummary {
    WorkflowExecutionSummary {
        execution_id: "00000000-0000-4000-8000-000000000001".into(),
        workflow_name: name.into(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".into(),
        current_node: None,
        started_at: 1.0,
        updated_at: 1.0,
        completed_at: None,
        error_reason: None,
        total_token_usage: Default::default(),
    }
}

fn yaml(name: &str) -> String {
    format!("name: {name}\ndescription: custom description\nnodes:\n  main:\n    command: 'true'\n")
}

#[test]
fn test_定義一覧_宣言名で実行中を判定しbuiltinと利用者を名前順に返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let builtin = list_builtin_workflows()[0].name.clone();
    std::fs::write(
        dir.path().join("different-filename.yml"),
        yaml("declared-name"),
    )
    .unwrap();
    std::fs::write(dir.path().join(format!("{builtin}.yml")), yaml(&builtin)).unwrap();
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());
    // When
    let items = query.list_workflows(&[running("declared-name")]).unwrap();
    // Then
    assert!(items.windows(2).all(|pair| pair[0].name < pair[1].name));
    let custom = items
        .iter()
        .find(|item| item.name == "different-filename")
        .unwrap();
    assert!(custom.is_running);
    assert!(!custom.builtin);
    assert_eq!(custom.description, "custom description");
    assert_eq!(custom.source_format, WorkflowSourceFormat::Yaml);
    assert!(custom.failure.is_none());
    let overridden = items.iter().find(|item| item.name == builtin).unwrap();
    assert!(!overridden.builtin);
    assert_eq!(overridden.description, "custom description");
    assert!(items.iter().any(|item| item.builtin));
}

#[test]
fn test_定義一覧_同じstemの後続定義が実行中なら統合後も実行中になる() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("same.lua"),
        r#"
local r = require("releash")
return r.workflow{
  name = "same", description = "Lua workflow",
  main = r.command{ command = "true" },
}
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("same.yml"), yaml("target")).unwrap();
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());

    // When
    let items = query.list_workflows(&[running("target")]).unwrap();

    // Then
    let merged = items
        .iter()
        .filter(|item| item.name == "same")
        .collect::<Vec<_>>();
    assert_eq!(merged.len(), 1);
    assert!(merged[0].failure.is_none(), "{:?}", merged[0].failure);
    assert!(merged[0].is_running);
}
