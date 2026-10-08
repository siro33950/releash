use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrStatus {
    pub open_prs: HashMap<String, PrInfo>,
    pub merged_branches: Vec<String>,
}

impl PrStatus {
    pub fn branch_is_merged(&self, branch: &str, merged_in_git: bool) -> bool {
        merged_in_git
            || (!self.open_prs.contains_key(branch)
                && self.merged_branches.iter().any(|name| name == branch))
    }
}

#[cfg(test)]
#[path = "pr_test.rs"]
mod pr_tests;
