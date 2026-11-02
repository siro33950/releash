use std::collections::BTreeMap;

use crate::domain::workflow::WorkflowError;

/// Session の prompt を構成する Facet の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacetKind {
    Policy,
    Knowledge,
    Instruction,
}

impl std::str::FromStr for FacetKind {
    type Err = WorkflowError;

    /// Facet の種類を解釈し、解釈できない指定を拒否する。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "policy" => Ok(Self::Policy),
            "knowledge" => Ok(Self::Knowledge),
            "instruction" => Ok(Self::Instruction),
            _ => Err(WorkflowError::validation(format!(
                "不明なファセットの種類: {value}"
            ))),
        }
    }
}

impl FacetKind {
    /// Facet の種類の正規識別子を返す。
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instruction",
        }
    }

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
    /// Facet の識別子を返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Facet の key が英数字で始まり、英数字・ハイフン・アンダースコアだけを含むことを検証する。
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        let mut chars = value.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(WorkflowError::validation(format!(
                "ファセットキー '{value}' が無効です（先頭は英数字、以降は英数字・ハイフン・アンダースコアのみ許可）"
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
#[path = "facet_test.rs"]
pub(crate) mod facet_tests;
