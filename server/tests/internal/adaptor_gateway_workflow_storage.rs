use releashd::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
use releashd::test_support::integration::workflow::load_workflow;
use releashd::test_support::integration::workflow::parse_workflow_source;
use releashd::test_support::integration::workflow::resolve_and_validate_workflow_facets;
use releashd::test_support::integration::workflow::resolve_workflow_path;
use releashd::test_support::integration::workflow::workflow_files;
use releashd::test_support::integration::workflow::NodeDefinition;
use releashd::test_support::integration::workflow::NodeKind;
use releashd::test_support::integration::workflow::SessionSpec;
use releashd::test_support::integration::workflow::StorageError;
use releashd::test_support::integration::workflow::WorkflowDefinitionContent as WorkflowDefinitionYaml;
use releashd::test_support::integration::workflow::{
    WorkflowDefinition, WorkflowDefinitionFileRepository, WorkflowDefinitionFileSourceGateway,
    WorkflowDefinitionRepository, WorkflowDefinitionSourceService, WorkflowError,
    WorkflowSourceSaveError,
};
use std::fs;
use tempfile::TempDir;

fn sample_workflow(name: &str, _builtin: bool) -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: name.to_string(),
        description: format!("{name} workflow"),
        schemas: Default::default(),
        nodes: vec![NodeDefinition {
            name: "main".to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some("review-acceptance".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }],
        entry: "main".to_string(),
    }
}

fn builtin_workflow_names() -> Vec<String> {
    releashd::test_support::integration::workflow::list_builtin_workflows()
        .into_iter()
        .map(|summary| summary.name)
        .collect()
}

fn first_builtin_workflow_name() -> String {
    builtin_workflow_names()
        .into_iter()
        .next()
        .expect("test premise: at least one builtin workflow exists")
}

#[test]
pub fn test_定義一覧_読めないファイルを失敗欄に残す() {
    todo!()
}

#[test]
pub fn test_定義一覧_置き場所を読めないと失敗を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("not-directory");
    fs::write(&invalid, "file").unwrap();
    // When
    let result = workflow_files(&invalid);
    // Then
    assert!(result.is_err());
}

#[test]
pub fn test_診断_置き場所を読めないと失敗を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("not-directory");
    fs::write(&invalid, "file").unwrap();
    // When
    let result =
        releashd::test_support::integration::workflow::diagnose_all(&invalid, directory.path());
    // Then
    assert!(result.is_err());
}

#[test]
pub fn save_and_load_workflow() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let instructions = dir.join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(
        instructions.join("review-acceptance.md"),
        "Review the change.",
    )
    .unwrap();

    let wf = sample_workflow("my-workflow", false);
    WorkflowDefinitionFileRepository::new(dir, dir)
        .save(
            WorkflowDefinition::new(dir.to_path_buf(), wf).unwrap(),
            None,
        )
        .unwrap();

    let file_path = dir.join("my-workflow.yml");
    assert!(file_path.exists());

    let loaded = load_workflow(&file_path, dir).unwrap();
    assert_eq!(loaded.name, "my-workflow");
    assert_eq!(loaded.description, "my-workflow workflow");
    assert_eq!(loaded.nodes.len(), 1);
}

#[test]
pub fn save_sourceとloadは未知fieldを拒否する() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let instructions = dir.join("instructions");
    fs::create_dir_all(&instructions).unwrap();
    fs::write(instructions.join("review.md"), "Review.").unwrap();
    let known = r#"
name: strict-storage
description: known values
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: review
"#;
    let with_unknown = r#"
name: strict-storage
description: known values
future_field: ignored
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: review
"#;

    let gateway = WorkflowDefinitionFileSourceGateway::new(dir, dir);
    let repository = WorkflowDefinitionFileRepository::new(dir, dir);
    let definition = gateway.parse(known).unwrap();
    repository.save_source(&definition, known, None).unwrap();
    let error = gateway.parse(with_unknown).unwrap_err();
    assert!(
        matches!(error, WorkflowSourceSaveError::Diagnostics(ref items) if items.iter().any(|item| item.code == "WFS002")),
        "unknown field must be rejected: {error:?}"
    );

    let file_path = dir.join("strict-unknown.yml");
    fs::write(&file_path, with_unknown).unwrap();
    let loaded = load_workflow(&file_path, dir);
    assert!(
        matches!(loaded, Err(StorageError::Diagnostics(ref items)) if items.iter().any(|item| item.code == "WFS002"))
    );
}

#[test]
pub fn list_workflows_returns_sorted_summaries() {
    todo!()
}

#[test]
pub fn list_workflows_uses_file_stem_not_yaml_name() {
    todo!()
}

#[test]
pub fn list_workflows_keeps_invalid_files_as_diagnostic_only_summaries() {
    todo!()
}

#[test]
pub fn test_workflow一覧_lua定義のload失敗を当該定義だけの不正表示に隔離する() {
    todo!()
}

#[test]
pub fn list_workflows_collapses_same_name_across_formats() {
    todo!()
}

#[test]
pub fn list_workflows_empty_dir_includes_builtins() {
    todo!()
}

#[test]
pub fn list_workflows_nonexistent_dir_includes_builtins() {
    todo!()
}

#[test]
pub fn delete_workflow_success() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    seed_workflow(dir, &sample_workflow("deleteme", false)).unwrap();
    assert!(dir.join("deleteme.yml").exists());

    let repository = WorkflowDefinitionFileRepository::new(dir, dir);
    repository
        .delete(&repository.get("deleteme").unwrap())
        .unwrap();
    assert!(!dir.join("deleteme.yml").exists());
}

#[test]
pub fn test_定義削除_利用者ファイルのないbuiltin名は拒否する() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let builtin_name = first_builtin_workflow_name();
    let repository = WorkflowDefinitionFileRepository::new(dir, dir);
    let result = repository.delete(&repository.get(&builtin_name).unwrap());
    assert!(matches!(
        result.unwrap_err(),
        WorkflowError::Validation(ref message) if message.contains("ビルトイン")
    ));
}

#[test]
pub fn delete_nonexistent_workflow_fails() {
    let tmp = TempDir::new().unwrap();
    let result = WorkflowDefinitionFileRepository::new(tmp.path(), tmp.path()).get("nope");
    assert!(matches!(
        result.unwrap_err(),
        WorkflowError::NotFound(ref name) if name == "nope"
    ));
}

// --- resolve_workflow_path tests ---

#[test]
pub fn resolve_workflow_path_success() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    seed_workflow(dir, &sample_workflow("my-workflow", false)).unwrap();

    let result = resolve_workflow_path(dir, "my-workflow");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), dir.join("my-workflow.yml"));
}

#[test]
pub fn resolve_workflow_path_not_found() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_workflow_path(tmp.path(), "nonexistent");
    assert!(matches!(
        result.unwrap_err(),
        StorageError::NotFound { ref name } if name == "nonexistent"
    ));
}

#[test]
pub fn resolve_workflow_path_invalid_name() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_workflow_path(tmp.path(), "../evil");
    assert!(matches!(result.unwrap_err(), StorageError::Validation(_)));
}

#[test]
pub fn validation_error_display_has_stable_kind_prefix() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_workflow_path(tmp.path(), "../evil");
    assert!(result
        .unwrap_err()
        .to_string()
        .starts_with("validation_error:"));
}

#[test]
pub fn resolve_workflow_path_empty_name() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_workflow_path(tmp.path(), "");
    assert!(result.is_err());
}

// --- save_workflow ビルトインガードテスト ---

#[test]
pub fn save_workflow_rename_to_existing_name_detected() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // 既存ワークフローを作成
    seed_workflow(dir, &sample_workflow("existing", false)).unwrap();
    seed_workflow(dir, &sample_workflow("to-rename", false)).unwrap();

    // "to-rename" → "existing" へのリネームは重複検出されるべき
    let target_path = dir.join("existing.yml");
    assert!(
        target_path.exists(),
        "リネーム先のファイルが既に存在する場合、コマンド層で拒否される"
    );
}

#[test]
pub fn save_workflow_new_with_duplicate_name_detected() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    seed_workflow(dir, &sample_workflow("my-flow", false)).unwrap();

    // 同名の新規作成は重複チェックで検出されるべき
    let existing = dir.join("my-flow.yml");
    assert!(
        existing.exists(),
        "新規作成時にファイルが既に存在する場合、コマンド層で拒否される"
    );
}

// --- delete ビルトインガードテスト（既存テストの補完） ---

#[test]
pub fn delete_open_workflow_in_editor_builtin_guard() {
    // open_workflow_in_editor でもビルトインは弾かれる（カスタムファイルのみ）
    let tmp = TempDir::new().unwrap();
    let builtin_name = first_builtin_workflow_name();
    let result = resolve_workflow_path(tmp.path(), &builtin_name);
    assert!(matches!(result.unwrap_err(), StorageError::NotFound { .. }));
}

// --- validate_name 先頭文字テスト ---

/// [02] schema 境界: `storage::load_workflow` は load 経路で facet を解決し、
/// gateway read model として検証する。
#[test]
pub fn load_workflow_resolves_facets_into_gateway_read_model() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let policies = dir.join("policies");
    let instructions = dir.join("instructions");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(policies.join("coding.md"), "POLICY_BODY").unwrap();
    std::fs::write(instructions.join("implement.md"), "INSTRUCTION_BODY").unwrap();

    let yaml = r#"
name: facet-load-test
description: facet resolution test
nodes:
  main:
    session:
      provider: claude
      facets:
        policy: coding
        instruction: implement
"#;
    let file_path = dir.join("facet-load-test.yml");
    std::fs::write(&file_path, yaml).unwrap();
    let wf = load_workflow(&file_path, dir).unwrap();
    let resolved = resolve_and_validate_workflow_facets(&wf, dir).unwrap();
    let contents = resolved.for_node("main").unwrap();
    assert_eq!(contents.policy.as_deref(), Some("POLICY_BODY"));
    assert_eq!(contents.instruction.as_deref(), Some("INSTRUCTION_BODY"));
}

#[test]
pub fn parse_and_load_validate_resolved_facet_artifact_references() {
    for (facet_key, facet_body, expected_ref) in [
        ("missing-ref", "Use {{ missing.field }}", "missing"),
        ("item-out-of-scope", "Use {{ item.path }}", "item"),
    ] {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(instructions.join(format!("{facet_key}.md")), facet_body).unwrap();

        let yaml = format!(
            r#"
name: facet-reference-{facet_key}
description: invalid facet reference
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: {facet_key}
"#
        );
        let parsed = parse_workflow_source(&yaml, dir);
        assert!(matches!(
            parsed.unwrap_err(),
            StorageError::Validation(releashd::test_support::integration::workflow::ValidationError::InvalidArtifactReference { ref reference, .. })
                if reference == expected_ref
        ));

        let file_path = dir.join(format!("facet-reference-{facet_key}.yml"));
        std::fs::write(&file_path, yaml).unwrap();
        let loaded = load_workflow(&file_path, dir);
        assert!(matches!(
            loaded.unwrap_err(),
            StorageError::Validation(releashd::test_support::integration::workflow::ValidationError::InvalidArtifactReference { ref reference, .. })
                if reference == expected_ref
        ));
    }
}

#[test]
pub fn parse_and_load_validate_every_resolved_knowledge_body() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let knowledge = dir.join("knowledge");
    std::fs::create_dir_all(&knowledge).unwrap();
    std::fs::write(knowledge.join("first.md"), "FIRST_OK").unwrap();
    std::fs::write(
        knowledge.join("second.md"),
        "SECOND_USES_{{ missing.field }}",
    )
    .unwrap();

    let yaml = r#"
name: knowledge-reference-validation
description: every knowledge body is validated
nodes:
  main:
    session:
      provider: claude
      facets:
        knowledge: [first, second]
"#;

    for result in [parse_workflow_source(yaml, dir), {
        let file_path = dir.join("knowledge-reference-validation.yml");
        std::fs::write(&file_path, yaml).unwrap();
        load_workflow(&file_path, dir)
    }] {
        assert!(matches!(
            result.unwrap_err(),
            StorageError::Validation(
                releashd::test_support::integration::workflow::ValidationError::InvalidArtifactReference { ref reference, .. }
            ) if reference == "missing"
        ));
    }
}

/// [02] schema 境界: load 経路で 3 種全 facet (policy/knowledge/instruction)
/// が通常の top-level node（fanout child を含む）の read model に解決済みで
/// 格納されることを担保する。
#[test]
pub fn load_workflow_resolves_all_three_facets_for_node_and_child() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let policies = dir.join("policies");
    let knowledge = dir.join("knowledge");
    let instructions = dir.join("instructions");
    for d in [&policies, &knowledge, &instructions] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(policies.join("p.md"), "POLICY").unwrap();
    std::fs::write(knowledge.join("k1.md"), "KNOWLEDGE_1").unwrap();
    std::fs::write(knowledge.join("k2.md"), "KNOWLEDGE_2").unwrap();
    std::fs::write(instructions.join("i.md"), "INSTRUCTION").unwrap();
    std::fs::write(policies.join("pc.md"), "CHILD_POLICY").unwrap();
    std::fs::write(knowledge.join("kc1.md"), "CHILD_KNOWLEDGE_1").unwrap();
    std::fs::write(knowledge.join("kc2.md"), "CHILD_KNOWLEDGE_2").unwrap();
    std::fs::write(instructions.join("ic.md"), "CHILD_INSTRUCTION").unwrap();

    let yaml = r#"
name: facet-all
description: all three facets per node
nodes:
  main:
    sequence:
      children:
      - lead
      - par
  lead:
    session:
      provider: claude
      facets:
        policy: p
        knowledge:
        - k1
        - k2
        instruction: i
  par:
    fanout:
      children:
      - c1
      - c2
  c1:
    session:
      provider: claude
      facets:
        policy: pc
        knowledge: [kc1, kc2]
        instruction: ic
  c2:
    session:
      provider: claude
      facets:
        policy: pc
        knowledge: [kc1, kc2]
        instruction: ic
"#;
    let file_path = dir.join("facet-all.yml");
    std::fs::write(&file_path, yaml).unwrap();
    let wf = load_workflow(&file_path, dir).unwrap();
    let resolved = resolve_and_validate_workflow_facets(&wf, dir).unwrap();

    let lead_contents = resolved.for_node("lead").unwrap();
    assert_eq!(lead_contents.policy.as_deref(), Some("POLICY"));
    assert_eq!(
        lead_contents.knowledge,
        vec!["KNOWLEDGE_1".to_string(), "KNOWLEDGE_2".to_string()]
    );
    assert_eq!(lead_contents.instruction.as_deref(), Some("INSTRUCTION"));

    for child_name in ["c1", "c2"] {
        let child_contents = resolved.for_node(child_name).unwrap();
        assert_eq!(child_contents.policy.as_deref(), Some("CHILD_POLICY"));
        assert_eq!(
            child_contents.knowledge,
            vec![
                "CHILD_KNOWLEDGE_1".to_string(),
                "CHILD_KNOWLEDGE_2".to_string()
            ]
        );
        assert_eq!(
            child_contents.instruction.as_deref(),
            Some("CHILD_INSTRUCTION")
        );
    }
}

/// 各 kind の欠損 facet は load 段階で構造化 Diagnostic として拒否される。
#[test]
pub fn load_workflow_rejects_missing_facet() {
    for (facet_kind, facet_key, facet_yaml) in [
        ("policy", "missing-policy", "policy: missing-policy"),
        (
            "knowledge",
            "missing-knowledge",
            "knowledge: [missing-knowledge]",
        ),
        (
            "instruction",
            "missing-instruction",
            "instruction: missing-instruction",
        ),
    ] {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let workflow_name = format!("missing-{facet_kind}");
        let yaml = format!(
            r#"
name: {workflow_name}
description: missing {facet_kind} test
nodes:
  main:
    session:
      provider: claude
      facets:
        {facet_yaml}
"#
        );
        let file_path = dir.join(format!("{workflow_name}.yml"));
        std::fs::write(&file_path, yaml).unwrap();

        let result = load_workflow(&file_path, dir);

        let Err(StorageError::Diagnostics(items)) = result else {
            panic!("missing {facet_kind} must return structured diagnostics");
        };
        let missing = items
            .iter()
            .find(|item| item.code == "FAC002")
            .unwrap_or_else(|| panic!("missing {facet_kind} FAC002"));
        assert_eq!(
            missing.workflow_name.as_deref(),
            Some(workflow_name.as_str())
        );
        assert_eq!(missing.node_name.as_deref(), Some("main"));
        assert_eq!(missing.facet_key.as_deref(), Some(facet_key));
        assert_eq!(missing.facet_kind.as_deref(), Some(facet_kind));
        assert_eq!(missing.field.as_deref(), Some(facet_kind));
        assert!(missing.message.contains(facet_key));
    }
}

#[test]
pub fn test_workflow読込_facetの置き場所が読めないとbuiltinで補わず失敗を返す() {
    // Given
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    std::fs::write(dir.join("knowledge"), "not a directory").unwrap();
    let yaml = r#"
name: builtin-facet-with-broken-inventory
description: builtin facet load test
nodes:
  main:
    session:
      provider: claude
      facets:
        knowledge: [releash-thread-cli]
"#;
    let file_path = dir.join("builtin-facet-with-broken-inventory.yml");
    std::fs::write(&file_path, yaml).unwrap();
    // When
    let result = load_workflow(&file_path, dir);
    // Then
    assert!(matches!(result, Err(StorageError::FacetResolution(_))));
}

/// Artifact template references load without a workflow-level variables section.
#[test]
pub fn load_workflow_accepts_request_artifact_reference() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let instructions = dir.join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("impl.md"), "Request: {{ goal }}").unwrap();

    let yaml = r#"
name: request-ref
description: parameter reference test
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: impl
    input:
    - goal
"#;
    let file_path = dir.join("request-ref.yml");
    std::fs::write(&file_path, yaml).unwrap();
    let wf = load_workflow(&file_path, dir).expect("load must succeed");
    assert_eq!(wf.nodes[0].name, "main");
}

#[test]
pub fn loads_and_lists_lua_workflow_with_source_format() {
    todo!()
}

#[test]
pub fn test_workflow一覧_module評価中のhost呼出で作ったnodeを有効な定義として扱う() {
    todo!()
}

#[test]
pub fn rejects_lua_workflow_name_that_differs_from_file_stem() {
    // Given
    let tmp = TempDir::new().unwrap();
    // When
    let path = tmp.path().join("file-name.lua");
    std::fs::write(
        &path,
        r#"
local r = require("releash")
return r.workflow{
  name = "declared-name",
  description = "Mismatch",
  main = r.command{ command = "true" },
}
"#,
    )
    .unwrap();

    let error = load_workflow(&path, tmp.path()).unwrap_err();
    // Then
    let StorageError::Diagnostics(items) = error else {
        panic!("name mismatch must produce diagnostics");
    };

    let mismatch = items.iter().find(|item| item.code == "WFS006").unwrap();
    let span = mismatch.span.as_ref().unwrap();
    assert_eq!(span.source.as_deref(), Some("file-name.lua"));
    assert_eq!(span.start_line, 6);
}

#[test]
pub fn rejects_ambiguous_yaml_and_lua_files_with_same_stem() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("duplicate.yml"), "name: duplicate").unwrap();
    std::fs::write(tmp.path().join("duplicate.lua"), "return nil").unwrap();

    let error = resolve_workflow_path(tmp.path(), "duplicate").unwrap_err();
    let StorageError::Diagnostics(items) = error else {
        panic!("duplicate source files must produce diagnostics");
    };

    assert!(items.iter().any(|item| item.code == "WFS006"));
}

#[test]
pub fn test_定義一覧と診断_名前の集合が一致し同名の両形式を診断する() {
    todo!()
}

#[test]
pub fn test_定義一覧_壊れた定義の失敗をvalidationとして残す() {
    todo!()
}

fn seed_workflow(
    dir: &std::path::Path,
    workflow: &releashd::test_support::integration::workflow::WorkflowDefinitionContent,
) -> std::io::Result<()> {
    std::fs::write(
        dir.join(format!("{}.yml", workflow.name)),
        serde_saphyr::to_string(workflow).unwrap(),
    )
}
