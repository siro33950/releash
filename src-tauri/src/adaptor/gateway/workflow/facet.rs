use super::builtin;
use super::schema::{FacetRefs, WorkflowDefinitionYaml};
use super::storage;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// ファセットの読み込みベースディレクトリ。
///
/// [02] 境界: storage / builtin / driver の caller 全てがここを参照することで、
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

pub use crate::domain::workflow::{FacetContents, WorkflowFacetContents};

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

pub(crate) fn facet_exists(
    kind: FacetKind,
    key: &str,
    base_dir: &Path,
) -> Result<bool, FacetError> {
    validate_facet_key(key)?;
    if builtin::is_builtin_facet(kind, key) {
        return Ok(true);
    }
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    Ok(path.try_exists()?)
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

fn resolve_refs(facets: &FacetRefs, base_dir: &Path) -> Result<FacetContents, FacetError> {
    let resolved_policy = match facets.policy.as_deref() {
        Some(k) => Some(load_facet(FacetKind::Policy, k, base_dir)?),
        None => None,
    };
    let resolved_knowledge = facets
        .knowledge
        .iter()
        .map(|key| load_facet(FacetKind::Knowledge, key, base_dir))
        .collect::<Result<Vec<_>, _>>()?;
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
    use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering;
    use tempfile::TempDir;

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
        let mut expected = builtin::list_builtin_facet_keys(FacetKind::Knowledge)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        expected.push("architecture".to_string());
        expected.sort();
        expected.dedup();
        assert_eq!(keys, expected);
    }

    #[test]
    fn list_facets_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("knowledge")).unwrap();
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        // custom dir は空でも builtin Knowledge facets は含まれる
        let mut expected = builtin::list_builtin_facet_keys(FacetKind::Knowledge)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(keys, expected);
    }

    #[test]
    fn list_facets_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
        // custom dir が存在しなくても builtin Knowledge facets は含まれる
        let mut expected = builtin::list_builtin_facet_keys(FacetKind::Knowledge)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(keys, expected);
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
        let content = "{{ goal }} {{ plan.summary }} {{ plan.a.b }} {{ bad ref }}";
        let undefined = prompt_rendering::find_undefined_template_variables(content);
        assert_eq!(
            undefined,
            vec!["plan.a.b".to_string(), "bad ref".to_string()]
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
