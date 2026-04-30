use super::prompt_schema::PromptTemplate;
use super::schema::{Summary, Workflow};
use super::validation;
use std::fs;
use std::path::{Path, PathBuf};

pub fn workflows_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("releash")
        .join("workflows")
}

pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("ワークフローディレクトリの作成に失敗: {e}"))?;
    }
    Ok(())
}

pub fn save_workflow(dir: &Path, workflow: &Workflow) -> Result<(), String> {
    validation::validate(workflow)?;

    ensure_dir(dir)?;

    let content =
        serde_saphyr::to_string(workflow).map_err(|e| format!("YAMLシリアライズ失敗: {e}"))?;

    let file_path = dir.join(format!("{}.yml", workflow.name));
    let tmp_path = file_path.with_extension("yml.tmp");

    fs::write(&tmp_path, &content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
    fs::rename(&tmp_path, &file_path).map_err(|e| format!("ファイルのリネーム失敗: {e}"))?;

    Ok(())
}

pub fn load_workflow(path: &Path) -> Result<Workflow, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("ファイル読み込み失敗: {e}"))?;
    let workflow: Workflow =
        serde_saphyr::from_str(&content).map_err(|e| format!("YAMLパース失敗: {e}"))?;
    validation::validate(&workflow)?;
    Ok(workflow)
}

fn list_yml_summaries<T>(
    dir: &Path,
    loader: impl Fn(&Path) -> Result<T, String>,
    to_summary: impl Fn(T) -> Summary,
    label: &str,
) -> Result<Vec<Summary>, String> {
    if !dir.exists() {
        return Ok(vec![]);
    }

    let entries = fs::read_dir(dir).map_err(|e| format!("ディレクトリ読み込み失敗: {e}"))?;

    let mut summaries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("エントリ読み込み失敗: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yml") {
            match loader(&path) {
                Ok(item) => summaries.push(to_summary(item)),
                Err(e) => {
                    log::warn!("{label}読み込みスキップ: {}: {e}", path.display());
                }
            }
        }
    }

    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(summaries)
}

pub fn list_workflows(dir: &Path) -> Result<Vec<Summary>, String> {
    list_yml_summaries(
        dir,
        load_workflow,
        |wf| Summary {
            name: wf.name,
            description: wf.description,
            builtin: wf.builtin,
        },
        "ワークフロー",
    )
}

pub fn resolve_workflow_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validation::validate_name(name)?;
    let file_path = dir.join(format!("{name}.yml"));
    if !file_path.exists() {
        return Err(format!("ワークフロー '{name}' が見つかりません"));
    }
    Ok(file_path)
}

pub fn delete_workflow(dir: &Path, name: &str) -> Result<(), String> {
    let file_path = resolve_workflow_path(dir, name)?;

    let workflow = load_workflow(&file_path)?;
    if workflow.builtin {
        return Err(format!("ビルトインワークフロー '{name}' は削除できません"));
    }

    fs::remove_file(&file_path).map_err(|e| format!("ファイル削除失敗: {e}"))?;
    Ok(())
}

pub fn prompts_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("releash")
        .join("prompts")
}

pub fn load_prompt(path: &Path) -> Result<PromptTemplate, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("ファイル読み込み失敗: {e}"))?;
    let template: PromptTemplate =
        serde_saphyr::from_str(&content).map_err(|e| format!("YAMLパース失敗: {e}"))?;
    validation::validate_prompt_template(&template)?;
    Ok(template)
}

pub fn list_prompts(dir: &Path) -> Result<Vec<Summary>, String> {
    list_yml_summaries(
        dir,
        load_prompt,
        |tpl| Summary {
            name: tpl.name,
            description: tpl.description,
            builtin: tpl.builtin,
        },
        "プロンプト",
    )
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
                mode: StepMode::Auto,
                prompt: "test-prompt".to_string(),
                rules: vec![],
                cycle_guard: None,
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
        save_workflow(dir, &sample_workflow("bravo", true)).unwrap();

        let list = list_workflows(dir).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "bravo");
        assert!(list[1].builtin);
        assert_eq!(list[2].name, "charlie");
    }

    #[test]
    fn list_workflows_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let list = list_workflows(tmp.path()).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn list_workflows_nonexistent_dir() {
        let list = list_workflows(Path::new("/nonexistent/path")).unwrap();
        assert!(list.is_empty());
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

        save_workflow(dir, &sample_workflow("quick-fix", true)).unwrap();

        let result = delete_workflow(dir, "quick-fix");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ビルトインワークフロー"));
        assert!(dir.join("quick-fix.yml").exists());
    }

    #[test]
    fn delete_nonexistent_workflow_fails() {
        let tmp = TempDir::new().unwrap();
        let result = delete_workflow(tmp.path(), "nope");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("見つかりません"));
    }

    // --- Prompt template storage tests ---

    use crate::workflow::prompt_schema::{PromptTemplate, PromptVariable};
    use crate::workflow::validation;

    fn sample_prompt(name: &str, builtin: bool) -> PromptTemplate {
        PromptTemplate {
            name: name.to_string(),
            description: format!("{name} prompt"),
            content: "テスト用プロンプト".to_string(),
            variables: vec![PromptVariable {
                name: "project_name".to_string(),
                description: "プロジェクト名".to_string(),
                default: None,
            }],
            builtin,
        }
    }

    fn save_prompt(dir: &Path, template: &PromptTemplate) -> Result<(), String> {
        validation::validate_prompt_template(template)?;
        ensure_dir(dir)?;
        let content =
            serde_saphyr::to_string(template).map_err(|e| format!("YAMLシリアライズ失敗: {e}"))?;
        let file_path = dir.join(format!("{}.yml", template.name));
        let tmp_path = file_path.with_extension("yml.tmp");
        fs::write(&tmp_path, &content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
        fs::rename(&tmp_path, &file_path).map_err(|e| format!("ファイルのリネーム失敗: {e}"))?;
        Ok(())
    }

    #[test]
    fn save_and_load_prompt() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let tpl = sample_prompt("my-prompt", false);
        save_prompt(dir, &tpl).unwrap();

        let file_path = dir.join("my-prompt.yml");
        assert!(file_path.exists());

        let loaded = load_prompt(&file_path).unwrap();
        assert_eq!(loaded.name, "my-prompt");
        assert_eq!(loaded.description, "my-prompt prompt");
        assert_eq!(loaded.variables.len(), 1);
    }

    #[test]
    fn list_prompts_returns_sorted() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        save_prompt(dir, &sample_prompt("charlie", false)).unwrap();
        save_prompt(dir, &sample_prompt("alpha", false)).unwrap();
        save_prompt(dir, &sample_prompt("bravo", true)).unwrap();

        let list = list_prompts(dir).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "bravo");
        assert!(list[1].builtin);
        assert_eq!(list[2].name, "charlie");
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
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("見つかりません"));
    }

    #[test]
    fn resolve_workflow_path_invalid_name() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "../evil");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_workflow_path_empty_name() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_workflow_path(tmp.path(), "");
        assert!(result.is_err());
    }
}
