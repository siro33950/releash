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
    pub open_prs: Vec<(String, PrInfo)>,
    pub completed_prs: Vec<(String, PrInfo)>,
}

impl PrStatus {
    pub fn open_for_branch(&self, branch: &str) -> Option<&PrInfo> {
        self.open_prs
            .iter()
            .rev()
            .find(|(name, _)| name == branch)
            .map(|(_, pr)| pr)
    }

    pub fn for_branch(&self, branch: &str) -> Option<&PrInfo> {
        self.open_for_branch(branch).or_else(|| {
            [PrState::Merged, PrState::Closed]
                .into_iter()
                .find_map(|state| {
                    self.completed_prs
                        .iter()
                        .rev()
                        .find(|(name, pr)| name == branch && pr.state == state)
                        .map(|(_, pr)| pr)
                })
        })
    }
}

#[cfg(test)]
#[path = "pr_test.rs"]
mod pr_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrClassification {
    Open,
    Draft,
    Merged,
    Closed,
}
impl PrInfo {
    pub fn classification(&self) -> PrClassification {
        match self.state {
            PrState::Open if self.draft => PrClassification::Draft,
            PrState::Open => PrClassification::Open,
            PrState::Merged => PrClassification::Merged,
            PrState::Closed => PrClassification::Closed,
        }
    }

    /// この PR があるブランチで、新しい PR を作る操作を出すかを決める。
    /// 状態が Closed のときだけ出す。Open、Draft、Merged のときは出さない。
    pub fn allows_new_pull_request(&self) -> bool {
        self.state == PrState::Closed
    }
}
