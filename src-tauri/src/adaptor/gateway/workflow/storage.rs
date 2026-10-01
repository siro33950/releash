use super::builtin;
use super::diagnostics;
use super::domain_mapping::workflow_definition_to_domain;
use super::facet;
use super::schema::{Summary, WorkflowDefinitionYaml};
use crate::domain::workflow::validation::{self, ValidationError};
use crate::domain::workflow::WorkflowSourceFormat;
use crate::usecase::workflow::diagnostic_dto::{DiagnosticItem, DiagnosticStage, Severity};
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

impl From<StorageError> for crate::domain::workflow::WorkflowError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Io(error) => Self::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: error.to_string(),
            }),
            StorageError::YamlSerialize(error) => {
                Self::Technical(crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                })
            }
            StorageError::NotFound { name } => Self::NotFound(name),
            StorageError::BuiltinProtected { name } => Self::InvalidState(name),
            error @ (StorageError::YamlDeserialize(_)
            | StorageError::Diagnostics(_)
            | StorageError::Validation(_)
            | StorageError::FacetResolution(_)) => Self::Validation(error.to_string()),
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

pub(crate) fn workflow_files(
    dir: &Path,
) -> Result<Vec<(String, std::path::PathBuf)>, StorageError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if workflow_source_format(&path).is_some() {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                files.push((name.to_string(), path));
            }
        }
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(files)
}

fn list_file_summaries<T>(
    dir: &Path,
    loader: impl Fn(&Path) -> Result<T, StorageError>,
    to_summary: impl Fn(T) -> Summary,
) -> Result<Vec<Summary>, StorageError> {
    let mut summaries = Vec::new();
    for (stem, path) in workflow_files(dir)? {
        match loader(&path) {
            Ok(item) => {
                let mut summary = to_summary(item);
                summary.name = stem.to_string();
                summary.source_format = workflow_source_format(&path).unwrap_or_default();
                summaries.push(summary);
            }
            Err(e) => {
                summaries.push(Summary {
                    name: stem.to_string(),
                    description: if matches!(&e, StorageError::Diagnostics(_)) {
                        "Invalid workflow definition".into()
                    } else {
                        String::new()
                    },
                    failure: Some(crate::domain::failure::WorkFailure::from_error(
                        &crate::domain::workflow::WorkflowError::from(e),
                    )),
                    builtin: false,
                    is_running: false,
                    source_format: workflow_source_format(&path).unwrap(),
                });
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
                if previous.failure.is_none() {
                    previous.failure = summary.failure;
                }
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
        let diagnosis = diagnose_workflow_file(path, &content, dir, facets_base_dir);
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
        Ok(workflow)
    };
    let mut summaries = list_file_summaries(dir, load_for_listing, |wf| Summary {
        failure: None,
        name: wf.name.clone(),
        description: wf.description,
        builtin: false, // name がファイル stem で上書きされた後に再計算する
        is_running: false,
        source_format: WorkflowSourceFormat::Yaml,
    })?;
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
    let mut existing = Vec::new();
    for path in paths {
        if path.try_exists()? {
            existing.push(path);
        }
    }
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
#[path = "storage_test.rs"]
mod storage_tests;
