use super::builtin;
use super::schema::{FacetRefs, WorkflowDefinitionYaml};
#[cfg(any(test, feature = "test-support"))]
use super::storage;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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

pub use crate::domain::workflow::FacetKind;

pub use crate::domain::workflow::{FacetContents, WorkflowFacetContents};

pub fn validate_facet_key(key: &str) -> Result<(), FacetError> {
    crate::domain::workflow::FacetKey::new(key)
        .map(|_| ())
        .map_err(|_| FacetError::InvalidKey {
            key: key.to_string(),
        })
}

pub fn load_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<String, FacetError> {
    validate_facet_key(key)?;
    load_facet_with_source(kind, key, base_dir).map(|(_, content)| content)
}

pub(super) fn load_facet_with_source(
    kind: FacetKind,
    key: &str,
    base_dir: &Path,
) -> Result<(crate::domain::workflow::WorkflowSource, String), FacetError> {
    use crate::domain::workflow::WorkflowSource;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    match fs::read_to_string(&path) {
        Ok(content) => return Ok((WorkflowSource::File { path }, content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(content) = builtin::get_builtin_facet(kind, key) {
        return Ok((WorkflowSource::Builtin, content.to_string()));
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
    Ok(builtin::get_builtin_facet(kind, key).is_some()
        || base_dir
            .join(kind.dir_name())
            .join(format!("{key}.md"))
            .try_exists()?)
}

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
pub fn delete_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<(), FacetError> {
    validate_facet_key(key)?;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(FacetError::NotFound {
            kind,
            key: key.to_string(),
        }),
        Err(error) => Err(error.into()),
    }
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

pub fn list_facet_summaries(
    kind: FacetKind,
    base_dir: &Path,
) -> Result<Vec<crate::domain::workflow::FacetSummary>, FacetError> {
    use crate::domain::workflow::{Facet, FacetKey};
    let mut summaries = Vec::new();
    for key in list_facets(kind, base_dir)? {
        let Ok(facet_key) = FacetKey::new(key.clone()) else {
            summaries.push(crate::domain::workflow::FacetSummary {
                key,
                kind: kind.canonical_name().to_string(),
                description: String::new(),
                builtin: false,
            });
            continue;
        };
        let (source, content) = load_facet_with_source(kind, &key, base_dir)?;
        let summary =
            Facet::restore(kind, facet_key, base_dir.to_path_buf(), source, content).summary();
        summaries.push(summary);
    }
    Ok(summaries)
}

#[cfg(any(test, feature = "test-support"))]
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
