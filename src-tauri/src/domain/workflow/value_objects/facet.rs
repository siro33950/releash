use std::collections::BTreeMap;

use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacetKind {
    Policy,
    Knowledge,
    Instruction,
}

impl FacetKind {
    /// ストレージ上のディレクトリ名（複数形）。ファイルシステム経路にのみ使う。
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Policy => "policies",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instructions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FacetKey(String);

impl FacetKey {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.starts_with('.')
            || value.contains('/')
            || value.contains('\\')
            || value.contains("..")
        {
            return Err(WorkflowError::validation(format!(
                "invalid facet key: {value}"
            )));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetSummary {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacetContents {
    pub policy: Option<String>,
    pub knowledge: Vec<String>,
    pub instruction: Option<String>,
}

impl FacetContents {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none() && self.knowledge.is_empty() && self.instruction.is_none()
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

    pub fn insert_node(&mut self, node_name: String, contents: FacetContents) {
        self.nodes.insert(node_name, contents);
    }

    pub fn iter_node_contents(&self) -> impl Iterator<Item = (&str, &FacetContents)> {
        self.nodes
            .iter()
            .map(|(node_name, contents)| (node_name.as_str(), contents))
    }
}

#[cfg(test)]
mod facet_tests {
    use super::*;

    #[test]
    fn test_facet_kind_dir_nameは既存ディレクトリ名を返す() {
        assert_eq!(FacetKind::Policy.dir_name(), "policies");
        assert_eq!(FacetKind::Instruction.dir_name(), "instructions");
    }

    #[test]
    fn test_facet_key_path要素を拒否する() {
        assert!(FacetKey::new("coding").is_ok());
        assert!(FacetKey::new("../coding").is_err());
        assert!(FacetKey::new("foo/bar").is_err());
    }
}
