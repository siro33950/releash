use super::builtin;
use super::diagnostics;
use super::domain_mapping::workflow_definition_to_domain;
use super::facet;
use super::schema::{Summary, WorkflowDefinitionYaml};
use crate::adaptor::protocol::workflow::{DiagnosticItem, DiagnosticStage, Severity};
use crate::domain::workflow::validation::{self, ValidationError};
use crate::domain::workflow::WorkflowSourceFormat;
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    YamlDeserialize(serde_saphyr::Error),
    YamlSerialize(serde_saphyr::ser::Error),
    Diagnostics(Vec<DiagnosticItem>),
    Validation(ValidationError),
    FacetResolution(facet::FacetError),
    NotFound { name: String },
    BuiltinProtected { name: String },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/Oエラー: {e}"),
            Self::YamlDeserialize(e) => write!(f, "YAMLパース失敗: {e}"),
            Self::YamlSerialize(e) => write!(f, "YAMLシリアライズ失敗: {e}"),
            Self::Diagnostics(items) => {
                let messages = items
                    .iter()
                    .map(|item| format!("{}: {}", item.code, item.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "workflow_diagnostics: {messages}")
            }
            Self::Validation(e) => write!(f, "validation_error: {e}"),
            Self::FacetResolution(e) => write!(f, "facet解決失敗: {e}"),
            Self::NotFound { name } => {
                write!(f, "ワークフロー '{name}' が見つかりません")
            }
            Self::BuiltinProtected { name } => {
                write!(f, "ビルトインワークフロー '{name}' は削除できません")
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::YamlDeserialize(e) => Some(e),
            Self::YamlSerialize(e) => Some(e),
            Self::Diagnostics(_) => None,
            Self::Validation(e) => Some(e),
            Self::FacetResolution(e) => Some(e),
            Self::NotFound { .. } => None,
            Self::BuiltinProtected { .. } => None,
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_saphyr::Error> for StorageError {
    fn from(e: serde_saphyr::Error) -> Self {
        Self::YamlDeserialize(e)
    }
}

impl From<serde_saphyr::ser::Error> for StorageError {
    fn from(e: serde_saphyr::ser::Error) -> Self {
        Self::YamlSerialize(e)
    }
}

impl From<ValidationError> for StorageError {
    fn from(e: ValidationError) -> Self {
        Self::Validation(e)
    }
}

impl From<facet::FacetError> for StorageError {
    fn from(e: facet::FacetError) -> Self {
        Self::FacetResolution(e)
    }
}

impl Serialize for StorageError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// 同じ名前を複数の形式が宣言した一覧エントリの説明。
const DUPLICATE_NAME_DESCRIPTION: &str = "Duplicate workflow definition";

pub fn workflows_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("releash")
        .join("workflows")
}

pub fn ensure_dir(dir: &Path) -> Result<(), StorageError> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_workflow(dir: &Path, workflow: &WorkflowDefinitionYaml) -> Result<(), StorageError> {
    validate_workflow_definition(workflow)?;

    ensure_dir(dir)?;

    // ディスクに保存する際は builtin フラグを常に false にする
    // （builtin 判定はコード側で行うため、YAMLに書き込まない）
    let mut to_save = workflow.clone();
    to_save.builtin = false;
    let content = serde_saphyr::to_string(&to_save)?;

    let file_path = dir.join(format!("{}.yml", workflow.name));
    let tmp_path = dir.join(format!(
        "{}.yml.{}.tmp",
        workflow.name,
        uuid::Uuid::new_v4()
    ));

    fs::write(&tmp_path, &content)?;
    if let Err(e) = fs::rename(&tmp_path, &file_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    Ok(())
}

pub fn parse_workflow_source(
    content: &str,
    facets_base_dir: &Path,
) -> Result<WorkflowDefinitionYaml, StorageError> {
    let diagnosis = diagnostics::diagnose_workflow_source(content, None);
    if diagnosis.has_errors() {
        return Err(StorageError::Diagnostics(diagnosis.diagnostics));
    }
    let mut workflow = diagnosis.workflow.ok_or_else(|| {
        StorageError::Diagnostics(vec![DiagnosticItem::new(
            "WFS001",
            Severity::Error,
            DiagnosticStage::ParseShape,
            None,
            "workflow source could not be parsed",
        )])
    })?;
    workflow.builtin = builtin::is_builtin_workflow(&workflow.name);
    let _ = resolve_and_validate_workflow_facets(&workflow, facets_base_dir)?;
    Ok(workflow)
}

pub fn load_workflow_source(dir: &Path, name: &str) -> Result<String, StorageError> {
    let path = resolve_workflow_path(dir, name)?;
    Ok(fs::read_to_string(path)?)
}

pub fn save_workflow_source(
    dir: &Path,
    facets_base_dir: &Path,
    content: &str,
) -> Result<WorkflowDefinitionYaml, StorageError> {
    let mut workflow = parse_workflow_source(content, facets_base_dir)?;
    ensure_dir(dir)?;

    let file_path = dir.join(format!("{}.yml", workflow.name));
    let tmp_path = dir.join(format!(
        "{}.yml.{}.tmp",
        workflow.name,
        uuid::Uuid::new_v4()
    ));

    fs::write(&tmp_path, content)?;
    if let Err(e) = fs::rename(&tmp_path, &file_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    workflow.builtin = false;
    Ok(workflow)
}

trait WorkflowDefinitionLoader {
    fn diagnose(
        &self,
        path: &Path,
        content: &str,
        workflows_dir: &Path,
        facets_base_dir: &Path,
    ) -> diagnostics::WorkflowSourceDiagnostics;
}

struct YamlWorkflowDefinitionLoader;

impl WorkflowDefinitionLoader for YamlWorkflowDefinitionLoader {
    fn diagnose(
        &self,
        path: &Path,
        content: &str,
        _workflows_dir: &Path,
        _facets_base_dir: &Path,
    ) -> diagnostics::WorkflowSourceDiagnostics {
        diagnostics::diagnose_workflow_source(
            content,
            path.file_stem().and_then(|stem| stem.to_str()),
        )
    }
}

struct LuaWorkflowDefinitionLoader;

impl WorkflowDefinitionLoader for LuaWorkflowDefinitionLoader {
    fn diagnose(
        &self,
        path: &Path,
        content: &str,
        workflows_dir: &Path,
        facets_base_dir: &Path,
    ) -> diagnostics::WorkflowSourceDiagnostics {
        diagnostics::diagnose_lua_workflow_source(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<unknown>.lua"),
            content,
            workflows_dir,
            facets_base_dir,
            path.file_stem().and_then(|stem| stem.to_str()),
        )
    }
}

pub(crate) fn diagnose_workflow_file(
    path: &Path,
    content: &str,
    workflows_dir: &Path,
    facets_base_dir: &Path,
) -> diagnostics::WorkflowSourceDiagnostics {
    let loader: &dyn WorkflowDefinitionLoader = match workflow_source_format(path) {
        Some(WorkflowSourceFormat::Yaml) => &YamlWorkflowDefinitionLoader,
        Some(WorkflowSourceFormat::Lua) => &LuaWorkflowDefinitionLoader,
        None => {
            return diagnostics::WorkflowSourceDiagnostics {
                workflow: None,
                diagnostics: vec![DiagnosticItem::new(
                    "WFS002",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    None,
                    format!("unsupported workflow source extension: {}", path.display()),
                )],
            };
        }
    };
    loader.diagnose(path, content, workflows_dir, facets_base_dir)
}

/// workflow 定義ファイルを読み込み、facet 参照を解決した上で validation する。
///
/// [02] schema 境界: load 経路で `facet.rs` を呼び、session / fanout child の
/// gateway read model に解決済み内容を格納し、facet 本文の Artifact 参照も検証する。
/// 実行用 Workflow には未解決 ref を残さない（schema 層は ref キーを保持しつつ、
/// 実行系は resolved cache から直接合成する）。
pub fn load_workflow(
    path: &Path,
    facets_base_dir: &Path,
) -> Result<WorkflowDefinitionYaml, StorageError> {
    let content = fs::read_to_string(path)?;
    let diagnosis = diagnose_workflow_file(
        path,
        &content,
        path.parent().unwrap_or_else(|| Path::new(".")),
        facets_base_dir,
    );
    if diagnosis.has_errors() {
        return Err(StorageError::Diagnostics(diagnosis.diagnostics));
    }
    let mut workflow = diagnosis.workflow.ok_or_else(|| {
        StorageError::Diagnostics(vec![DiagnosticItem::new(
            "WFS001",
            Severity::Error,
            DiagnosticStage::ParseShape,
            None,
            "workflow source could not be parsed",
        )])
    })?;
    // YAMLの builtin フラグは無視し、コード側（builtin.rs）で判定する
    workflow.builtin = builtin::is_builtin_workflow(&workflow.name);
    let _ = resolve_and_validate_workflow_facets(&workflow, facets_base_dir)?;
    Ok(workflow)
}

fn list_file_summaries<T, E: fmt::Display>(
    dir: &Path,
    loader: impl Fn(&Path) -> Result<T, E>,
    to_summary: impl Fn(T) -> Summary,
    label: &str,
) -> Result<Vec<Summary>, StorageError> {
    if !dir.exists() {
        return Ok(vec![]);
    }

    let entries = fs::read_dir(dir)?;

    let mut summaries = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if workflow_source_format(&path).is_some() {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                log::warn!(
                    "{label}読み込みスキップ: {}: 無効なファイル名",
                    path.display()
                );
                continue;
            };
            match loader(&path) {
                Ok(item) => {
                    let mut summary = to_summary(item);
                    summary.name = stem.to_string();
                    summary.source_format = workflow_source_format(&path).unwrap_or_default();
                    summaries.push(summary);
                }
                Err(e) => {
                    log::warn!("{label}読み込みスキップ: {}: {e}", path.display());
                }
            }
        }
    }

    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(collapse_duplicate_names(summaries))
}

/// 同じ名前を複数の形式が宣言した状態は `resolve_workflow_path` が `WFS006` として
/// 拒否する。一覧でも 1 件へ畳み込み、選べる行と実行できる定義を一致させる。
fn collapse_duplicate_names(summaries: Vec<Summary>) -> Vec<Summary> {
    let mut collapsed: Vec<Summary> = Vec::with_capacity(summaries.len());
    for summary in summaries {
        match collapsed.last_mut() {
            Some(previous) if previous.name == summary.name => {
                previous.description = DUPLICATE_NAME_DESCRIPTION.to_string();
            }
            _ => collapsed.push(summary),
        }
    }
    collapsed
}

#[cfg(test)]
pub fn list_workflows(dir: &Path) -> Result<Vec<Summary>, StorageError> {
    list_workflows_with_facets(dir, dir)
}

pub(crate) fn list_workflows_with_facets(
    dir: &Path,
    facets_base_dir: &Path,
) -> Result<Vec<Summary>, StorageError> {
    // list 用途では facet 未解決でも一覧表示に支障がないため、deserialize+validate のみを行う。
    // facet 解決が必要な実行系経路は明示的に `load_workflow` を呼ぶ。
    let load_for_listing = |path: &Path| -> Result<WorkflowDefinitionYaml, StorageError> {
        let content = fs::read_to_string(path)?;
        let stem = path.file_stem().and_then(|stem| stem.to_str());
        let diagnosis = diagnose_workflow_file(path, &content, dir, facets_base_dir);
        if diagnosis.has_errors() {
            return Ok(WorkflowDefinitionYaml {
                name: stem.unwrap_or("invalid").to_string(),
                description: "Invalid workflow definition".to_string(),
                builtin: false,
                schemas: Default::default(),
                nodes: Vec::new(),
                ..Default::default()
            });
        }
        let mut workflow = diagnosis.workflow.ok_or_else(|| {
            StorageError::Diagnostics(vec![DiagnosticItem::new(
                "WFS001",
                Severity::Error,
                DiagnosticStage::ParseShape,
                None,
                "workflow source could not be parsed",
            )])
        })?;
        workflow.builtin = builtin::is_builtin_workflow(&workflow.name);
        Ok(workflow)
    };
    let mut summaries = list_file_summaries(
        dir,
        load_for_listing,
        |wf| Summary {
            name: wf.name.clone(),
            description: wf.description,
            builtin: false, // name がファイル stem で上書きされた後に再計算する
            is_running: false,
            source_format: WorkflowSourceFormat::Yaml,
        },
        "ワークフロー",
    )?;
    // ファイル stem で上書きされた最終的な name に基づいて builtin を再計算
    for s in &mut summaries {
        s.builtin = builtin::is_builtin_workflow(&s.name);
    }
    for s in builtin::list_builtin_workflows() {
        if !summaries.iter().any(|existing| existing.name == s.name) {
            summaries.push(s);
        }
    }
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(summaries)
}

fn validate_workflow_definition(workflow: &WorkflowDefinitionYaml) -> Result<(), StorageError> {
    let diagnostics = diagnostics::diagnose_workflow_definition(workflow, None);
    if diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        return Err(StorageError::Diagnostics(diagnostics));
    }
    Ok(())
}

pub(crate) fn resolve_and_validate_workflow_facets(
    workflow: &WorkflowDefinitionYaml,
    facets_base_dir: &Path,
) -> Result<facet::WorkflowFacetContents, StorageError> {
    let reference_diagnostics =
        diagnostics::diagnose_workflow_facet_references(workflow, facets_base_dir)?;
    if reference_diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        return Err(StorageError::Diagnostics(reference_diagnostics));
    }
    let facet_contents = facet::resolve_workflow_facets(workflow, facets_base_dir)?;
    validate_resolved_facet_references(workflow, &facet_contents)?;
    Ok(facet_contents)
}

fn validate_resolved_facet_references(
    workflow: &WorkflowDefinitionYaml,
    facet_contents: &facet::WorkflowFacetContents,
) -> Result<(), ValidationError> {
    let domain_workflow = workflow_definition_to_domain(workflow);
    for (node_name, contents) in facet_contents.iter_node_contents() {
        let Some(node) = domain_workflow.node_by_name(node_name) else {
            continue;
        };
        for content in contents
            .policy
            .iter()
            .chain(contents.knowledge.iter())
            .chain(contents.instruction.iter())
        {
            if let Some(err) =
                validation::validate_template_references_for_node(&domain_workflow, node, content)
                    .into_iter()
                    .next()
            {
                return Err(err);
            }
        }
    }
    Ok(())
}

pub fn resolve_workflow_path(dir: &Path, name: &str) -> Result<PathBuf, StorageError> {
    validation::validate_name(name)?;
    let paths = [
        dir.join(format!("{name}.yml")),
        dir.join(format!("{name}.lua")),
    ];
    let existing = paths
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    match existing.as_slice() {
        [] => Err(StorageError::NotFound {
            name: name.to_string(),
        }),
        [path] => Ok(path.clone()),
        _ => Err(StorageError::Diagnostics(vec![DiagnosticItem::new(
            "WFS006",
            Severity::Error,
            DiagnosticStage::ParseShape,
            None,
            format!("workflow name '{name}' is duplicated"),
        )])),
    }
}

pub(crate) fn workflow_source_format(path: &Path) -> Option<WorkflowSourceFormat> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("yml") => Some(WorkflowSourceFormat::Yaml),
        Some("lua") => Some(WorkflowSourceFormat::Lua),
        _ => None,
    }
}

pub fn delete_workflow(dir: &Path, name: &str) -> Result<(), StorageError> {
    validation::validate_name(name)?;
    match resolve_workflow_path(dir, name) {
        Ok(file_path) => {
            fs::remove_file(file_path)?;
            return Ok(());
        }
        Err(StorageError::NotFound { .. }) => {}
        Err(error) => return Err(error),
    }
    // builtin として認識できた場合（load 成功で Some）は削除を拒否する。
    // load 失敗（Err）の場合も「builtin として存在し得る」とみなし、安全側に倒して
    // 保護する（誤削除防止 / load 失敗の解決は別経路の責務）。
    if matches!(
        builtin::load_builtin_workflow_resolved(name),
        Ok(Some(_)) | Err(_)
    ) {
        return Err(StorageError::BuiltinProtected {
            name: name.to_string(),
        });
    }
    Err(StorageError::NotFound {
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, SessionSpec,
    };
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
    fn save_and_load_workflow() {
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
    fn save_sourceとloadは未知fieldを拒否する() {
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
    fn list_workflows_returns_sorted_summaries() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        save_workflow(dir, &sample_workflow("charlie", false)).unwrap();
        save_workflow(dir, &sample_workflow("alpha", false)).unwrap();
        save_workflow(dir, &sample_workflow("bravo", false)).unwrap();

        let list = list_workflows(dir).unwrap();
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
            let entry = list.iter().find(|s| s.name == name).unwrap_or_else(|| {
                panic!("builtin workflow '{name}' must be present in merged list")
            });
            assert!(entry.builtin, "builtin '{name}' must be marked builtin");
        }
    }

    #[test]
    fn list_workflows_uses_file_stem_not_yaml_name() {
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
    fn list_workflows_keeps_invalid_files_as_diagnostic_only_summaries() {
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
    fn list_workflows_collapses_same_name_across_formats() {
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

        let list = list_workflows(dir).unwrap();
        let matched = list
            .iter()
            .filter(|summary| summary.name == "duplicated")
            .collect::<Vec<_>>();

        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].description, DUPLICATE_NAME_DESCRIPTION);
        assert!(resolve_workflow_path(dir, "duplicated").is_err());
    }

    #[test]
    fn list_workflows_empty_dir_includes_builtins() {
        let tmp = TempDir::new().unwrap();
        let list = list_workflows(tmp.path()).unwrap();
        for name in builtin_workflow_names() {
            assert!(list.iter().any(|s| s.name == name));
        }
    }

    #[test]
    fn list_workflows_nonexistent_dir_includes_builtins() {
        let list = list_workflows(Path::new("/nonexistent/path")).unwrap();
        for name in builtin_workflow_names() {
            assert!(list.iter().any(|s| s.name == name));
        }
    }

    #[test]
    fn delete_workflow_success() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        save_workflow(dir, &sample_workflow("deleteme", false)).unwrap();
        assert!(dir.join("deleteme.yml").exists());

        delete_workflow(dir, "deleteme").unwrap();
        assert!(!dir.join("deleteme.yml").exists());
    }

    #[test]
    fn delete_builtin_workflow_fails() {
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
    fn delete_nonexistent_workflow_fails() {
        let tmp = TempDir::new().unwrap();
        let result = delete_workflow(tmp.path(), "nope");
        assert!(matches!(
            result.unwrap_err(),
            StorageError::NotFound { ref name } if name == "nope"
        ));
    }

    // --- resolve_workflow_path tests ---

    #[test]
    fn resolve_workflow_path_success() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        save_workflow(dir, &sample_workflow("my-workflow", false)).unwrap();

        let result = resolve_workflow_path(dir, "my-workflow");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), dir.join("my-workflow.yml"));
    }

    #[test]
    fn resolve_workflow_path_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "nonexistent");
        assert!(matches!(
            result.unwrap_err(),
            StorageError::NotFound { ref name } if name == "nonexistent"
        ));
    }

    #[test]
    fn resolve_workflow_path_invalid_name() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "../evil");
        assert!(matches!(result.unwrap_err(), StorageError::Validation(_)));
    }

    #[test]
    fn validation_error_display_has_stable_kind_prefix() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "../evil");
        assert!(result
            .unwrap_err()
            .to_string()
            .starts_with("validation_error:"));
    }

    #[test]
    fn resolve_workflow_path_empty_name() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "");
        assert!(result.is_err());
    }

    // --- save_workflow ビルトインガードテスト ---

    #[test]
    fn save_builtin_name_workflow_is_prevented_by_guard() {
        // commands.rs でビルトイン名のチェックを行うため、storage層ではそのチェックをシミュレート
        assert!(builtin::is_builtin_workflow(&first_builtin_workflow_name()));
        // カスタム名はOK
        assert!(!builtin::is_builtin_workflow("my-custom"));
    }

    #[test]
    fn save_workflow_rename_to_existing_name_detected() {
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
    fn save_workflow_new_with_duplicate_name_detected() {
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
    fn delete_open_workflow_in_editor_builtin_guard() {
        // ビルトインは削除不可（既にdelete_builtin_workflow_failsでテスト済み）
        // open_workflow_in_editor でもビルトインは弾かれる（カスタムファイルのみ）
        let tmp = TempDir::new().unwrap();
        let builtin_name = first_builtin_workflow_name();
        let result = resolve_workflow_path(tmp.path(), &builtin_name);
        assert!(matches!(result.unwrap_err(), StorageError::NotFound { .. }));
    }

    // --- validate_name 先頭文字テスト ---

    #[test]
    fn validate_name_rejects_leading_special_chars() {
        assert!(validation::validate_name("-starts-with-dash").is_err());
        assert!(validation::validate_name("_starts-with-underscore").is_err());
        assert!(validation::validate_name("a-valid-name").is_ok());
        assert!(validation::validate_name("1-starts-with-digit").is_ok());
    }

    /// [02] schema 境界: `storage::load_workflow` は load 経路で facet を解決し、
    /// gateway read model として検証する。
    #[test]
    fn load_workflow_resolves_facets_into_gateway_read_model() {
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
    fn parse_and_load_validate_resolved_facet_artifact_references() {
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
    fn parse_and_load_validate_every_resolved_knowledge_body() {
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
    fn load_workflow_resolves_all_three_facets_for_node_and_child() {
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
    fn load_workflow_rejects_missing_facet() {
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
    fn load_workflow_resolves_builtin_facet_with_broken_inventory() {
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

        let workflow = load_workflow(&file_path, dir).unwrap();

        assert_eq!(workflow.name, "builtin-facet-with-broken-inventory");
    }

    /// Artifact template references load without a workflow-level variables section.
    #[test]
    fn load_workflow_accepts_request_artifact_reference() {
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
    fn loads_and_lists_lua_workflow_with_source_format() {
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

        let workflow = load_workflow(&path, tmp.path()).unwrap();
        let summaries = list_workflows(tmp.path()).unwrap();

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
    fn rejects_lua_workflow_name_that_differs_from_file_stem() {
        let tmp = TempDir::new().unwrap();
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
        let StorageError::Diagnostics(items) = error else {
            panic!("name mismatch must produce diagnostics");
        };

        let mismatch = items.iter().find(|item| item.code == "WFS006").unwrap();
        let span = mismatch.span.as_ref().unwrap();
        assert_eq!(span.source.as_deref(), Some("file-name.lua"));
        assert_eq!(span.start_line, 6);
    }

    #[test]
    fn rejects_ambiguous_yaml_and_lua_files_with_same_stem() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("duplicate.yml"), "name: duplicate").unwrap();
        std::fs::write(tmp.path().join("duplicate.lua"), "return nil").unwrap();

        let error = resolve_workflow_path(tmp.path(), "duplicate").unwrap_err();
        let StorageError::Diagnostics(items) = error else {
            panic!("duplicate source files must produce diagnostics");
        };

        assert!(items.iter().any(|item| item.code == "WFS006"));
    }
}
