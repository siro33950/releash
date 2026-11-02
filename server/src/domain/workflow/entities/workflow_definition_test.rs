use super::*;
use crate::domain::failure::WorkFailure;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatus, TokenUsage, WorkflowExecutionSummary,
};

fn content(name: &str) -> WorkflowDefinitionContent {
    WorkflowDefinitionContent {
        name: name.into(),
        description: "description".into(),
        ..Default::default()
    }
}

fn definition(
    name: &str,
    source: WorkflowSource,
    format: WorkflowSourceFormat,
    content: Result<WorkflowDefinitionContent, WorkflowError>,
) -> WorkflowDefinition {
    WorkflowDefinition::restore(name.into(), "/workflows".into(), source, format, content)
}

fn file(name: &str) -> WorkflowDefinition {
    definition(
        name,
        WorkflowSource::File {
            path: format!("/actual/{name}.yml").into(),
        },
        WorkflowSourceFormat::Yaml,
        Ok(content(name)),
    )
}

fn builtin() -> WorkflowDefinition {
    definition(
        "03_full-review",
        WorkflowSource::Builtin,
        WorkflowSourceFormat::Yaml,
        Ok(content("03_full-review")),
    )
}

fn validation<T: std::fmt::Debug>(result: Result<T, WorkflowError>, reason: &str) {
    assert_eq!(result.unwrap_err(), WorkflowError::validation(reason));
}

#[test]
fn test_新規定義_名前を検証し利用者の保存先を作る() {
    // Given / When
    let definition = WorkflowDefinition::new("/workflows".into(), content("new-name")).unwrap();
    // Then
    assert_eq!(definition.file_stem(), "new-name");
    assert_eq!(definition.format(), WorkflowSourceFormat::Yaml);
    assert_eq!(definition.content(), Ok(&content("new-name")));
    assert_eq!(
        definition.source(),
        &WorkflowSource::File {
            path: "/workflows/new-name.yml".into()
        }
    );
    assert!(matches!(
        WorkflowDefinition::new("/workflows".into(), content("../bad")),
        Err(WorkflowError::Validation(_))
    ));
}

#[test]
fn test_組み込み定義_保存と削除と外部編集を理由付きで拒否する() {
    // Given
    let builtin = builtin();
    // When / Then
    validation(
        builtin.validate_save(None, &[]),
        "ビルトインワークフローは編集できません",
    );
    validation(
        file("03_full-review").validate_save(Some(&builtin), &[]),
        "ビルトインワークフローは編集できません",
    );
    validation(
        builtin.ensure_deletable(),
        "ビルトインワークフローは削除できません",
    );
    validation(
        builtin.file_path(),
        "ビルトインワークフローは外部エディタで開けません",
    );
    validation(
        builtin.editor_paths(),
        "ビルトインワークフローは外部エディタで開けません",
    );
    assert!(builtin.summary(&[]).builtin);
}

#[test]
fn test_利用者定義_組み込みと同名でも更新と削除と外部編集ができる() {
    // Given
    let original = file("03_full-review");
    let existing = [builtin().summary(&[])];
    // When / Then
    assert_eq!(original.validate_save(Some(&original), &existing), Ok(()));
    assert_eq!(original.ensure_deletable(), Ok(()));
    assert_eq!(
        original.file_path().unwrap(),
        Path::new("/actual/03_full-review.yml")
    );
    assert_eq!(
        original.editor_paths().unwrap(),
        [
            PathBuf::from("/workflows"),
            PathBuf::from("/actual/03_full-review.yml")
        ]
    );
    assert!(!original.summary(&[]).builtin);
}

#[test]
fn test_保存検証_新規と改名のときだけ同名を拒否する() {
    // Given
    let definition = file("03_full-review");
    let existing = [builtin().summary(&[])];
    let original = file("old");
    // When / Then
    validation(
        definition.validate_save(None, &existing),
        "ワークフロー '03_full-review' は既に存在します",
    );
    validation(
        definition.validate_save(Some(&original), &existing),
        "ワークフロー '03_full-review' は既に存在します",
    );
    assert_eq!(definition.validate_save(None, &[]), Ok(()));
    assert_eq!(definition.validate_save(Some(&original), &[]), Ok(()));
    assert_eq!(definition.obsolete_path(None), None);
    assert_eq!(definition.obsolete_path(Some("03_full-review")), None);
    assert_eq!(
        definition.obsolete_path(Some("old")),
        Some("/workflows/old.yml".into())
    );
    let invalid = file("../bad");
    assert!(matches!(
        invalid.validate_save(Some(&invalid), &[]),
        Err(WorkflowError::Validation(_))
    ));
}

#[test]
fn test_保存と複製_luaと解釈失敗を拒否する() {
    // Given
    let lua = definition(
        "03_full-review",
        WorkflowSource::File {
            path: "/workflows/03_full-review.lua".into(),
        },
        WorkflowSourceFormat::Lua,
        Ok(content("03_full-review")),
    );
    let error = WorkflowError::validation("workflow_diagnostics: WFS001: invalid");
    let invalid = definition(
        "broken",
        WorkflowSource::File {
            path: "/workflows/broken.yml".into(),
        },
        WorkflowSourceFormat::Yaml,
        Err(error.clone()),
    );
    // When / Then
    validation(
        file("03_full-review").validate_save(Some(&lua), &[]),
        "Lua workflow は保存できません。外部エディタで編集してください",
    );
    validation(
        lua.duplicate("copy"),
        "Lua workflow は複製できません。外部エディタで編集してください",
    );
    assert_eq!(invalid.content(), Err(&error));
    assert_eq!(invalid.duplicate("copy"), Err(error.clone()));
    assert_eq!(invalid.validate_save(None, &[]), Err(error));
}

#[test]
fn test_定義複製_名前を検証し組み込みから利用者定義を作る() {
    // Given
    let original = builtin();
    // When
    let duplicate = original.duplicate("copy").unwrap();
    // Then
    assert_eq!(duplicate.file_stem(), "copy");
    assert_eq!(duplicate.format(), WorkflowSourceFormat::Yaml);
    assert_eq!(duplicate.content(), Ok(&content("copy")));
    assert_eq!(
        duplicate.file_path().unwrap(),
        Path::new("/workflows/copy.yml")
    );
    assert!(!duplicate.summary(&[]).builtin);
    assert_eq!(original.content().unwrap().name, "03_full-review");
    assert!(matches!(
        original.duplicate("../bad"),
        Err(WorkflowError::Validation(_))
    ));
}

#[test]
fn test_定義概要_ファイル名と取得元と実行中の状態を使う() {
    // Given
    let definition = definition(
        "file-name",
        WorkflowSource::File {
            path: "/workflows/file-name.lua".into(),
        },
        WorkflowSourceFormat::Lua,
        Ok(content("internal-name")),
    );
    let running = WorkflowExecutionSummary {
        execution_id: "execution".into(),
        workflow_name: "file-name".into(),
        status: ExecutionStatus::Running,
        worktree_path: String::new(),
        current_node: None,
        created_from: ExecutionOrigin::Cli,
        started_at: 0.0,
        updated_at: 0.0,
        completed_at: None,
        error_reason: None,
        total_token_usage: TokenUsage::default(),
    };
    // When
    let summary = definition.summary(&[running]);
    // Then
    assert_eq!(summary.name, "file-name");
    assert_eq!(summary.description, "description");
    assert_eq!(summary.failure, None);
    assert_eq!(summary.source_format, WorkflowSourceFormat::Lua);
    assert!(summary.is_running);
    assert!(!summary.builtin);
    assert!(!definition.summary(&[]).is_running);
}

#[test]
fn test_定義概要_診断失敗だけに説明を付け失敗を保持する() {
    // Given / When / Then
    for (error, description) in [
        (
            WorkflowError::validation("workflow_diagnostics: WFS001: invalid"),
            "Invalid workflow definition",
        ),
        (WorkflowError::validation("YAMLパース失敗: invalid"), ""),
        (WorkflowError::external("I/O error"), ""),
    ] {
        let definition = definition(
            "broken",
            WorkflowSource::File {
                path: "/workflows/broken.yml".into(),
            },
            WorkflowSourceFormat::Yaml,
            Err(error.clone()),
        );
        let summary = definition.summary(&[]);
        assert_eq!(summary.name, "broken");
        assert_eq!(summary.description, description);
        assert_eq!(summary.failure, Some(WorkFailure::from_error(&error)));
    }
}
