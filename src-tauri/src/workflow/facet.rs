use super::builtin;
use super::schema::{ChildNodeDefinition, NodeDefinition, ResolvedFacets, Workflow};
use super::storage;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// ファセットの読み込みベースディレクトリ。
///
/// [02] 境界: storage / builtin / engine の caller 全てがここを参照することで、
/// builtin → storage の循環依存を生まず、facet 側を単一の owner にする。
pub fn facets_base_dir() -> PathBuf {
    storage::workflows_dir()
}

/// システム定義テンプレート変数（commands.rs / diagnostics.rs 両方から参照）
pub const SYSTEM_TEMPLATE_VARIABLES: &[&str] = &["project_name", "task"];

#[derive(Debug)]
pub enum FacetError {
    InvalidKey { key: String },
    NotFound { kind: FacetKind, key: String },
    BuiltinProtected { kind: FacetKind, key: String },
    Io(std::io::Error),
}

impl fmt::Display for FacetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey { key } => write!(
                f,
                "ファセットキー '{key}' が無効です（先頭は英数字、以降は英数字・ハイフン・アンダースコアのみ許可）"
            ),
            Self::NotFound { kind, key } => write!(
                f,
                "ファセット '{key}' ({}) が見つかりません",
                kind.dir_name()
            ),
            Self::BuiltinProtected { kind, key } => write!(
                f,
                "ビルトインファセット '{key}' ({}) は削除できません",
                kind.dir_name()
            ),
            Self::Io(e) => write!(f, "I/Oエラー: {e}"),
        }
    }
}

impl std::error::Error for FacetError {}

impl From<std::io::Error> for FacetError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Serialize for FacetError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacetKind {
    Policy,
    Knowledge,
    Instruction,
    OutputContract,
}

impl FacetKind {
    pub fn dir_name(&self) -> &str {
        match self {
            Self::Policy => "policies",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instructions",
            Self::OutputContract => "output_contracts",
        }
    }
}

#[derive(Debug)]
pub struct ComposedPrompt {
    pub system_prompt: Option<String>,
    pub user_message: String,
}

/// ファセット本文からテンプレート変数名を抽出する（`{{var}}` パターン）
pub fn extract_template_variables(content: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut start = 0;
    while let Some(open) = content[start..].find("{{") {
        let abs_open = start + open + 2;
        if let Some(close) = content[abs_open..].find("}}") {
            let var_name = content[abs_open..abs_open + close].trim();
            if !var_name.is_empty() {
                vars.push(var_name.to_string());
            }
            start = abs_open + close + 2;
        } else {
            break;
        }
    }
    vars
}

/// テンプレート変数がすべてシステム定義変数であることを検証する。
/// 未定義変数があればそのリストを返す。
pub fn find_undefined_template_variables(content: &str) -> Vec<String> {
    extract_template_variables(content)
        .into_iter()
        .filter(|v| !SYSTEM_TEMPLATE_VARIABLES.contains(&v.as_str()))
        .collect()
}

/// テンプレート変数を指定した値で置換する。
/// `WorkflowEngine::render_facet_variables` と同一のパターンで展開。
pub fn render_template_variables(
    content: &str,
    values: &std::collections::HashMap<String, String>,
) -> String {
    let mut result = content.to_string();
    for (key, value) in values {
        let pattern = format!("{{{{{key}}}}}");
        result = result.replace(&pattern, value);
    }
    result
}

pub fn validate_facet_key(key: &str) -> Result<(), FacetError> {
    if key.is_empty() {
        return Err(FacetError::InvalidKey {
            key: key.to_string(),
        });
    }
    let mut chars = key.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(FacetError::InvalidKey {
            key: key.to_string(),
        });
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(FacetError::InvalidKey {
                key: key.to_string(),
            });
        }
    }
    Ok(())
}

pub fn load_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<String, FacetError> {
    validate_facet_key(key)?;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    if path.exists() {
        return Ok(fs::read_to_string(&path)?);
    }
    if let Some(content) = builtin::get_builtin_facet(kind, key) {
        return Ok(content.to_string());
    }
    Err(FacetError::NotFound {
        kind,
        key: key.to_string(),
    })
}

pub fn save_facet(
    kind: FacetKind,
    key: &str,
    content: &str,
    base_dir: &Path,
) -> Result<(), FacetError> {
    validate_facet_key(key)?;
    let dir = base_dir.join(kind.dir_name());
    storage::ensure_dir(&dir).map_err(|e| FacetError::Io(std::io::Error::other(e.to_string())))?;
    let path = dir.join(format!("{key}.md"));
    fs::write(&path, content)?;
    Ok(())
}

pub fn delete_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<(), FacetError> {
    validate_facet_key(key)?;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    if path.exists() {
        fs::remove_file(&path)?;
        return Ok(());
    }
    if builtin::is_builtin_facet(kind, key) {
        return Err(FacetError::BuiltinProtected {
            kind,
            key: key.to_string(),
        });
    }
    Err(FacetError::NotFound {
        kind,
        key: key.to_string(),
    })
}

pub fn list_facets(kind: FacetKind, base_dir: &Path) -> Result<Vec<String>, FacetError> {
    let mut keys = BTreeSet::new();
    let dir = base_dir.join(kind.dir_name());
    if dir.exists() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    keys.insert(stem.to_string());
                }
            }
        }
    }
    for k in builtin::list_builtin_facet_keys(kind) {
        keys.insert(k.to_string());
    }
    Ok(keys.into_iter().collect())
}

/// Markdownファイルの先頭行から説明を取得する。
/// 先頭行が `# ` で始まる場合はその見出しテキストを、そうでなければ先頭の非空行をそのまま使用する。
pub fn extract_description(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().to_string();
        }
        return trimmed.to_string();
    }
    String::new()
}

pub fn list_facet_summaries(
    kind: FacetKind,
    base_dir: &Path,
) -> Result<Vec<super::schema::FacetSummary>, FacetError> {
    let kind_name = kind.dir_name().to_string();
    let mut summaries = Vec::new();
    let dir = base_dir.join(kind.dir_name());

    let mut seen_keys = BTreeSet::new();
    if dir.exists() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let content = match fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            log::warn!("ファセットファイル読み込み失敗: {}: {e}", path.display());
                            String::new()
                        }
                    };
                    summaries.push(super::schema::FacetSummary {
                        key: stem.to_string(),
                        kind: kind_name.clone(),
                        description: extract_description(&content),
                        builtin: builtin::is_builtin_facet(kind, stem),
                    });
                    seen_keys.insert(stem.to_string());
                }
            }
        }
    }

    for key in builtin::list_builtin_facet_keys(kind) {
        if !seen_keys.contains(key) {
            let content = builtin::get_builtin_facet(kind, key).unwrap_or("");
            summaries.push(super::schema::FacetSummary {
                key: key.to_string(),
                kind: kind_name.clone(),
                description: extract_description(content),
                builtin: true,
            });
        }
    }

    summaries.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(summaries)
}

pub fn resolve_facet_path(
    kind: FacetKind,
    key: &str,
    base_dir: &Path,
) -> Result<std::path::PathBuf, FacetError> {
    validate_facet_key(key)?;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    if !path.exists() {
        return Err(FacetError::NotFound {
            kind,
            key: key.to_string(),
        });
    }
    Ok(path)
}

/// node の prompt 関連 facet 参照から組み立てた `ComposedPrompt` を返す。
///
/// [02] schema 境界: 実行時に未解決 ref を残さないため、`NodeDefinition.resolved_facets`
/// を唯一の参照源とする。ファイル I/O fallback は持たない（呼び出し前に
/// `storage::load_workflow` / `builtin::load_builtin_workflow_resolved` 経由で
/// `resolved_facets` を populate しておくこと）。
///
/// agent / approval 種別の node が対象。bash / parallel node には facet 参照は存在しない。
pub fn compose_facets(node: &NodeDefinition) -> ComposedPrompt {
    compose_from_resolved(&node.resolved_facets)
}

/// 並列子 node の prompt 関連 facet 参照から組み立てた `ComposedPrompt` を返す。
/// `compose_facets` と同じく `ChildNodeDefinition.resolved_facets` のみを参照する。
pub fn compose_child_facets(child: &ChildNodeDefinition) -> ComposedPrompt {
    compose_from_resolved(&child.resolved_facets)
}

fn compose_from_resolved(resolved: &ResolvedFacets) -> ComposedPrompt {
    let mut system_parts: Vec<String> = Vec::new();
    if let Some(ref content) = resolved.policy {
        system_parts.push(content.clone());
    }
    if let Some(ref content) = resolved.output_contract {
        system_parts.push(content.clone());
    }
    let system_prompt = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    let mut user_parts: Vec<String> = Vec::new();
    if let Some(ref content) = resolved.knowledge {
        user_parts.push(content.clone());
    }
    if let Some(ref content) = resolved.instruction {
        user_parts.push(content.clone());
    }
    ComposedPrompt {
        system_prompt,
        user_message: user_parts.join("\n\n"),
    }
}

/// `Workflow` に含まれる全 node / 子 node の facet 参照を解決し、
/// それぞれの `resolved_facets` フィールドに本文を格納する。
///
/// `storage::load_workflow` から呼ばれ、未解決 ref を schema 層に残さないようにする
/// （[02] schema 境界）。欠損 facet があれば `FacetError::NotFound` を伝搬し、
/// load 経路で実行可能とは判定しない。
pub fn resolve_workflow_facets(workflow: &mut Workflow, base_dir: &Path) -> Result<(), FacetError> {
    for node in &mut workflow.nodes {
        node.resolved_facets = resolve_refs(
            node.policy.as_deref(),
            node.knowledge.as_deref(),
            node.instruction.as_deref(),
            node.output_contract.as_deref(),
            base_dir,
        )?;
        if let Some(ref mut children) = node.parallel_children {
            for child in children {
                child.resolved_facets = resolve_refs(
                    child.policy.as_deref(),
                    child.knowledge.as_deref(),
                    child.instruction.as_deref(),
                    child.output_contract.as_deref(),
                    base_dir,
                )?;
            }
        }
    }
    Ok(())
}

/// テスト用ヘルパー: 単一 node の facet 参照を `base_dir` から解決し
/// `resolved_facets` に格納する。production の `resolve_workflow_facets` が
/// workflow 全体に対して行う処理を、engine / facet 各モジュールの単体テストで
/// 個別 node 単位に分解して使うためのもの。
///
/// `unwrap()` 等で潰さず `FacetError` をそのまま返すため、欠損 facet のテスト
/// シナリオもこのヘルパーを経由して書ける。
#[cfg(test)]
pub(crate) fn resolve_node_facets(
    node: &mut crate::workflow::schema::NodeDefinition,
    base_dir: &Path,
) -> Result<(), FacetError> {
    node.resolved_facets = resolve_refs(
        node.policy.as_deref(),
        node.knowledge.as_deref(),
        node.instruction.as_deref(),
        node.output_contract.as_deref(),
        base_dir,
    )?;
    Ok(())
}

/// テスト用ヘルパー: 並列子 node の facet 参照を解決する。
/// `resolve_node_facets` の `ChildNodeDefinition` 版。
#[cfg(test)]
pub(crate) fn resolve_child_facets(
    child: &mut crate::workflow::schema::ChildNodeDefinition,
    base_dir: &Path,
) -> Result<(), FacetError> {
    child.resolved_facets = resolve_refs(
        child.policy.as_deref(),
        child.knowledge.as_deref(),
        child.instruction.as_deref(),
        child.output_contract.as_deref(),
        base_dir,
    )?;
    Ok(())
}

fn resolve_refs(
    policy: Option<&str>,
    knowledge: Option<&str>,
    instruction: Option<&str>,
    output_contract: Option<&str>,
    base_dir: &Path,
) -> Result<ResolvedFacets, FacetError> {
    let resolved_policy = match policy {
        Some(k) => Some(load_facet(FacetKind::Policy, k, base_dir)?),
        None => None,
    };
    let resolved_knowledge = match knowledge {
        Some(k) => Some(load_facet(FacetKind::Knowledge, k, base_dir)?),
        None => None,
    };
    let resolved_instruction = match instruction {
        Some(k) => Some(load_facet(FacetKind::Instruction, k, base_dir)?),
        None => None,
    };
    let resolved_oc = match output_contract {
        Some(k) => Some(load_facet(FacetKind::OutputContract, k, base_dir)?),
        None => None,
    };
    Ok(ResolvedFacets {
        policy: resolved_policy,
        knowledge: resolved_knowledge,
        instruction: resolved_instruction,
        output_contract: resolved_oc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::NodeType;
    use tempfile::TempDir;

    fn make_facet_node(
        policy: Option<&str>,
        knowledge: Option<&str>,
        instruction: Option<&str>,
        output_contract: Option<&str>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: "test".to_string(),
            node_type: NodeType::Agent,
            policy: policy.map(String::from),
            knowledge: knowledge.map(String::from),
            instruction: instruction.map(String::from),
            output_contract: output_contract.map(String::from),
            ..NodeDefinition::default()
        }
    }

    fn setup_facet_files(dir: &Path) {
        let policies = dir.join("policies");
        let knowledge = dir.join("knowledge");
        let instructions = dir.join("instructions");
        let output_contracts = dir.join("output_contracts");
        for d in [&policies, &knowledge, &instructions, &output_contracts] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(policies.join("coding.md"), "Follow best practices.").unwrap();
        fs::write(policies.join("review.md"), "Review carefully.").unwrap();
        fs::write(knowledge.join("architecture.md"), "The system uses Tauri.").unwrap();
        fs::write(instructions.join("implement.md"), "Implement the feature.").unwrap();
        fs::write(output_contracts.join("plan-doc.md"), "Output as markdown.").unwrap();
    }

    // --- validate_facet_key ---

    #[test]
    fn valid_keys() {
        assert!(validate_facet_key("coder").is_ok());
        assert!(validate_facet_key("my-facet").is_ok());
        assert!(validate_facet_key("test_123").is_ok());
        assert!(validate_facet_key("a").is_ok());
        assert!(validate_facet_key("A1-b_c").is_ok());
    }

    #[test]
    fn invalid_keys() {
        assert!(validate_facet_key("").is_err());
        assert!(validate_facet_key("-start").is_err());
        assert!(validate_facet_key("_start").is_err());
        assert!(validate_facet_key("a/b").is_err());
        assert!(validate_facet_key("a b").is_err());
        assert!(validate_facet_key("../evil").is_err());
    }

    // --- load_facet ---

    #[test]
    fn load_existing_facet() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());
        let content = load_facet(FacetKind::Policy, "coding", tmp.path()).unwrap();
        assert_eq!(content, "Follow best practices.");
    }

    #[test]
    fn load_missing_facet_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = load_facet(FacetKind::Policy, "unknown", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn load_facet_with_invalid_key() {
        let tmp = TempDir::new().unwrap();
        let result = load_facet(FacetKind::Policy, "../evil", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::InvalidKey { .. }));
    }

    // --- save_facet ---

    #[test]
    fn save_new_facet() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Policy, "new-one", "content", tmp.path()).unwrap();
        let path = tmp.path().join("policies/new-one.md");
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "content");
    }

    #[test]
    fn save_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Knowledge, "test", "v1", tmp.path()).unwrap();
        save_facet(FacetKind::Knowledge, "test", "v2", tmp.path()).unwrap();
        let content = load_facet(FacetKind::Knowledge, "test", tmp.path()).unwrap();
        assert_eq!(content, "v2");
    }

    #[test]
    fn save_with_invalid_key() {
        let tmp = TempDir::new().unwrap();
        let result = save_facet(FacetKind::Knowledge, "", "content", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::InvalidKey { .. }));
    }

    // --- delete_facet ---

    #[test]
    fn delete_existing_facet() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Knowledge, "deleteme", "content", tmp.path()).unwrap();
        delete_facet(FacetKind::Knowledge, "deleteme", tmp.path()).unwrap();
        assert!(!tmp.path().join("knowledge/deleteme.md").exists());
    }

    #[test]
    fn delete_missing_facet_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = delete_facet(FacetKind::Knowledge, "nope", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    // --- list_facets ---

    #[test]
    fn list_facets_sorted() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        assert_eq!(keys, vec!["architecture"]);
    }

    #[test]
    fn list_facets_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("knowledge")).unwrap();
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn list_facets_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        assert!(keys.is_empty());
    }

    // `resolve_node_facets` はモジュール直下の `#[cfg(test)] pub(crate)` ヘルパーを
    // `super::resolve_node_facets` 経由で利用する（engine.rs のテストヘルパーと共有）。

    // --- compose_facets ---
    // Gherkin: ワークフローエンジンはステップ宣言から system_prompt と user_message を合成する

    #[test]
    fn compose_system_prompt_from_policy_and_output_contract() {
        // Scenario: policyとoutput_contractの両方を指定したステップから system_prompt が合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(Some("coding"), None, None, Some("plan-doc"));
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        let sys = result.system_prompt.expect("system_prompt should be set");
        assert!(sys.contains("Follow best practices."));
        assert!(sys.contains("Output as markdown."));
        assert_eq!(result.user_message, "");
    }

    #[test]
    fn compose_system_prompt_from_policy_only() {
        // Scenario: policyのみを指定したステップでも system_prompt が合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(Some("coding"), None, None, None);
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        assert_eq!(
            result.system_prompt.as_deref(),
            Some("Follow best practices.")
        );
        assert_eq!(result.user_message, "");
    }

    #[test]
    fn compose_system_prompt_from_output_contract_only() {
        // Scenario: output_contractのみを指定したステップでも system_prompt が合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(None, None, None, Some("plan-doc"));
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        assert_eq!(result.system_prompt.as_deref(), Some("Output as markdown."));
        assert_eq!(result.user_message, "");
    }

    #[test]
    fn compose_no_system_prompt_when_neither_policy_nor_output_contract() {
        // Scenario: policy も output_contract も指定がないと system_prompt は設定されない
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(None, Some("architecture"), Some("implement"), None);
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        assert!(result.system_prompt.is_none());
    }

    #[test]
    fn compose_user_message_from_knowledge_and_instruction() {
        // Scenario: knowledgeとinstructionを指定したステップから user_message が合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(None, Some("architecture"), Some("implement"), None);
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        assert!(result.user_message.contains("The system uses Tauri."));
        assert!(result.user_message.contains("Implement the feature."));
    }

    #[test]
    fn compose_user_message_empty_when_neither_knowledge_nor_instruction() {
        // Scenario: knowledge も instruction も指定がないと user_message は空文字として合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(Some("coding"), None, None, None);
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        assert_eq!(result.user_message, "");
    }

    #[test]
    fn compose_all_four_facets() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let mut node = make_facet_node(
            Some("coding"),
            Some("architecture"),
            Some("implement"),
            Some("plan-doc"),
        );
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let result = compose_facets(&node);

        let sys = result.system_prompt.expect("system_prompt should be set");
        assert!(sys.contains("Follow best practices."));
        assert!(sys.contains("Output as markdown."));
        assert!(result.user_message.contains("The system uses Tauri."));
        assert!(result.user_message.contains("Implement the feature."));
    }

    /// 解決経路における欠損 facet は load 時 (`resolve_refs`) で NotFound として
    /// 弾かれる。`compose_facets` 自体は I/O fallback を持たず、unresolved な node を
    /// 受け取った場合は空合成結果になる（実 production では load 経路で先に弾かれる）。
    #[test]
    fn resolve_with_missing_facet_returns_error() {
        let tmp = TempDir::new().unwrap();
        let mut node = make_facet_node(Some("nonexistent"), None, None, None);
        let result = resolve_node_facets(&mut node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn resolve_with_missing_knowledge_returns_error() {
        let tmp = TempDir::new().unwrap();
        let mut node = make_facet_node(None, Some("nonexistent"), None, None);
        let result = resolve_node_facets(&mut node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn resolve_with_missing_instruction_returns_error() {
        let tmp = TempDir::new().unwrap();
        let mut node = make_facet_node(None, None, Some("nonexistent"), None);
        let result = resolve_node_facets(&mut node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn resolve_with_missing_output_contract_returns_error() {
        let tmp = TempDir::new().unwrap();
        let mut node = make_facet_node(None, None, None, Some("nonexistent"));
        let result = resolve_node_facets(&mut node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    // --- compose → render_facet_variables パイプライン結合テスト ---

    #[test]
    fn compose_and_render_pipeline() {
        use crate::workflow::engine::WorkflowEngine;

        let tmp = TempDir::new().unwrap();
        let policies = tmp.path().join("policies");
        let instructions = tmp.path().join("instructions");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            policies.join("coding.md"),
            "Coding rules for {{project_name}}.",
        )
        .unwrap();
        std::fs::write(
            instructions.join("impl.md"),
            "Task: {{task}}\nProject: {{project_name}}",
        )
        .unwrap();

        let mut node = make_facet_node(Some("coding"), None, Some("impl"), None);
        resolve_node_facets(&mut node, tmp.path()).unwrap();
        let composed = compose_facets(&node);

        let worktree_path = "/home/user/my-project";
        let task = Some("Fix the bug");

        let rendered_system = composed
            .system_prompt
            .map(|s| WorkflowEngine::render_facet_variables(&s, worktree_path, task));
        let rendered_user =
            WorkflowEngine::render_facet_variables(&composed.user_message, worktree_path, task);

        assert_eq!(rendered_system.unwrap(), "Coding rules for my-project.");
        assert_eq!(rendered_user, "Task: Fix the bug\nProject: my-project");
    }

    // --- extract_description ---

    #[test]
    fn extract_description_heading() {
        assert_eq!(extract_description("# My Facet\nContent here"), "My Facet");
    }

    #[test]
    fn extract_description_no_heading() {
        assert_eq!(
            extract_description("Some content\nMore content"),
            "Some content"
        );
    }

    #[test]
    fn extract_description_empty() {
        assert_eq!(extract_description(""), "");
    }

    #[test]
    fn extract_description_leading_blank_lines() {
        assert_eq!(extract_description("\n\n# Title\nBody"), "Title");
    }

    // --- list_facet_summaries ---

    #[test]
    fn list_facet_summaries_merges_builtin_and_custom() {
        let tmp = TempDir::new().unwrap();
        let policies = tmp.path().join("policies");
        fs::create_dir_all(&policies).unwrap();
        fs::write(
            policies.join("custom-policy.md"),
            "# Custom Policy\nContent",
        )
        .unwrap();

        let summaries = list_facet_summaries(FacetKind::Policy, tmp.path()).unwrap();
        // 4 builtin policies + 1 custom
        assert_eq!(summaries.len(), 5);

        let custom = summaries.iter().find(|s| s.key == "custom-policy").unwrap();
        assert!(!custom.builtin);
        assert_eq!(custom.description, "Custom Policy");

        let coding = summaries.iter().find(|s| s.key == "coding").unwrap();
        assert!(coding.builtin);
    }

    // --- resolve_facet_path ---

    #[test]
    fn resolve_facet_path_existing() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Policy, "test", "content", tmp.path()).unwrap();
        let path = resolve_facet_path(FacetKind::Policy, "test", tmp.path()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn resolve_facet_path_missing() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_facet_path(FacetKind::Policy, "nope", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    // --- 重複チェック用ヘルパーテスト ---

    #[test]
    fn list_facets_detects_existing_custom_key() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Policy, "my-policy", "content", tmp.path()).unwrap();
        let existing = list_facets(FacetKind::Policy, tmp.path()).unwrap();
        assert!(existing.contains(&"my-policy".to_string()));
    }

    #[test]
    fn list_facets_includes_builtin_keys() {
        let tmp = TempDir::new().unwrap();
        let existing = list_facets(FacetKind::Policy, tmp.path()).unwrap();
        // ビルトインのポリシーキーが含まれる
        assert!(!existing.is_empty());
    }

    #[test]
    fn delete_builtin_facet_is_protected() {
        let tmp = TempDir::new().unwrap();
        // ビルトインキーの削除はBuiltinProtectedエラー
        let builtin_keys = builtin::list_builtin_facet_keys(FacetKind::Policy);
        if let Some(key) = builtin_keys.first() {
            let result = delete_facet(FacetKind::Policy, key, tmp.path());
            assert!(matches!(
                result.unwrap_err(),
                FacetError::BuiltinProtected { .. }
            ));
        }
    }
}
