/// ブランチ（ローカル / リモート）を表すエンティティ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_remote: bool,
}

impl Branch {
    pub fn needs_creation(name: &str, branches: &[(Self, bool)]) -> bool {
        !branches
            .iter()
            .any(|(branch, _)| !branch.is_remote && branch.name == name)
    }

    pub fn can_create_worktree(&self, has_worktree: bool) -> bool {
        !self.is_remote && !has_worktree
    }

    pub fn is_base_candidate(&self, excluded: Option<&str>) -> bool {
        !self.is_remote && excluded != Some(self.name.as_str())
    }

    pub fn local(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_remote: false,
        }
    }

    pub fn remote(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_remote: true,
        }
    }
}

#[cfg(test)]
#[path = "branch_test.rs"]
pub(crate) mod branch_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchTracking {
    pub upstream: String,
    pub ahead: usize,
    pub behind: usize,
}
