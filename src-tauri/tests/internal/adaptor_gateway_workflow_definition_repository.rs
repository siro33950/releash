use releash_lib::test_support::integration::workflow::WorkflowDefinitionRepository;
#[cfg(unix)]
#[test]
pub fn test_workflow探索失敗_定義とsourceと形式をbuiltinへ変換しない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let gateway = WorkflowDefinitionFileSourceGateway::new(dir.path(), dir.path());
    let name = releash_lib::test_support::integration::workflow::list_builtin_workflows()[0]
        .name
        .clone();
    let path = dir.path().join(format!("{name}.yml"));
    std::os::unix::fs::symlink(&path, &path).unwrap();
    // When
    let definition = repository.get(&name);
    let source = gateway.get_source(&name);
    let format = gateway.source_format(&name);
    // Then
    assert!(definition.is_err());
    assert!(source.is_err());
    assert!(format.is_err());
}

#[test]
pub fn test_workflow探索_定義がないと未設定を返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    // When
    let definition = repository.get("missing").unwrap();
    // Then
    assert!(definition.is_none());
}
use releash_lib::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
use releash_lib::test_support::integration::workflow::NodeDefinition;
use releash_lib::test_support::integration::workflow::NodeKind;
use releash_lib::test_support::integration::workflow::SessionSpec;
use releash_lib::test_support::integration::workflow::WorkflowDefinition;
use releash_lib::test_support::integration::workflow::WorkflowDefinitionFileRepository;
use releash_lib::test_support::integration::workflow::WorkflowDefinitionFileSourceGateway;
use releash_lib::test_support::integration::workflow::WorkflowDefinitionSourceGateway;
use releash_lib::test_support::integration::workflow::WorkflowError;
use releash_lib::test_support::integration::workflow::WorkflowSourceSaveError;
use std::fs;

use tempfile::TempDir;

fn definition(name: &str) -> WorkflowDefinition {
    WorkflowDefinition {
        name: name.to_string(),
        description: "desc".to_string(),
        builtin: false,
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
            ..Default::default()
        }],
        entry: "main".to_string(),
    }
}

fn seed_instruction_facet(facets: &TempDir) {
    let instructions = facets.path().join("instructions");
    fs::create_dir_all(&instructions).unwrap();
    fs::write(instructions.join("implement.md"), "Implement.").unwrap();
}

fn source(name: &str) -> String {
    format!(
        r#"# keep this comment
name: {name}
description: source workflow
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
"#
    )
}

fn invalid_source_unknown_field(name: &str) -> String {
    format!(
        r#"
name: {name}
description: invalid workflow
future_field: ignored
nodes:
  main:
    session:
      provider: claude
"#
    )
}

#[test]
pub fn saves_and_loads_workflow_yaml_through_existing_storage() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    let instructions = facets.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(
        instructions.join("review-acceptance.md"),
        "Review the change.",
    )
    .unwrap();
    let repo = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());

    repo.save(definition("wf"), None).unwrap();
    let loaded = repo.get("wf").unwrap().unwrap();

    assert_eq!(loaded.name, "wf");
    assert!(workflows.path().join("wf.yml").exists());
}

#[test]
pub fn rename_removes_old_workflow_file_after_successful_save() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    let repo = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
    repo.save(definition("old"), None).unwrap();

    repo.save(definition("new"), Some("old")).unwrap();

    assert!(!workflows.path().join("old.yml").exists());
    assert!(workflows.path().join("new.yml").exists());
}

#[test]
pub fn source_gateway_saves_and_reads_verbatim_workflow_source() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let source = source("wf");

    let saved = gateway.save_source(&source, None).unwrap();
    let loaded = gateway.get_source("wf").unwrap().unwrap();

    assert_eq!(saved.name, "wf");
    assert_eq!(loaded, source);
}

#[test]
pub fn source_gateway_rejects_invalid_source_without_overwriting_existing_file() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let original = source("stable");
    gateway.save_source(&original, None).unwrap();

    let err = gateway
        .save_source(&invalid_source_unknown_field("stable"), Some("stable"))
        .unwrap_err();
    let loaded = gateway.get_source("stable").unwrap().unwrap();

    assert!(
        matches!(err, WorkflowError::External(message) if message.contains("workflow_diagnostics") && message.contains("WFS002"))
    );
    assert_eq!(loaded, original);
}

#[test]
pub fn source_gateway_rename_removes_old_workflow_file_after_successful_save() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    gateway.save_source(&source("old"), None).unwrap();

    gateway.save_source(&source("new"), Some("old")).unwrap();

    assert!(!workflows.path().join("old.yml").exists());
    assert!(workflows.path().join("new.yml").exists());
}

#[test]
pub fn source_gateway_rejects_builtin_name_collision() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let builtin_name = releash_lib::test_support::integration::workflow::list_builtin_workflows()
        .first()
        .expect("builtin workflow fixture must exist")
        .name
        .clone();

    let err = gateway
        .save_source(&source(&builtin_name), None)
        .unwrap_err();

    assert!(err.to_string().contains("ビルトイン名と重複"));
    assert!(!workflows
        .path()
        .join(format!("{builtin_name}.yml"))
        .exists());
}

#[test]
pub fn source_gateway_rejects_invalid_workflow_name() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());

    let err = gateway.save_source(&source("-invalid"), None).unwrap_err();

    assert!(
        matches!(err, WorkflowError::External(message) if message.contains("workflow_diagnostics") && message.contains("WFS006"))
    );
    assert!(!workflows.path().join("-invalid.yml").exists());
}

#[test]
pub fn source_gateway_returns_structured_diagnostic_for_missing_knowledge() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    let knowledge = facets.path().join("knowledge");
    fs::create_dir_all(&knowledge).unwrap();
    fs::write(knowledge.join("known.md"), "Known context.").unwrap();
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let source = r#"
name: missing-knowledge
description: missing knowledge diagnostic
nodes:
  main:
    session:
      provider: claude
      facets:
        knowledge: [known, missing-name]
"#;

    let error = gateway
        .save_source_with_diagnostics(source, None)
        .unwrap_err();
    let WorkflowSourceSaveError::Diagnostics(items) = error else {
        panic!("missing knowledge must remain a structured diagnostic");
    };
    let diagnostic = items
        .iter()
        .find(|item| item.code == "FAC002")
        .expect("missing knowledge FAC002");
    assert_eq!(
        diagnostic.workflow_name.as_deref(),
        Some("missing-knowledge")
    );
    assert_eq!(diagnostic.node_name.as_deref(), Some("main"));
    assert_eq!(diagnostic.facet_key.as_deref(), Some("missing-name"));
    assert_eq!(diagnostic.facet_kind.as_deref(), Some("knowledge"));
    assert_eq!(diagnostic.field.as_deref(), Some("knowledge"));
    assert!(diagnostic.message.contains("missing-name"));
    assert!(!workflows.path().join("missing-knowledge.yml").exists());
}

#[test]
pub fn source_gateway_reads_lua_verbatim_and_rejects_save() {
    // Given
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let source = r#"
local r = require("releash")
return r.workflow{
  name = "lua-source", description = "Lua",
  main = r.command{ command = "true" },
}
"#;
    fs::write(workflows.path().join("lua-source.lua"), source).unwrap();

    assert_eq!(
        gateway.get_source("lua-source").unwrap().as_deref(),
        Some(source)
    );
    assert_eq!(
        gateway.source_format("lua-source").unwrap(),
        releash_lib::test_support::integration::workflow::WorkflowSourceFormat::Lua
    );
    let error = gateway
        .save_source(
            "name: replacement\ndescription: replacement\nnodes: {}\n",
            Some("lua-source"),
        )
        .unwrap_err();
    assert!(error.to_string().contains("Lua workflow"));
    assert_eq!(
        fs::read_to_string(workflows.path().join("lua-source.lua")).unwrap(),
        source
    );
}
