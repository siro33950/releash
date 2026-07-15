use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkflowExecutionId(String);

impl WorkflowExecutionId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if is_uuid_like(&value) {
            Ok(Self(value))
        } else {
            Err(WorkflowError::validation(format!(
                "invalid execution_id: {value}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkflowExecutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkflowDefinitionName(String);

impl WorkflowDefinitionName {
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

impl std::fmt::Display for WorkflowDefinitionName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeDefinitionName(String);

impl NodeDefinitionName {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WorkflowError::validation("node name must not be empty"));
        }
        Ok(Self(value))
    }
}

impl std::fmt::Display for NodeDefinitionName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceWorktreePath(String);

impl WorkspaceWorktreePath {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WorkflowError::validation("worktree path must not be empty"));
        }
        Ok(Self(value.trim_end_matches('/').to_string()))
    }
}

impl std::fmt::Display for WorkspaceWorktreePath {
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
    fn test_workflow_execution_id_uuid形式のみ受理する() {
        assert!(WorkflowExecutionId::new("00000000-0000-4000-8000-000000000001").is_ok());
        assert!(WorkflowExecutionId::new("../bad").is_err());
        assert!(WorkflowExecutionId::new("not-a-uuid").is_err());
    }

    #[test]
    fn test_workflow_definition_name_path要素を拒否する() {
        assert!(WorkflowDefinitionName::new("review").is_ok());
        assert!(WorkflowDefinitionName::new("../review").is_err());
        assert!(WorkflowDefinitionName::new("foo/bar").is_err());
        assert!(WorkflowDefinitionName::new("bad name!").is_err());
        assert!(WorkflowDefinitionName::new("_bad").is_err());
    }

    #[test]
    fn test_workspace_worktree_path末尾スラッシュを正規化する() {
        let path = WorkspaceWorktreePath::new("/tmp/repo/").unwrap();
        assert_eq!(path.to_string(), "/tmp/repo");
    }
}
