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
pub struct ExecutionTreeId(String);

impl ExecutionTreeId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if is_uuid_like(&value)
            || value.strip_prefix("agent-session-").is_some_and(|suffix| {
                suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
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

impl std::fmt::Display for ExecutionTreeId {
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
#[path = "ids_test.rs"]
mod ids_tests;
