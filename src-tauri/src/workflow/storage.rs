use super::builtin;
use super::schema::{Summary, Workflow};
use super::validation::{self, ValidationError};
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    YamlDeserialize(serde_saphyr::Error),
    YamlSerialize(serde_saphyr::ser::Error),
    Validation(ValidationError),
    NotFound { name: String },
    BuiltinProtected { name: String },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/Oエラー: {e}"),
            Self::YamlDeserialize(e) => write!(f, "YAMLパース失敗: {e}"),
            Self::YamlSerialize(e) => write!(f, "YAMLシリアライズ失敗: {e}"),
            Self::Validation(e) => write!(f, "{e}"),
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
            Self::Validation(e) => Some(e),
            _ => None,
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

impl Serialize for StorageError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub fn workflows_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("releash")
        .join("workflows")
}

pub fn facets_base_dir() -> PathBuf {
    workflows_dir()
}

pub fn ensure_dir(dir: &Path) -> Result<(), StorageError> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_workflow(dir: &Path, workflow: &Workflow) -> Result<(), StorageError> {
    validation::validate(workflow)?;

    ensure_dir(dir)?;

    let content = serde_saphyr::to_string(workflow)?;

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

pub fn load_workflow(path: &Path) -> Result<Workflow, StorageError> {
    let content = fs::read_to_string(path)?;
    let workflow: Workflow = serde_saphyr::from_str(&content)?;
    validation::validate(&workflow)?;
    Ok(workflow)
}

fn list_yml_summaries<T, E: fmt::Display>(
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
        if path.extension().and_then(|e| e.to_str()) == Some("yml") {
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
                    summaries.push(summary);
                }
                Err(e) => {
                    log::warn!("{label}読み込みスキップ: {}: {e}", path.display());
                }
            }
        }
    }

    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(summaries)
}

pub fn list_workflows(dir: &Path) -> Result<Vec<Summary>, StorageError> {
    let mut summaries = list_yml_summaries(
        dir,
        load_workflow,
        |wf| Summary {
            name: wf.name,
            description: wf.description,
            builtin: wf.builtin,
        },
        "ワークフロー",
    )?;
    for s in builtin::list_builtin_workflows() {
        if !summaries.iter().any(|existing| existing.name == s.name) {
            summaries.push(s);
        }
    }
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(summaries)
}

pub fn resolve_workflow_path(dir: &Path, name: &str) -> Result<PathBuf, StorageError> {
    validation::validate_name(name)?;
    let file_path = dir.join(format!("{name}.yml"));
    if !file_path.exists() {
        return Err(StorageError::NotFound {
            name: name.to_string(),
        });
    }
    Ok(file_path)
}

pub fn delete_workflow(dir: &Path, name: &str) -> Result<(), StorageError> {
    validation::validate_name(name)?;
    let file_path = dir.join(format!("{name}.yml"));
    if file_path.exists() {
        fs::remove_file(&file_path)?;
        return Ok(());
    }
    if builtin::get_builtin_workflow(name).is_some() {
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
    use crate::workflow::schema::{Step, StepMode};
    use tempfile::TempDir;

    fn sample_workflow(name: &str, builtin: bool) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: format!("{name} workflow"),
            builtin,
            steps: vec![Step {
                name: "step1".to_string(),
                mode: Some(StepMode::Auto),
                persona: None,
                policy: None,
                knowledge: None,
                instruction: Some("implement".to_string()),
                output_contract: None,
                rules: vec![],
                cycle_guard: None,
                pass_previous_response: None,
                pass_output_from: None,
                collect: None,
                parallel: None,
                aggregate: None,
                resets_cycle_for: None,
            }],
        }
    }

    #[test]
    fn save_and_load_workflow() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = sample_workflow("my-workflow", false);
        save_workflow(dir, &wf).unwrap();

        let file_path = dir.join("my-workflow.yml");
        assert!(file_path.exists());

        let loaded = load_workflow(&file_path).unwrap();
        assert_eq!(loaded.name, "my-workflow");
        assert_eq!(loaded.description, "my-workflow workflow");
        assert_eq!(loaded.steps.len(), 1);
    }

    #[test]
    fn list_workflows_returns_sorted_summaries() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        save_workflow(dir, &sample_workflow("charlie", false)).unwrap();
        save_workflow(dir, &sample_workflow("alpha", false)).unwrap();
        save_workflow(dir, &sample_workflow("bravo", false)).unwrap();

        let list = list_workflows(dir).unwrap();
        // ディスク3件 + ビルトイン(spec-driven-development) = 4件
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "bravo");
        assert_eq!(list[2].name, "charlie");
        assert_eq!(list[3].name, "spec-driven-development");
        assert!(list[3].builtin);
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
        // ディスク1件(renamed) + ビルトイン(spec-driven-development) = 2件
        let disk_entry = list.iter().find(|s| s.name == "renamed").unwrap();
        // Summary.nameはファイルstem（renamed）であるべき、YAML本文（original）ではない
        assert_eq!(disk_entry.name, "renamed");
    }

    #[test]
    fn list_workflows_empty_dir_includes_builtins() {
        let tmp = TempDir::new().unwrap();
        let list = list_workflows(tmp.path()).unwrap();
        assert!(list.iter().any(|s| s.name == "spec-driven-development"));
    }

    #[test]
    fn list_workflows_nonexistent_dir_includes_builtins() {
        let list = list_workflows(Path::new("/nonexistent/path")).unwrap();
        assert!(list.iter().any(|s| s.name == "spec-driven-development"));
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

        let result = delete_workflow(dir, "spec-driven-development");
        assert!(matches!(
            result.unwrap_err(),
            StorageError::BuiltinProtected { ref name } if name == "spec-driven-development"
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
    fn resolve_workflow_path_empty_name() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "");
        assert!(result.is_err());
    }
}
