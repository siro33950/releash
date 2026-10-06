
use crate::domain::workflow::services::validation;
use super::*;
use crate::adaptor::gateway::workflow::schema::{FacetRefs, NodeDefinition, NodeKind, SessionSpec};
use tempfile::TempDir;

fn sample_workflow(name: &str, builtin: bool) -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: name.to_string(),
        description: format!("{name} workflow"),
        builtin,
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
    builtin::list_builtin_workflows()
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
    // Given
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("unreadable.yml")).unwrap();
    // When
    let summaries = list_workflows(directory.path()).unwrap();
    // Then
    let row = summaries.iter().find(|s| s.name == "unreadable").unwrap();
    assert!(matches!(
        row.failure.as_ref().unwrap().kind,
        crate::domain::failure::Failure::Technical(_)
    ));
    assert!(row.description.is_empty());
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
    let result = super::super::diagnostics::diagnose_all(&invalid, directory.path());
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
    save_workflow(dir, &wf).unwrap();

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

    assert!(save_workflow_source(dir, dir, known).is_ok());
    let error = save_workflow_source(dir, dir, with_unknown).unwrap_err();
    assert!(
        matches!(error, StorageError::Diagnostics(ref items) if items.iter().any(|item| item.code == "WFS002")),
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
    // Given
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    save_workflow(dir, &sample_workflow("charlie", false)).unwrap();
    save_workflow(dir, &sample_workflow("alpha", false)).unwrap();
    save_workflow(dir, &sample_workflow("bravo", false)).unwrap();

    // When
    let list = list_workflows(dir).unwrap();
    // Then
    let builtin_names = builtin_workflow_names();
    let mut expected_names = vec![
        "alpha".to_string(),
        "bravo".to_string(),
        "charlie".to_string(),
    ];
    expected_names.extend(builtin_names.iter().cloned());
    expected_names.sort();
    assert_eq!(
        list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        expected_names
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    for name in builtin_names {
        let entry = list
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("builtin workflow '{name}' must be present in merged list"));
        assert!(entry.builtin, "builtin '{name}' must be marked builtin");
    }
}

#[test]
pub fn list_workflows_uses_file_stem_not_yaml_name() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // save_workflowで作成（ファイル名 = YAML本文name）
    save_workflow(dir, &sample_workflow("original", false)).unwrap();

    // ファイルをリネームしてYAML本文nameとファイルstemを乖離させる
    fs::rename(dir.join("original.yml"), dir.join("renamed.yml")).unwrap();

    let list = list_workflows(dir).unwrap();
    let disk_entry = list.iter().find(|s| s.name == "renamed").unwrap();
    // Summary.nameはファイルstem（renamed）であるべき、YAML本文（original）ではない
    assert_eq!(disk_entry.name, "renamed");
}

#[test]
pub fn list_workflows_keeps_invalid_files_as_diagnostic_only_summaries() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("broken.yml"),
        r#"
name: broken
description: invalid workflow
nodes:
  main:
    artifact: something
"#,
    )
    .unwrap();

    let list = list_workflows(dir).unwrap();
    let disk_entry = list.iter().find(|s| s.name == "broken").unwrap();

    assert_eq!(disk_entry.description, "Invalid workflow definition");
    assert!(!disk_entry.builtin);
}

#[test]
pub fn test_workflow一覧_lua定義のload失敗を当該定義だけの不正表示に隔離する() {
    // Given
    let tmp = TempDir::new().unwrap();
    let module_dir = tmp.path().join("broken-parts");
    fs::create_dir(&module_dir).unwrap();
    fs::write(
        module_dir.join("nodes.lua"),
        "local r = require(\"releash\")\nreturn r.command{ command = }\n",
    )
    .unwrap();
    let broken_path = tmp.path().join("broken.lua");
    fs::write(
        &broken_path,
        r#"local r = require("releash")
local nodes = require("broken-parts.nodes")
return r.workflow{
  name = "broken", description = "Broken workflow", main = nodes.main,
}
"#,
    )
    .unwrap();
    fs::write(
        tmp.path().join("invalid-yaml.yml"),
        r#"name: invalid-yaml
description: Invalid YAML workflow
nodes:
  main:
    artifact: something
"#,
    )
    .unwrap();
    fs::write(
        tmp.path().join("healthy.lua"),
        r#"local r = require("releash")
return r.workflow{
  name = "healthy", description = "Healthy workflow",
  main = r.command{ command = "printf healthy" },
}
"#,
    )
    .unwrap();

    // When
    let load_error = load_workflow(&broken_path, tmp.path()).unwrap_err();
    let summaries = list_workflows(tmp.path()).unwrap();

    // Then
    let StorageError::Diagnostics(load_diagnostics) = load_error else {
        panic!("Lua 定義の load 失敗は structured diagnostics であるべき");
    };
    assert!(load_diagnostics.iter().any(|item| item.code == "WFS009"));

    let broken = summaries
        .iter()
        .find(|summary| summary.name == "broken")
        .expect("broken summary");
    let invalid_yaml = summaries
        .iter()
        .find(|summary| summary.name == "invalid-yaml")
        .expect("invalid-yaml summary");
    assert_eq!(broken.description, invalid_yaml.description);
    assert_eq!(broken.description, "Invalid workflow definition");
    assert_eq!(broken.builtin, invalid_yaml.builtin);
    assert_eq!(broken.is_running, invalid_yaml.is_running);
    assert_eq!(broken.source_format, WorkflowSourceFormat::Lua);
    assert_eq!(invalid_yaml.source_format, WorkflowSourceFormat::Yaml);

    let healthy = summaries
        .iter()
        .find(|summary| summary.name == "healthy")
        .expect("healthy summary");
    assert_eq!(healthy.description, "Healthy workflow");
    assert_eq!(healthy.source_format, WorkflowSourceFormat::Lua);
}

#[test]
pub fn list_workflows_collapses_same_name_across_formats() {
    // Given
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    save_workflow(dir, &sample_workflow("duplicated", false)).unwrap();
    fs::write(
        dir.join("duplicated.lua"),
        r#"
local r = require("releash")
return r.workflow{
  name = "duplicated", description = "Lua duplicate",
  main = r.command{ command = "true" },
}
"#,
    )
    .unwrap();

    // When
    let list = list_workflows(dir).unwrap();
    let matched = list
        .iter()
        .filter(|summary| summary.name == "duplicated")
        .collect::<Vec<_>>();

    // Then
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].description, DUPLICATE_NAME_DESCRIPTION);
    assert!(resolve_workflow_path(dir, "duplicated").is_err());
}

#[test]
pub fn list_workflows_empty_dir_includes_builtins() {
    let tmp = TempDir::new().unwrap();
    let list = list_workflows(tmp.path()).unwrap();
    for name in builtin_workflow_names() {
        assert!(list.iter().any(|s| s.name == name));
    }
}

#[test]
pub fn list_workflows_nonexistent_dir_includes_builtins() {
    let list = list_workflows(Path::new("/nonexistent/path")).unwrap();
    for name in builtin_workflow_names() {
        assert!(list.iter().any(|s| s.name == name));
    }
}

#[test]
pub fn delete_workflow_success() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    save_workflow(dir, &sample_workflow("deleteme", false)).unwrap();
    assert!(dir.join("deleteme.yml").exists());

    delete_workflow(dir, "deleteme").unwrap();
    assert!(!dir.join("deleteme.yml").exists());
}

#[test]
pub fn delete_builtin_workflow_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let builtin_name = first_builtin_workflow_name();
    let result = delete_workflow(dir, &builtin_name);
    assert!(matches!(
        result.unwrap_err(),
        StorageError::BuiltinProtected { ref name } if name == &builtin_name
    ));
}

#[test]
pub fn delete_nonexistent_workflow_fails() {
    let tmp = TempDir::new().unwrap();
    let result = delete_workflow(tmp.path(), "nope");
    assert!(matches!(
        result.unwrap_err(),
        StorageError::NotFound { ref name } if name == "nope"
    ));
}

// --- resolve_workflow_path tests ---

#[test]
pub fn resolve_workflow_path_success() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    save_workflow(dir, &sample_workflow("my-workflow", false)).unwrap();

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
    save_workflow(dir, &sample_workflow("existing", false)).unwrap();
    save_workflow(dir, &sample_workflow("to-rename", false)).unwrap();

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

    save_workflow(dir, &sample_workflow("my-flow", false)).unwrap();

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
    // ビルトインは削除不可（既にdelete_builtin_workflow_failsでテスト済み）
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
            StorageError::Validation(validation::ValidationError::InvalidArtifactReference { ref reference, .. })
                if reference == expected_ref
        ));

        let file_path = dir.join(format!("facet-reference-{facet_key}.yml"));
        std::fs::write(&file_path, yaml).unwrap();
        let loaded = load_workflow(&file_path, dir);
        assert!(matches!(
            loaded.unwrap_err(),
            StorageError::Validation(validation::ValidationError::InvalidArtifactReference { ref reference, .. })
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
                validation::ValidationError::InvalidArtifactReference { ref reference, .. }
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
    // Given
    let tmp = TempDir::new().unwrap();
    let source = r#"
local r = require("releash")
return r.workflow{
  name = "lua-workflow",
  description = "Lua workflow",
  main = r.command{ command = "true" },
}
"#;
    let path = tmp.path().join("lua-workflow.lua");
    std::fs::write(&path, source).unwrap();

    // When
    let workflow = load_workflow(&path, tmp.path()).unwrap();
    let summaries = list_workflows(tmp.path()).unwrap();

    // Then
    assert_eq!(workflow.name, "lua-workflow");
    assert_eq!(workflow.nodes[0].name, "main");
    assert_eq!(
        summaries
            .iter()
            .find(|summary| summary.name == "lua-workflow")
            .unwrap()
            .source_format,
        WorkflowSourceFormat::Lua
    );
}

#[test]
pub fn test_workflow一覧_module評価中のhost呼出で作ったnodeを有効な定義として扱う() {
    // Given
    let tmp = TempDir::new().unwrap();
    let module_dir = tmp.path().join("module-host-parts");
    std::fs::create_dir(&module_dir).unwrap();
    std::fs::write(
        module_dir.join("nodes.lua"),
        r#"local r = require("releash")
return { main = r.command{ command = "true" } }
"#,
    )
    .unwrap();
    let workflow_path = tmp.path().join("module-host.lua");
    std::fs::write(
        &workflow_path,
        r#"local r = require("releash")
local nodes = require("module-host-parts.nodes")
return r.workflow{
  name = "module-host", description = "Module host workflow", main = nodes.main,
}
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("healthy.lua"),
        r#"local r = require("releash")
return r.workflow{
  name = "healthy", description = "Healthy workflow",
  main = r.command{ command = "printf healthy" },
}
"#,
    )
    .unwrap();

    // When
    let workflow = load_workflow(&workflow_path, tmp.path()).unwrap();
    let summaries = list_workflows(tmp.path()).unwrap();

    // Then
    let main = workflow
        .nodes
        .iter()
        .find(|node| node.name == "main")
        .expect("module が返した main node");
    let NodeKind::Command(command) = &main.kind else {
        panic!("module が返した node は command であるべき");
    };
    assert_eq!(command.command, "true");

    let module_host = summaries
        .iter()
        .find(|summary| summary.name == "module-host")
        .expect("module-host summary");
    assert_eq!(module_host.description, "Module host workflow");
    assert_eq!(module_host.source_format, WorkflowSourceFormat::Lua);

    let healthy = summaries
        .iter()
        .find(|summary| summary.name == "healthy")
        .expect("healthy summary");
    assert_eq!(healthy.description, "Healthy workflow");
    assert_eq!(healthy.source_format, WorkflowSourceFormat::Lua);
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
    // Given
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    let mut valid = sample_workflow("valid-yaml", false);
    valid.nodes[0] = NodeDefinition {
        name: "main".into(),
        kind: NodeKind::Command(crate::domain::workflow::CommandSpec {
            command: "true".into(),
            env: Default::default(),
        }),
        ..Default::default()
    };
    fs::write(
        dir.join("valid-yaml.yml"),
        serde_saphyr::to_string(&valid).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("valid-lua.lua"),
        "local r = require('releash'); return r.workflow{name='valid-lua', description='Valid Lua workflow', main=r.command{command='true'}}",
    )
    .unwrap();
    fs::write(dir.join("same.lua"), "local r = require('releash'); return r.workflow{name='same', description='Same workflow', main=r.command{command='true'}}").unwrap();
    fs::create_dir(dir.join("same.yml")).unwrap();
    let mismatch = sample_workflow("body-name", false);
    fs::write(
        dir.join("file-stem.yml"),
        serde_saphyr::to_string(&mismatch)
            .unwrap()
            .replace("review-acceptance", "missing-instruction"),
    )
    .unwrap();
    fs::write(dir.join("broken.yml"), "[broken").unwrap();
    fs::create_dir(dir.join("unreadable.yml")).unwrap();
    fs::write(dir.join("duplicate.yml"), "[broken yaml").unwrap();
    fs::write(dir.join("duplicate.lua"), "invalid lua syntax").unwrap();
    let builtin = builtin::list_builtin_workflows()[0].name.clone();
    valid.name = builtin.clone();
    fs::write(
        dir.join(format!("{builtin}.yml")),
        serde_saphyr::to_string(&valid).unwrap(),
    )
    .unwrap();
    // When
    let rows = list_workflows_with_facets(dir, dir).unwrap();
    let report = super::super::diagnostics::diagnose_all(dir, dir).unwrap();
    // Then
    let listed: std::collections::BTreeSet<_> = rows.iter().map(|row| row.name.clone()).collect();
    let diagnosed: std::collections::BTreeSet<_> =
        report.workflow_summaries.keys().cloned().collect();
    assert_eq!(listed, diagnosed);
    assert!(!diagnosed.contains("body-name"));
    assert!(report
        .items
        .iter()
        .any(|item| item.code == "FAC002" && item.workflow_name.as_deref() == Some("file-stem")));
    let same: Vec<_> = rows.iter().filter(|row| row.name == "same").collect();
    assert_eq!(same.len(), 1);
    assert_eq!(
        same[0].failure,
        Some(crate::domain::failure::WorkFailure::from_error(
            &crate::domain::workflow::WorkflowError::from(StorageError::Io(
                fs::read_to_string(dir.join("same.yml")).unwrap_err()
            ))
        ))
    );
    assert!(matches!(
        same[0].failure.as_ref().unwrap().kind,
        crate::domain::failure::Failure::Technical(_)
    ));
    assert_eq!(report.workflow_summaries["valid-yaml"].error_count, 0);
    assert_eq!(
        report.workflow_summaries["valid-lua"].error_count, 0,
        "{:?}",
        report.items
    );
    assert_eq!(report.workflow_summaries[&builtin].error_count, 0);
    assert_eq!(rows.iter().filter(|row| row.name == "duplicate").count(), 1);
    let duplicate: Vec<_> = report
        .items
        .iter()
        .filter(|item| item.workflow_name.as_deref() == Some("duplicate"))
        .collect();
    assert!(duplicate.iter().any(|item| item.message.contains("YAML")));
    assert!(duplicate.iter().any(|item| item
        .span
        .as_ref()
        .is_some_and(|span| span.source.as_deref() == Some("duplicate.lua"))));
}

#[test]
pub fn test_定義一覧_壊れた定義の失敗をvalidationとして残す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("broken.yml"), "[broken").unwrap();
    let diagnosis = diagnose_workflow_file(
        &directory.path().join("broken.yml"),
        "[broken",
        directory.path(),
        directory.path(),
    );
    // When
    let rows = list_workflows(directory.path()).unwrap();
    // Then
    let row = rows.iter().find(|row| row.name == "broken").unwrap();
    assert!(matches!(
        row.failure.as_ref().unwrap().kind,
        crate::domain::failure::Failure::Business(_)
    ));
    assert_eq!(
        row.failure,
        Some(crate::domain::failure::WorkFailure::from_error(
            &crate::domain::workflow::WorkflowError::Validation(
                StorageError::Diagnostics(diagnosis.diagnostics).to_string()
            )
        ))
    );
    assert_eq!(row.description, "Invalid workflow definition");
}
