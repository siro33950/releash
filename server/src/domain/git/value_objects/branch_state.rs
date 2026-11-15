/// worktree が checkout しているブランチと、その状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeBranch {
    /// ブランチ名。rebase の途中は、rebase しているブランチの名前。
    pub name: String,
    pub state: BranchState,
}

/// ブランチの状態。upstream との関係か、進行中の merge・rebase のどちらかである。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchState {
    /// upstream がある。
    Tracking(Tracking),
    /// upstream が無い。
    Unpublished {
        /// origin のどのブランチにも無い commit の数。
        unpushed: u32,
    },
    /// merge か rebase の途中である。
    Operating(Operation),
}

/// upstream との差。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracking {
    /// upstream の名前。remote の名前を先頭に付けた、`origin/feature/thread-ui` の形。
    pub upstream: String,
    /// 先行の commit 数。ブランチにあり、upstream に無い commit の数。
    pub ahead: u32,
    /// 遅れの commit 数。upstream にあり、ブランチに無い commit の数。
    pub behind: u32,
}

impl Tracking {
    /// commit を 1 つ作った後の、先行の commit 数を返す。
    pub fn ahead_after_commit(&self) -> u32 {
        self.ahead.saturating_add(1)
    }
}

/// 進行中の merge か rebase。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub kind: OperationKind,
    /// 続行で作る commit のメッセージ。rebase では適用している commit のメッセージ、merge では merge の commit のメッセージ。
    /// 続行で作る commit が無いとき（rebase が `break` か、`exec` の失敗で止まっているとき）は `None`。
    pub message: Option<String>,
}

/// 進行中の操作の種類。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationKind {
    Merge,
    Rebase(RebaseProgress),
}

/// rebase の進み具合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseProgress {
    /// rebase の先。ブランチ名。ブランチ名を求められないときは commit の短い ID。
    pub onto: String,
    /// 適用している commit が、何番目か。1 から数える。
    pub step: u32,
    /// rebase で適用する commit の数。
    pub total: u32,
}

/// ブランチの commit の送り方。送り先も決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// upstream に送る。送り先は upstream がある remote である。
    ToUpstream,
    /// origin に同じ名前のブランチを作って送り、それを upstream にする。
    Publish,
    /// upstream を `--force-with-lease` で上書きして送る。送り先は upstream がある remote である。
    ForceWithLease,
}

#[cfg(test)]
#[path = "branch_state_test.rs"]
mod branch_state_tests;
