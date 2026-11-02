use releashd::test_support::integration::workflow::{
    list_builtin_workflows, ExecutionOrigin, ExecutionStatus, WorkflowDefinitionFileQueryService,
    WorkflowDefinitionFileRepository, WorkflowDefinitionQueryService, WorkflowDefinitionRepository,
    WorkflowExecutionSummary, WorkflowSourceFormat,
};

fn running(name: &str) -> WorkflowExecutionSummary {
    WorkflowExecutionSummary {
        execution_id: "00000000-0000-4000-8000-000000000001".into(),
        workflow_name: name.into(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".into(),
        current_node: None,
        created_from: ExecutionOrigin::Cli,
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
fn test_保存先一覧_利用者とbuiltinの名前を重複なく返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let builtin = list_builtin_workflows()[0].name.clone();
    std::fs::write(dir.path().join(format!("{builtin}.yml")), yaml(&builtin)).unwrap();
    std::fs::write(dir.path().join("custom.lua"), "invalid lua").unwrap();
    std::fs::write(dir.path().join("custom.yml"), "invalid yaml").unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    // When
    let names = repository.list_names().unwrap();
    // Then
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(names.contains(&"custom".to_string()));
    assert!(names.contains(&builtin));
    assert_eq!(names.len(), list_builtin_workflows().len() + 1);
}
