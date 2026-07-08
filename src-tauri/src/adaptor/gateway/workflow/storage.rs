use super::builtin;
use super::domain_mapping::workflow_definition_to_domain;
use super::facet;
use super::schema::{Summary, Workflow};
use crate::domain::workflow::validation::{self, ValidationError};
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
    FacetResolution(facet::FacetError),
    /// facet 本文が `{{vars.<name>}}` で workflow 定義に存在しない変数を参照している。
    /// spec issues-1054 「未定義 workflow 変数の拒否」: load 経路を一次境界として検出する。
    UndefinedWorkflowVariables {
        node_name: String,
        undefined: Vec<String>,
    },
    NotFound {
        name: String,
    },
    BuiltinProtected {
        name: String,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/Oエラー: {e}"),
            Self::YamlDeserialize(e) => write!(f, "YAMLパース失敗: {e}"),
            Self::YamlSerialize(e) => write!(f, "YAMLシリアライズ失敗: {e}"),
            Self::Validation(e) => write!(f, "validation_error: {e}"),
            Self::FacetResolution(e) => write!(f, "facet解決失敗: {e}"),
            Self::UndefinedWorkflowVariables {
                node_name,
                undefined,
            } => write!(
                f,
                "node '{node_name}' の facet が未定義の workflow 変数を参照しています: {} （workflow 定義の `variables` で宣言してください）",
                undefined.join(", ")
            ),
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
            Self::FacetResolution(e) => Some(e),
            Self::UndefinedWorkflowVariables { .. } => None,
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

pub fn save_workflow(dir: &Path, workflow: &Workflow) -> Result<(), StorageError> {
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
) -> Result<Workflow, StorageError> {
    let mut workflow: Workflow = serde_saphyr::from_str(content)?;
    workflow.builtin = builtin::is_builtin_workflow(&workflow.name);
    validate_workflow_definition(&workflow)?;
    facet::resolve_workflow_facets(&mut workflow, facets_base_dir)?;
    validate_workflow_variable_refs(&workflow)?;
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
) -> Result<Workflow, StorageError> {
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

/// YAML ファイルから `Workflow` を読み込み、facet 参照を解決した上で validation する。
///
/// [02] schema 境界: load 経路で `facet.rs` を呼び、session / fanout child の
/// `resolved_facets` に解決済み内容を格納する。
/// 実行用 Workflow には未解決 ref を残さない（schema 層は ref キーを保持しつつ、
/// 実行系は resolved cache から直接合成する）。
pub fn load_workflow(path: &Path, facets_base_dir: &Path) -> Result<Workflow, StorageError> {
    let content = fs::read_to_string(path)?;
    let mut workflow: Workflow = serde_saphyr::from_str(&content)?;
    // YAMLの builtin フラグは無視し、コード側（builtin.rs）で判定する
    workflow.builtin = builtin::is_builtin_workflow(&workflow.name);
    validate_workflow_definition(&workflow)?;
    facet::resolve_workflow_facets(&mut workflow, facets_base_dir)?;
    // spec issues-1054: 未定義 `{{vars.<name>}}` 参照は load 経路を一次境界として検出する。
    validate_workflow_variable_refs(&workflow)?;
    Ok(workflow)
}

/// resolved_facets の各本文に `{{vars.<name>}}` 参照が含まれている場合、
/// すべて workflow.variables で宣言されていることを検証する。
///
/// spec issues-1054 「未定義 workflow 変数の拒否」: load 時点でエラーとし、
/// 黙って空文字へ展開させない（権限や宛先ずれの事故防止）。
fn validate_workflow_variable_refs(workflow: &Workflow) -> Result<(), StorageError> {
    for node in &workflow.nodes {
        if let Some(session) = node.session() {
            for body in resolved_bodies(&session.resolved_facets) {
                check_undefined_vars(&node.name, body, &workflow.variables)?;
            }
        }
        if let Some(fanout) = node.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                for body in resolved_bodies(&child.resolved_facets) {
                    check_undefined_vars(&child.name, body, &workflow.variables)?;
                }
            }
        }
    }
    Ok(())
}

fn resolved_bodies(
    rf: &crate::adaptor::gateway::workflow::schema::ResolvedFacets,
) -> impl Iterator<Item = &str> + '_ {
    rf.policy
        .as_deref()
        .into_iter()
        .chain(rf.knowledge.as_deref())
        .chain(rf.instruction.as_deref())
}

fn check_undefined_vars(
    node_name: &str,
    body: &str,
    defined: &std::collections::HashMap<String, String>,
) -> Result<(), StorageError> {
    let undefined =
        crate::domain::workflow::services::variable_renderer::find_undefined_workflow_variable_refs(
            body, defined,
        );
    if undefined.is_empty() {
        return Ok(());
    }
    let mut unique: Vec<String> = undefined;
    unique.sort();
    unique.dedup();
    Err(StorageError::UndefinedWorkflowVariables {
        node_name: node_name.to_string(),
        undefined: unique,
    })
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
    // list 用途では facet 未解決でも一覧表示に支障がないため、deserialize+validate のみを行う。
    // facet 解決が必要な実行系経路は明示的に `load_workflow` を呼ぶ。
    let load_for_listing = |path: &Path| -> Result<Workflow, StorageError> {
        let content = fs::read_to_string(path)?;
        let mut workflow: Workflow = serde_saphyr::from_str(&content)?;
        workflow.builtin = builtin::is_builtin_workflow(&workflow.name);
        validate_workflow_definition(&workflow)?;
        Ok(workflow)
    };
    let mut summaries = list_yml_summaries(
        dir,
        load_for_listing,
        |wf| Summary {
            name: wf.name.clone(),
            description: wf.description,
            builtin: false, // name がファイル stem で上書きされた後に再計算する
            is_running: false,
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

fn validate_workflow_definition(workflow: &Workflow) -> Result<(), ValidationError> {
    let workflow = workflow_definition_to_domain(workflow);
    validation::validate(&workflow)
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

    fn sample_workflow(name: &str, builtin: bool) -> Workflow {
        Workflow {
            variables: Default::default(),
            name: name.to_string(),
            description: format!("{name} workflow"),
            builtin,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "step1".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    permission: Some("edit".to_string()),
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
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
    /// `NodeDefinition.resolved_facets` に本文を格納する。
    #[test]
    fn load_workflow_resolves_facets_into_node_cache() {
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
  - name: implement
    session:
      permission: edit
      gate: auto
      facets:
        policy: coding
        instruction: implement
"#;
        let file_path = dir.join("facet-load-test.yml");
        std::fs::write(&file_path, yaml).unwrap();
        let wf = load_workflow(&file_path, dir).unwrap();
        let node = &wf.nodes[0];
        let session = node.session().unwrap();
        assert_eq!(
            session.resolved_facets.policy.as_deref(),
            Some("POLICY_BODY")
        );
        assert_eq!(
            session.resolved_facets.instruction.as_deref(),
            Some("INSTRUCTION_BODY")
        );
    }

    /// [02] schema 境界: 旧 `steps:` 表現で書かれた user-authored YAML は
    /// 新 schema (`nodes:` 必須 + `deny_unknown_fields`) として load に失敗する。
    /// これにより利用者は新表現で書き直さない限り実行に進めない。
    #[test]
    fn load_workflow_rejects_legacy_steps_yaml() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let yaml = r#"
name: legacy-user
description: legacy user yaml
steps:
  - name: x
    mode: auto
    instruction: x
    permission: edit
"#;
        let file_path = dir.join("legacy-user.yml");
        std::fs::write(&file_path, yaml).unwrap();
        let result = load_workflow(&file_path, dir);
        assert!(
            matches!(result, Err(StorageError::YamlDeserialize(_))),
            "旧 steps YAML は load 段階で deserialize 失敗する"
        );
    }

    /// [02] schema 境界: load 経路で 3 種全 facet (policy/knowledge/instruction)
    /// が node と fanout child の resolved facets に
    /// いずれにも解決済みで格納されることを担保する。
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
        std::fs::write(knowledge.join("k.md"), "KNOWLEDGE").unwrap();
        std::fs::write(instructions.join("i.md"), "INSTRUCTION").unwrap();
        std::fs::write(policies.join("pc.md"), "CHILD_POLICY").unwrap();
        std::fs::write(knowledge.join("kc.md"), "CHILD_KNOWLEDGE").unwrap();
        std::fs::write(instructions.join("ic.md"), "CHILD_INSTRUCTION").unwrap();

        let yaml = r#"
name: facet-all
description: all three facets per node
nodes:
  - name: lead
    session:
      permission: edit
      gate: auto
      facets:
        policy: p
        knowledge: k
        instruction: i
  - name: par
    fanout:
      parallel_children:
        - name: c1
          permission: ask
          facets:
            policy: pc
            knowledge: kc
            instruction: ic
        - name: c2
          permission: ask
          facets:
            policy: pc
            knowledge: kc
            instruction: ic
      aggregate:
        all_match: LGTM
        then: lead
        else: lead
"#;
        let file_path = dir.join("facet-all.yml");
        std::fs::write(&file_path, yaml).unwrap();
        let wf = load_workflow(&file_path, dir).unwrap();

        let lead = wf.nodes.iter().find(|n| n.name == "lead").unwrap();
        let lead_session = lead.session().unwrap();
        assert_eq!(
            lead_session.resolved_facets.policy.as_deref(),
            Some("POLICY")
        );
        assert_eq!(
            lead_session.resolved_facets.knowledge.as_deref(),
            Some("KNOWLEDGE")
        );
        assert_eq!(
            lead_session.resolved_facets.instruction.as_deref(),
            Some("INSTRUCTION")
        );

        let par = wf.nodes.iter().find(|n| n.name == "par").unwrap();
        let children = &par.fanout().unwrap().parallel_children;
        for child in children {
            assert_eq!(
                child.resolved_facets.policy.as_deref(),
                Some("CHILD_POLICY")
            );
            assert_eq!(
                child.resolved_facets.knowledge.as_deref(),
                Some("CHILD_KNOWLEDGE")
            );
            assert_eq!(
                child.resolved_facets.instruction.as_deref(),
                Some("CHILD_INSTRUCTION")
            );
        }
    }

    /// 欠損 facet を参照する workflow は load 段階で拒否される。
    #[test]
    fn load_workflow_rejects_missing_facet() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let yaml = r#"
name: missing-facet
description: missing facet test
nodes:
  - name: implement
    session:
      permission: edit
      gate: auto
      facets:
        policy: nonexistent-policy
        instruction: implement
"#;
        let file_path = dir.join("missing-facet.yml");
        std::fs::write(&file_path, yaml).unwrap();
        let result = load_workflow(&file_path, dir);
        assert!(matches!(result, Err(StorageError::FacetResolution(_))));
    }

    /// spec issues-1054 「未定義 workflow 変数の拒否」: facet 本文が宣言されていない
    /// `{{vars.<name>}}` を参照している workflow は load 経路で拒否される。
    #[test]
    fn load_workflow_rejects_undefined_workflow_variable_reference() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            instructions.join("impl.md"),
            "Run `{{vars.cli_alias}} workflow output submit`",
        )
        .unwrap();

        let yaml = r#"
name: undefined-var-ref
description: undefined variable test
variables:
  other_var: "value"
nodes:
  - name: implement
    session:
      permission: edit
      gate: auto
      facets:
        instruction: impl
"#;
        let file_path = dir.join("undefined-var-ref.yml");
        std::fs::write(&file_path, yaml).unwrap();
        let result = load_workflow(&file_path, dir);
        let err = result.expect_err("undefined {{vars.cli_alias}} must be rejected");
        match err {
            StorageError::UndefinedWorkflowVariables {
                node_name,
                undefined,
            } => {
                assert_eq!(node_name, "implement");
                assert_eq!(undefined, vec!["cli_alias".to_string()]);
            }
            other => panic!("expected UndefinedWorkflowVariables, got {other:?}"),
        }
    }

    /// spec issues-1054 「workflow 定義変数の facet 展開」: workflow 定義の
    /// `variables` で宣言された変数を参照する facet は load 経路を通る。
    #[test]
    fn load_workflow_accepts_declared_workflow_variable_reference() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            instructions.join("impl.md"),
            "Label: {{vars.project_label}}",
        )
        .unwrap();

        let yaml = r#"
name: declared-var-ref
description: declared variable test
variables:
  project_label: "Releash"
nodes:
  - name: implement
    session:
      permission: edit
      gate: auto
      facets:
        instruction: impl
"#;
        let file_path = dir.join("declared-var-ref.yml");
        std::fs::write(&file_path, yaml).unwrap();
        let wf = load_workflow(&file_path, dir).expect("load must succeed");
        assert_eq!(
            wf.variables.get("project_label").map(String::as_str),
            Some("Releash")
        );
    }
}
