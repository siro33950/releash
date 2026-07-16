use super::builtin;
use super::schema::{FacetRefs, WorkflowDefinitionYaml};
use super::storage;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
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
}

impl FacetKind {
    /// ストレージ上のディレクトリ名（複数形）。ファイルシステム経路にのみ使う。
    pub fn dir_name(&self) -> &str {
        match self {
            Self::Policy => "policies",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instructions",
        }
    }

    /// UI / CLI / DiagnosticReport が共有する正規識別子（単数形）。
    /// backend command の `parse_domain_facet_kind` が受理する語彙と一致する。
    pub fn canonical_name(&self) -> &str {
        match self {
            Self::Policy => "policy",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instruction",
        }
    }
}

/// `artifact:` がある node の user message 末尾に置く完了時アクション。
pub fn artifact_completion_action(
    key: &str,
    execution_id: &str,
    node_name: &str,
    node_execution_id: Option<&str>,
) -> String {
    let quoted_key = crate::domain::shell::quote_path_for_shell(key);
    let quoted_execution_id = crate::domain::shell::quote_path_for_shell(execution_id);
    let quoted_node_name = crate::domain::shell::quote_path_for_shell(node_name);
    let node_execution_arg = node_execution_id
        .map(crate::domain::shell::quote_path_for_shell)
        .map(|id| format!("  --node-execution {id} \\\n"))
        .unwrap_or_default();
    format!(
        "## 完了時の必須アクション\n\n\
提出値が確定した時点で、次の assistant action は最終応答ではなく CLI 実行でなければならない。\n\
チャット本文に JSON や要約を書いても提出とは扱われない。必ず次のコマンドで Artifact を提出すること。\n\
このコマンドが成功するまで node は完了していない。\n\n\
```sh\n\
releash workflow output submit {execution_id} \\\n  --node {node_name} \\\n{node_execution_arg}  --type {key} \\\n  --json '{{...}}'\n\
```"
,
        execution_id = quoted_execution_id,
        node_name = quoted_node_name,
        node_execution_arg = node_execution_arg,
        key = quoted_key
    )
}

#[derive(Debug)]
pub struct ComposedPrompt {
    pub system_prompt: Option<String>,
    pub user_message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacetContents {
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
}

impl FacetContents {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none() && self.knowledge.is_none() && self.instruction.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowFacetContents {
    nodes: BTreeMap<String, FacetContents>,
}

impl WorkflowFacetContents {
    pub fn for_node(&self, node_name: &str) -> Option<&FacetContents> {
        self.nodes.get(node_name)
    }

    fn insert_node(&mut self, node_name: String, contents: FacetContents) {
        self.nodes.insert(node_name, contents);
    }

    #[cfg(test)]
    pub(crate) fn from_node_for_test(
        node_name: impl Into<String>,
        contents: FacetContents,
    ) -> Self {
        let mut resolved = Self::default();
        resolved.insert_node(node_name.into(), contents);
        resolved
    }

    pub fn iter_node_contents(&self) -> impl Iterator<Item = (&str, &FacetContents)> {
        self.nodes
            .iter()
            .map(|(node_name, contents)| (node_name.as_str(), contents))
    }
}

pub fn empty_facet_contents() -> &'static FacetContents {
    static EMPTY: std::sync::OnceLock<FacetContents> = std::sync::OnceLock::new();
    EMPTY.get_or_init(FacetContents::default)
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
    let kind_name = kind.canonical_name().to_string();
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
/// 実行時に未解決 ref を残さないため、gateway 側 read model の解決済み本文を
/// 唯一の参照源とする。ファイル I/O fallback は持たない。
///
/// session node が対象。command / fanout node には facet 参照は存在しない。
pub fn compose_facets(resolved: Option<&FacetContents>) -> ComposedPrompt {
    match resolved {
        Some(resolved) => compose_from_parts(resolved),
        None => compose_from_parts(empty_facet_contents()),
    }
}

fn compose_from_parts(resolved: &FacetContents) -> ComposedPrompt {
    let mut system_parts: Vec<String> = Vec::new();
    if let Some(ref content) = resolved.policy {
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

/// `WorkflowDefinitionYaml` に含まれる全 session node の facet 参照を解決し、gateway 側 read model として返す。
///
/// 欠損 facet があれば `FacetError::NotFound` を伝搬し、load 経路で実行可能とは判定しない。
pub fn resolve_workflow_facets(
    workflow: &WorkflowDefinitionYaml,
    base_dir: &Path,
) -> Result<WorkflowFacetContents, FacetError> {
    let mut resolved = WorkflowFacetContents::default();
    for node in &workflow.nodes {
        if let Some(session) = node.session() {
            resolved.insert_node(node.name.clone(), resolve_refs(&session.facets, base_dir)?);
        }
    }
    Ok(resolved)
}

/// テスト用ヘルパー: 単一 node の facet 参照を `base_dir` から解決する。
///
/// `unwrap()` 等で潰さず `FacetError` をそのまま返すため、欠損 facet のテスト
/// シナリオもこのヘルパーを経由して書ける。
#[cfg(test)]
pub(crate) fn resolve_node_facets(
    node: &crate::adaptor::gateway::workflow::schema::NodeDefinition,
    base_dir: &Path,
) -> Result<FacetContents, FacetError> {
    node.session()
        .map(|session| resolve_refs(&session.facets, base_dir))
        .unwrap_or_else(|| Ok(FacetContents::default()))
}

fn resolve_refs(facets: &FacetRefs, base_dir: &Path) -> Result<FacetContents, FacetError> {
    let resolved_policy = match facets.policy.as_deref() {
        Some(k) => Some(load_facet(FacetKind::Policy, k, base_dir)?),
        None => None,
    };
    let resolved_knowledge = match facets.knowledge.as_deref() {
        Some(k) => Some(load_facet(FacetKind::Knowledge, k, base_dir)?),
        None => None,
    };
    let resolved_instruction = match facets.instruction.as_deref() {
        Some(k) => Some(load_facet(FacetKind::Instruction, k, base_dir)?),
        None => None,
    };
    Ok(FacetContents {
        policy: resolved_policy,
        knowledge: resolved_knowledge,
        instruction: resolved_instruction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::prompt_rendering;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, SessionSpec,
    };
    use tempfile::TempDir;

    fn assert_no_deprecated_workflow_vocabulary(label: &str, content: &str) {
        let content = content.to_ascii_lowercase();
        let deprecated_terms = [
            ["workflow ", "runs"].concat(),
            ["workflow ", "reject"].concat(),
            ["--", "step"].concat(),
            ["run", "_id"].concat(),
            ["run", "-id"].concat(),
            ["workflow_", "pending"].concat(),
            ["pending_", "command"].concat(),
            ["cli_", "mutation_", "requested"].concat(),
            ["cli_", "mutation_", "rejected"].concat(),
        ];
        for term in deprecated_terms {
            assert!(
                !content.contains(&term),
                "{label} contains deprecated workflow vocabulary '{term}'"
            );
        }
    }

    fn make_facet_node(
        policy: Option<&str>,
        knowledge: Option<&str>,
        instruction: Option<&str>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: "test".to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    policy: policy.map(String::from),
                    knowledge: knowledge.map(String::from),
                    instruction: instruction.map(String::from),
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn setup_facet_files(dir: &Path) {
        let policies = dir.join("policies");
        let knowledge = dir.join("knowledge");
        let instructions = dir.join("instructions");
        for d in [&policies, &knowledge, &instructions] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(policies.join("coding.md"), "Follow best practices.").unwrap();
        fs::write(policies.join("review.md"), "Review carefully.").unwrap();
        fs::write(knowledge.join("architecture.md"), "The system uses Tauri.").unwrap();
        fs::write(instructions.join("implement.md"), "Implement the feature.").unwrap();
    }

    #[test]
    fn deprecated_vocabulary_guard_rejects_retired_cli_mutation_event_names() {
        for retired_name in [
            ["cli_", "mutation_", "requested"].concat(),
            ["cli_", "mutation_", "rejected"].concat(),
        ] {
            let result = std::panic::catch_unwind(|| {
                assert_no_deprecated_workflow_vocabulary("fixture", &retired_name)
            });
            assert!(
                result.is_err(),
                "guard accepted retired name {retired_name}"
            );
        }
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
        // custom: architecture + builtin: releash-thread-cli (BTreeSet で sort 済み)
        assert_eq!(keys, vec!["architecture", "releash-thread-cli"]);
    }

    #[test]
    fn list_facets_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("knowledge")).unwrap();
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        // custom dir は空でも builtin Knowledge facets は含まれる
        assert_eq!(keys, vec!["releash-thread-cli"]);
    }

    #[test]
    fn list_facets_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        // custom dir が存在しなくても builtin Knowledge facets は含まれる
        assert_eq!(keys, vec!["releash-thread-cli"]);
    }

    // `resolve_node_facets` はモジュール直下の `#[cfg(test)] pub(crate)` ヘルパーを
    // `super::resolve_node_facets` 経由で利用する（engine.rs のテストヘルパーと共有）。

    // --- compose_facets ---
    // Gherkin: ワークフローエンジンは node 宣言から system_prompt と user_message を合成する

    #[test]
    fn compose_prompt_from_policy_knowledge_and_instruction() {
        // Scenario: policy / knowledge / instruction を指定した node から prompt が合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let node = make_facet_node(Some("coding"), Some("architecture"), Some("implement"));
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let result = compose_facets(Some(&resolved));

        let sys = result.system_prompt.expect("system_prompt should be set");
        assert!(sys.contains("Follow best practices."));
        assert!(result.user_message.contains("The system uses Tauri."));
        assert!(result.user_message.contains("Implement the feature."));
    }

    #[test]
    fn compose_system_prompt_from_policy_only() {
        // Scenario: policyのみを指定した node でも system_prompt が合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let node = make_facet_node(Some("coding"), None, None);
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let result = compose_facets(Some(&resolved));

        assert_eq!(
            result.system_prompt.as_deref(),
            Some("Follow best practices.")
        );
        assert_eq!(result.user_message, "");
    }

    #[test]
    fn contract_completion_action_requires_cli_as_next_action() {
        let action = artifact_completion_action("plan-doc", "execution-1", "plan", None);

        assert!(action.contains("完了時の必須アクション"));
        assert!(action.contains("次の assistant action は最終応答ではなく CLI 実行"));
        assert!(action.contains("releash workflow output submit execution-1"));
        assert!(action.contains("--node plan"));
        assert!(action.contains("--type plan-doc"));
        assert!(action.contains("--json"));
        assert!(!action.contains("--file"));
        assert!(!action.contains("--node-execution"));
        assert_no_deprecated_workflow_vocabulary("Artifact completion action", &action);
    }

    #[test]
    fn builtin_agent_facets_use_current_workflow_vocabulary() {
        for kind in [
            FacetKind::Policy,
            FacetKind::Knowledge,
            FacetKind::Instruction,
        ] {
            for key in builtin::list_builtin_facet_keys(kind) {
                let content = builtin::get_builtin_facet(kind, key)
                    .expect("a listed builtin facet must be readable");
                assert_no_deprecated_workflow_vocabulary(key, content);
            }
        }
    }

    #[test]
    fn workflow_agent_sources_use_current_workflow_vocabulary() {
        let sources = [
            (
                "contract repair prompts",
                include_str!("../../../domain/workflow/services/contract.rs"),
            ),
            ("prompt rendering", include_str!("prompt_rendering.rs")),
            ("facet rendering", include_str!("facet.rs")),
        ];
        for (label, content) in sources {
            assert_no_deprecated_workflow_vocabulary(label, content);
        }
    }

    #[test]
    fn contract_completion_action_addresses_node_execution_when_present() {
        let action = artifact_completion_action(
            "plan-doc",
            "execution-1",
            "plan",
            Some("node-execution; bad"),
        );

        assert!(action.contains("--node-execution 'node-execution; bad'"));
        assert!(!action.contains("--node-execution node-execution; bad"));
    }

    #[test]
    fn contract_completion_action_quotes_shell_metacharacters() {
        let action = artifact_completion_action(
            "review; curl https://example.invalid #",
            "execution; bad",
            "node; bad",
            None,
        );

        assert!(action.contains("--type 'review; curl https://example.invalid #'"));
        assert!(!action.contains("--type review; curl"));
        assert!(action.contains("submit 'execution; bad'"));
        assert!(action.contains("--node 'node; bad'"));
    }

    #[test]
    fn compose_no_system_prompt_when_policy_missing() {
        // Scenario: policy を指定しないと system_prompt は設定されない
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let node = make_facet_node(None, Some("architecture"), Some("implement"));
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let result = compose_facets(Some(&resolved));

        assert!(result.system_prompt.is_none());
    }

    #[test]
    fn compose_user_message_from_knowledge_and_keeps_instruction_for_context() {
        // instruction は Agent system context の dedup 経路へ渡すため user_message へ直結しない。
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let node = make_facet_node(None, Some("architecture"), Some("implement"));
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let result = compose_facets(Some(&resolved));

        assert!(result.user_message.contains("The system uses Tauri."));
        assert!(result.user_message.contains("Implement the feature."));
        assert_eq!(
            resolved.instruction.as_deref(),
            Some("Implement the feature.")
        );
    }

    #[test]
    fn compose_user_message_empty_when_neither_knowledge_nor_instruction() {
        // Scenario: knowledge も instruction も指定がないと user_message は空文字として合成される
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let node = make_facet_node(Some("coding"), None, None);
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let result = compose_facets(Some(&resolved));

        assert_eq!(result.user_message, "");
    }

    #[test]
    fn compose_all_three_facets() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let node = make_facet_node(Some("coding"), Some("architecture"), Some("implement"));
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let result = compose_facets(Some(&resolved));

        let sys = result.system_prompt.expect("system_prompt should be set");
        assert!(sys.contains("Follow best practices."));
        assert!(result.user_message.contains("The system uses Tauri."));
        assert!(result.user_message.contains("Implement the feature."));
    }

    /// 解決経路における欠損 facet は load 時 (`resolve_refs`) で NotFound として
    /// 弾かれる。`compose_facets` 自体は I/O fallback を持たず、unresolved な node を
    /// 受け取った場合は空合成結果になる（実 production では load 経路で先に弾かれる）。
    #[test]
    fn resolve_with_missing_facet_returns_error() {
        let tmp = TempDir::new().unwrap();
        let node = make_facet_node(Some("nonexistent"), None, None);
        let result = resolve_node_facets(&node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn resolve_with_missing_knowledge_returns_error() {
        let tmp = TempDir::new().unwrap();
        let node = make_facet_node(None, Some("nonexistent"), None);
        let result = resolve_node_facets(&node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn resolve_with_missing_instruction_returns_error() {
        let tmp = TempDir::new().unwrap();
        let node = make_facet_node(None, None, Some("nonexistent"));
        let result = resolve_node_facets(&node, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    // --- compose → Artifact reference rendering パイプライン結合テスト ---

    #[test]
    fn compose_and_render_pipeline() {
        use crate::adaptor::gateway::workflow::prompt_rendering;

        let tmp = TempDir::new().unwrap();
        let policies = tmp.path().join("policies");
        let instructions = tmp.path().join("instructions");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            policies.join("coding.md"),
            "Coding rules for {{ request }}.",
        )
        .unwrap();
        std::fs::write(instructions.join("impl.md"), "Task: {{ request }}").unwrap();

        let node = make_facet_node(Some("coding"), None, Some("impl"));
        let resolved = resolve_node_facets(&node, tmp.path()).unwrap();
        let composed = compose_facets(Some(&resolved));

        let task = Some("Fix the bug");

        let (_system_prompt, rendered_user) = prompt_rendering::build_node_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            task,
            &std::collections::HashMap::new(),
            &std::collections::BTreeMap::new(),
        )
        .expect("prompt rendering");
        let rendered_instruction = prompt_rendering::render_node_workflow_instruction(
            &node,
            Some(&resolved),
            task,
            &std::collections::HashMap::new(),
        )
        .expect("workflow instruction");

        assert_eq!(
            composed.system_prompt.as_deref(),
            Some("Coding rules for {{ request }}.")
        );
        assert_eq!(rendered_user, "Task: Fix the bug");
        assert_eq!(rendered_instruction, "Task: Fix the bug");
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
        let builtin_count = builtin::list_builtin_facet_keys(FacetKind::Policy).len();
        assert_eq!(summaries.len(), builtin_count + 1);

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

    // --- Artifact template rendering ---

    #[test]
    fn find_undefined_template_variables_returns_only_invalid_reference_syntax() {
        let content = "{{ request }} {{ plan.summary }} {{ request.more }} {{ bad ref }}";
        let undefined = prompt_rendering::find_undefined_template_variables(content);
        assert_eq!(
            undefined,
            vec!["request.more".to_string(), "bad ref".to_string()]
        );
    }

    #[test]
    fn render_template_variables_treats_surrounding_whitespace_inside_refs_as_equivalent() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("request".to_string(), "do".to_string());
        let out = prompt_rendering::render_template_variables("a {{ request }} b", &vars);
        assert_eq!(out, "a do b");
    }

    #[test]
    fn render_template_variables_keeps_unresolved_ref_verbatim_including_whitespace() {
        // 解決できない参照は元の `{{ ... }}` をそのまま残し、内側のスペースを変更しない。
        let vars = std::collections::HashMap::new();
        let out = prompt_rendering::render_template_variables("x {{ unknown }} y", &vars);
        assert_eq!(out, "x {{ unknown }} y");
    }
}
