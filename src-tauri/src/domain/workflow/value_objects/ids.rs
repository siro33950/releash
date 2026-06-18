use serde::{Deserialize, Serialize};

use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(String);

impl RunId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if is_uuid_like(&value) {
            Ok(Self(value))
        } else {
            Err(WorkflowError::validation(format!(
                "invalid run_id: {value}"
            )))
        }
    }

    pub fn unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowName(String);

impl WorkflowName {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.is_empty() {
            return Err(WorkflowError::validation("ワークフロー名が空です"));
        }
        let mut chars = value.chars();
        let first = chars.next().expect("non-empty workflow name");
        if !first.is_ascii_alphanumeric()
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(WorkflowError::validation(format!(
                "ワークフロー名 '{value}' は先頭を英数字にし、2文字目以降は英数字・ハイフン・アンダースコアのみ使用できます"
            )));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkflowName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeName(String);

impl NodeName {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WorkflowError::validation("node name must not be empty"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorktreePath(String);

impl WorktreePath {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WorkflowError::validation("worktree path must not be empty"));
        }
        Ok(Self(value.trim_end_matches('/').to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorktreePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod ids_tests {
    use super::*;

    #[test]
    fn test_run_id_uuid形式のみ受理する() {
        assert!(RunId::new("00000000-0000-4000-8000-000000000001").is_ok());
        assert!(RunId::new("../bad").is_err());
        assert!(RunId::new("not-a-uuid").is_err());
    }

    #[test]
    fn test_workflow_name_path要素を拒否する() {
        assert!(WorkflowName::new("review").is_ok());
        assert!(WorkflowName::new("../review").is_err());
        assert!(WorkflowName::new("foo/bar").is_err());
        assert!(WorkflowName::new("bad name!").is_err());
        assert!(WorkflowName::new("_bad").is_err());
    }

    #[test]
    fn test_worktree_path末尾スラッシュを正規化する() {
        let path = WorktreePath::new("/tmp/repo/").unwrap();
        assert_eq!(path.as_str(), "/tmp/repo");
    }
}
