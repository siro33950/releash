use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub draft: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrStatus {
    pub open_prs: HashMap<String, PrInfo>,
    pub completed_prs: HashMap<String, PrInfo>,
}

impl PrStatus {
    pub fn for_branch(&self, branch: &str) -> Option<&PrInfo> {
        self.open_prs
            .get(branch)
            .or_else(|| self.completed_prs.get(branch))
    }

    pub fn branch_is_merged(&self, branch: &str, merged_in_git: bool) -> bool {
        merged_in_git
            || (!self.open_prs.contains_key(branch)
                && self
                    .completed_prs
                    .get(branch)
                    .is_some_and(|pr| pr.state == PrState::Merged))
    }
}

#[cfg(test)]
#[path = "pr_test.rs"]
mod pr_tests;
