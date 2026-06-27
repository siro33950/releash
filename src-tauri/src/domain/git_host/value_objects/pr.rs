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
