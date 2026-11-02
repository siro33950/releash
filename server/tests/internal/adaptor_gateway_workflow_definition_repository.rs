use releashd::test_support::integration::workflow::WorkflowDefinitionRepository;
#[cfg(unix)]
#[test]
pub fn test_workflow探索失敗_定義とsourceと形式をbuiltinへ変換しない() {
    todo!()
}

#[test]
pub fn test_workflow探索_定義がないと未設定を返す() {
    todo!()
}
use releashd::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
use releashd::test_support::integration::workflow::NodeDefinition;
use releashd::test_support::integration::workflow::NodeKind;
use releashd::test_support::integration::workflow::SessionSpec;
use releashd::test_support::integration::workflow::WorkflowDefinitionContent;
use releashd::test_support::integration::workflow::WorkflowDefinitionFileRepository;
use releashd::test_support::integration::workflow::WorkflowDefinitionFileSourceGateway;
use releashd::test_support::integration::workflow::WorkflowDefinitionSourceService;
use releashd::test_support::integration::workflow::WorkflowError;
use releashd::test_support::integration::workflow::WorkflowSourceSaveError;
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
    todo!()
}

#[test]
pub fn rename_removes_old_workflow_file_after_successful_save() {
    todo!()
}

#[test]
pub fn source_gateway_saves_and_reads_verbatim_workflow_source() {
    todo!()
}

#[test]
pub fn source_gateway_rejects_invalid_source_without_overwriting_existing_file() {
    todo!()
}

#[test]
pub fn source_gateway_rename_removes_old_workflow_file_after_successful_save() {
    todo!()
}

#[test]
pub fn source_gateway_rejects_builtin_name_collision() {
    todo!()
}

#[test]
pub fn source_gateway_rejects_invalid_workflow_name() {
    todo!()
}

#[test]
pub fn source_gateway_returns_structured_diagnostic_for_missing_knowledge() {
    todo!()
}

#[test]
pub fn source_gateway_reads_lua_verbatim_and_rejects_save() {
    todo!()
}
