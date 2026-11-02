use releashd::test_support::integration::workflow::WorkflowDefinitionRepository;
#[cfg(unix)]
#[test]
pub fn test_workflow探索失敗_定義とsourceをbuiltinへ変換しない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let gateway = WorkflowDefinitionFileSourceGateway::new(dir.path(), dir.path());
    let name = releashd::test_support::integration::workflow::list_builtin_workflows()[0]
        .name
        .clone();
    let path = dir.path().join(format!("{name}.yml"));
    std::os::unix::fs::symlink(&path, &path).unwrap();
    // When
    let definition = repository.get(&name);
    let source = gateway.get_source(&name);
    // Then
    assert!(definition.is_err());
    assert!(source.is_err());
}

#[test]
pub fn test_workflow探索_定義がないと未設定を返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    // When
    let definition = repository.find(Some("missing")).unwrap();
    assert!(repository.find(None).unwrap().is_none());
    let gateway = WorkflowDefinitionFileSourceGateway::new(dir.path(), dir.path());
    assert_eq!(gateway.get_source("missing").unwrap(), None);
    assert!(matches!(
        repository.get("missing"),
        Err(WorkflowError::NotFound(_))
    ));
    // Then
    assert!(definition.is_none());
}
use releashd::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
use releashd::test_support::integration::workflow::NodeDefinition;
use releashd::test_support::integration::workflow::NodeKind;
use releashd::test_support::integration::workflow::SessionSpec;
use releashd::test_support::integration::workflow::WorkflowDefinitionFileRepository;
use releashd::test_support::integration::workflow::WorkflowDefinitionFileSourceGateway;
use releashd::test_support::integration::workflow::WorkflowDefinitionSourceService;
use releashd::test_support::integration::workflow::WorkflowError;
use releashd::test_support::integration::workflow::WorkflowSourceSaveError;
use releashd::test_support::integration::workflow::{
    WorkflowDefinition, WorkflowDefinitionContent,
};
use std::fs;

use tempfile::TempDir;

fn definition(name: &str) -> WorkflowDefinitionContent {
    WorkflowDefinitionContent {
        name: name.to_string(),
        description: "desc".to_string(),
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

    repo.save(
        WorkflowDefinition::new(workflows.path().to_path_buf(), definition("wf")).unwrap(),
        None,
    )
    .unwrap();
    let loaded = repo.get("wf").unwrap();

    assert_eq!(loaded.content().unwrap().name, "wf");
    assert!(workflows.path().join("wf.yml").exists());
}

#[test]
pub fn rename_removes_old_workflow_file_after_successful_save() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    let repo = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
    repo.save(
        WorkflowDefinition::new(workflows.path().to_path_buf(), definition("old")).unwrap(),
        None,
    )
    .unwrap();

    repo.save(
        WorkflowDefinition::new(workflows.path().to_path_buf(), definition("new")).unwrap(),
        Some("old"),
    )
    .unwrap();

    assert!(!workflows.path().join("old.yml").exists());
    assert!(workflows.path().join("new.yml").exists());
}

#[test]
pub fn source_gateway_saves_and_reads_verbatim_workflow_source() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let repository = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
    let source = source("wf");

    let saved = save_source(&gateway, &repository, &source, None).unwrap();
    let loaded = gateway.get_source("wf").unwrap().unwrap();

    assert_eq!(saved.content().unwrap().name, "wf");
    assert_eq!(loaded, source);
}

#[test]
pub fn source_gateway_rejects_invalid_source_without_overwriting_existing_file() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let repository = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
    let original = source("stable");
    save_source(&gateway, &repository, &original, None).unwrap();

    let err = save_source(
        &gateway,
        &repository,
        &invalid_source_unknown_field("stable"),
        Some("stable"),
    )
    .unwrap_err();
    let loaded = gateway.get_source("stable").unwrap().unwrap();

    assert!(
        matches!(err, WorkflowSourceSaveError::Diagnostics(items) if items.iter().any(|item| item.code == "WFS002"))
    );
    assert_eq!(loaded, original);
}

#[test]
pub fn source_gateway_rename_removes_old_workflow_file_after_successful_save() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let repository = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
    save_source(&gateway, &repository, &source("old"), None).unwrap();

    save_source(&gateway, &repository, &source("new"), Some("old")).unwrap();

    assert!(!workflows.path().join("old.yml").exists());
    assert!(workflows.path().join("new.yml").exists());
}

#[test]
pub fn test_定義取得と一覧_builtinと同名の利用者ファイルはfile由来() {
    todo!()
}

#[test]
pub fn source_gateway_rejects_invalid_workflow_name() {
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
    let repository = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());

    let err = save_source(&gateway, &repository, &source("-invalid"), None).unwrap_err();

    assert!(
        matches!(err, WorkflowSourceSaveError::Diagnostics(items) if items.iter().any(|item| item.code == "WFS006"))
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

    let error = gateway.parse(source).unwrap_err();
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
    let repository = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
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
        repository.get("lua-source").unwrap().format(),
        releashd::test_support::integration::workflow::WorkflowSourceFormat::Lua
    );
    seed_instruction_facet(&facets);
    let error = save_source(
        &gateway,
        &repository,
        &self::source("replacement"),
        Some("lua-source"),
    )
    .unwrap_err();
    assert!(
        matches!(error, WorkflowSourceSaveError::Workflow(error) if error.to_string().contains("Lua workflow"))
    );
    assert_eq!(
        fs::read_to_string(workflows.path().join("lua-source.lua")).unwrap(),
        source
    );
}

fn save_source(
    _gateway: &WorkflowDefinitionFileSourceGateway,
    _repository: &WorkflowDefinitionFileRepository,
    _source: &str,
    _original_name: Option<&str>,
) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
    todo!()
}

#[test]
fn test_定義取得_同梱定義はbuiltin由来() {
    todo!()
}

#[test]
fn test_定義取得_解釈できない利用者ファイルはentityの失敗に残る() {
    todo!()
}

#[test]
fn test_定義保存_置換失敗時は旧ファイルを残し一時ファイルを消す() {
    // Given
    let dir = TempDir::new().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let original =
        WorkflowDefinition::new(dir.path().to_path_buf(), definition("original")).unwrap();
    repository.save(original, None).unwrap();
    fs::create_dir(dir.path().join("replacement.yml")).unwrap();
    let replacement =
        WorkflowDefinition::new(dir.path().to_path_buf(), definition("replacement")).unwrap();
    // When
    let result = repository.save(replacement.clone(), Some("original"));
    let source_result = repository.save_source(&replacement, "new content", Some("original"));
    // Then
    assert!(result.is_err());
    assert!(source_result.is_err());
    assert!(dir.path().join("original.yml").is_file());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn test_定義取得_読めない利用者ファイルは失敗を持つentityになる() {
    todo!()
}

#[test]
fn test_定義一覧_実行中の定義を取得元にかかわらず示す() {
    todo!()
}

#[test]
fn test_workflow一覧_同名のyamlとluaを一行に畳む() {
    todo!()
}

#[test]
fn test_builtin解決失敗_entityと一覧の失敗に保持し他の定義を返す() {
    todo!()
}
