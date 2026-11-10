use releashd::test_support::integration::workflow::WorkflowDefinitionRepository;
use releashd::test_support::integration::workflow::{
    WorkflowDefinitionFileQueryService, WorkflowDefinitionQueryService, WorkflowSource,
};
#[cfg(unix)]
#[test]
pub fn test_workflow探索失敗_定義とsourceをbuiltinへ変換しない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let name = releashd::test_support::integration::workflow::list_builtin_workflows()[0]
        .name
        .clone();
    let path = dir.path().join(format!("{name}.yml"));
    std::os::unix::fs::symlink(&path, &path).unwrap();
    // When
    let definition = repository.get(&name);
    // Then
    assert!(definition.is_err());
}

#[test]
pub fn test_workflow探索_定義がないと未設定を返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    // When
    let definition = repository.find(Some("missing")).unwrap();
    assert!(repository.find(None).unwrap().is_none());
    assert!(matches!(
        repository.get("missing"),
        Err(WorkflowError::NotFound(_))
    ));
    // Then
    assert!(definition.is_none());
}
use releashd::test_support::integration::workflow::WorkflowDefinitionFileRepository;

use releashd::test_support::integration::workflow::WorkflowError;

use std::fs;

use tempfile::TempDir;

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

#[test]
pub fn test_定義取得と一覧_builtinと同名の利用者ファイルはfile由来() {
    // Given
    let workflows = TempDir::new().unwrap();
    let facets = TempDir::new().unwrap();
    seed_instruction_facet(&facets);
    let repository = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
    let query = WorkflowDefinitionFileQueryService::new(workflows.path(), facets.path());
    let name = releashd::test_support::integration::workflow::list_builtin_workflows()[0]
        .name
        .clone();
    let text = source(&name);
    fs::write(workflows.path().join(format!("{name}.yml")), &text).unwrap();
    // When
    let definition = repository.get(&name).unwrap();
    let summaries = query.list_workflows(&[]).unwrap();
    // Then
    assert!(
        matches!(definition.source(), WorkflowSource::File { path } if path == &workflows.path().join(format!("{name}.yml")))
    );
    assert_eq!(
        summaries
            .iter()
            .filter(|summary| summary.name == name)
            .count(),
        1
    );
    assert!(
        !summaries
            .iter()
            .find(|summary| summary.name == name)
            .unwrap()
            .builtin
    );
}

#[test]
fn test_定義取得_同梱定義はbuiltin由来() {
    // Given
    let dir = TempDir::new().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());
    let name = releashd::test_support::integration::workflow::list_builtin_workflows()[0]
        .name
        .clone();
    // When
    let definition = repository.get(&name).unwrap();
    // Then
    assert_eq!(definition.source(), &WorkflowSource::Builtin);
    assert!(definition.content().is_ok());
    assert!(
        query
            .list_workflows(&[])
            .unwrap()
            .iter()
            .find(|summary| summary.name == name)
            .unwrap()
            .builtin
    );
}

#[test]
fn test_定義取得_解釈できない利用者ファイルはentityの失敗に残る() {
    // Given
    let dir = TempDir::new().unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());
    fs::write(dir.path().join("invalid.yml"), "nodes: [").unwrap();
    // When
    let definition = repository.get("invalid").unwrap();
    // Then
    assert!(matches!(definition.source(), WorkflowSource::File { .. }));
    let error = definition.content().unwrap_err();
    assert!(matches!(error, WorkflowError::Diagnostics(_)));
    let summaries = query.list_workflows(&[]).unwrap();
    let summary = summaries
        .iter()
        .find(|summary| summary.name == "invalid")
        .unwrap();
    assert_eq!(summary.description, "Invalid workflow definition");
    assert_eq!(
        summary.failure,
        Some(releashd::test_support::integration::platform::WorkFailure::from_error(error))
    );
}

#[test]
fn test_定義取得_読めない利用者ファイルは失敗を持つentityになる() {
    // Given
    let dir = TempDir::new().unwrap();
    let name = releashd::test_support::integration::workflow::list_builtin_workflows()[0]
        .name
        .clone();
    let path = dir.path().join(format!("{name}.yml"));
    fs::create_dir(&path).unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());
    // When
    let definition = repository.get(&name).unwrap();
    // Then
    assert!(
        matches!(definition.source(), WorkflowSource::File { path: source } if source == &path)
    );
    assert!(matches!(
        definition.content(),
        Err(WorkflowError::Technical(_))
    ));
    assert!(query
        .list_workflows(&[])
        .unwrap()
        .iter()
        .find(|row| row.name == name)
        .unwrap()
        .failure
        .is_some());
}

#[test]
fn test_workflow一覧_同名のyamlとluaを一行に畳む() {
    // Given
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("duplicated.yml"),
        "name: duplicated\ndescription: YAML duplicate\nnodes:\n  main: {command: 'true'}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("duplicated.lua"),
        r#"
local r = require("releash")
return r.workflow{
  name = "duplicated", description = "Lua duplicate",
  main = r.command{ command = "true" },
}
"#,
    )
    .unwrap();
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());
    // When
    let list = query.list_workflows(&[]).unwrap();
    let matches = list
        .iter()
        .filter(|row| row.name == "duplicated")
        .collect::<Vec<_>>();
    // Then
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].description,
        releashd::test_support::integration::workflow::DUPLICATE_NAME_DESCRIPTION
    );
}

#[test]
fn test_builtin解決失敗_entityと一覧の失敗に保持し他の定義を返す() {
    use releashd::test_support::integration::platform::WorkFailure;
    // Given
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("policies")).unwrap();
    fs::write(dir.path().join("policies/coding.md"), "Use {{ missing }}").unwrap();
    fs::write(
        dir.path().join("healthy.yml"),
        "name: healthy\ndescription: Healthy\nnodes:\n  main: {command: 'true'}\n",
    )
    .unwrap();
    let repository = WorkflowDefinitionFileRepository::new(dir.path(), dir.path());
    let query = WorkflowDefinitionFileQueryService::new(dir.path(), dir.path());
    // When
    let definition = repository.get("02_implement-existing-spec").unwrap();
    let summaries = query.list_workflows(&[]).unwrap();
    // Then
    assert_eq!(definition.source(), &WorkflowSource::Builtin);
    let failure = WorkFailure::from_error(definition.content().unwrap_err());
    assert!(failure.message.contains("missing"), "{failure:?}");
    let builtin = summaries
        .iter()
        .find(|row| row.name == "02_implement-existing-spec")
        .unwrap();
    assert!(builtin.builtin);
    assert_eq!(builtin.failure.as_ref(), Some(&failure));
    assert!(summaries
        .iter()
        .any(|row| row.name == "healthy" && row.failure.is_none()));
    assert_eq!(
        summaries.len(),
        releashd::test_support::integration::workflow::list_builtin_workflows().len() + 1
    );
}
