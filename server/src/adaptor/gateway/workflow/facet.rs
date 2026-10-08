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

/// workflow source directory から authoring layout の Facet base を解決する。
/// `<dir>/facets` が directory ならそれを使い、無ければ従来 layout の `<dir>` を使う。
pub(crate) fn resolve_facets_base_dir(workflow_source_dir: &Path) -> PathBuf {
    let canonical_dir = workflow_source_dir.join("facets");
    if canonical_dir.is_dir() {
        canonical_dir
    } else {
        workflow_source_dir.to_path_buf()
    }
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
    match fs::read_to_string(&path) {
        Ok(content) => return Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
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
    if dir.try_exists()? {
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
    if dir.try_exists()? {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let content = fs::read_to_string(&path)?;
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
#[path = "facet_test.rs"]
mod facet_tests;
