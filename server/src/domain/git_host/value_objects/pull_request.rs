use crate::domain::git::{Branch, GitError};
use crate::domain::git_host::{CachedResult, GitHostError};

use super::{Checks, PrConversation, PrInfo, PrStatus};

/// PR の面に出す、PR の内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// 番号、URL、状態、draft かどうか。
    pub info: PrInfo,
    pub title: String,
    /// 本文。Markdown。
    pub body: String,
    pub checks: Checks,
    pub conversation: PrConversation,
}

/// 選んでいる Worktree のブランチの PR。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchPullRequest {
    /// ブランチの PR が無い。
    Absent,
    /// ブランチの PR と、その内容。
    Present(PullRequest),
}

impl BranchPullRequest {
    /// PR の面に出すブランチの PR を決める。
    ///
    /// ブランチの PR は、Repository の PR の一覧 `status` から `PrStatus::for_branch` で決める。
    /// Workspaces のブランチの行と同じ PR になる。
    ///
    /// 値は次のとおりにする。
    /// - 一覧をまだ取れていないときは、無い。
    /// - ブランチの PR が無いときは、`Absent`。
    /// - ブランチの PR があり、持っている内容 `held` がその番号の PR のものであるときは、`Present`。
    /// - ブランチの PR があり、`held` が無いか、別の番号の PR のものであるときは、無い。
    ///
    /// 失敗は、一覧の取得の失敗があればそれにする。
    /// 無ければ、ブランチの PR があるときに限り、内容の取得の失敗にする。
    pub fn resolve(
        status: CachedResult<PrStatus>,
        branch: &str,
        held: CachedResult<PullRequest>,
    ) -> CachedResult<Self> {
        let Some(list) = status.value else {
            return CachedResult {
                value: None,
                error: status.error,
            };
        };
        match list.for_branch(branch) {
            None => CachedResult {
                value: Some(Self::Absent),
                error: status.error,
            },
            Some(info) => CachedResult {
                value: held
                    .value
                    .filter(|pr| pr.info.number == info.number)
                    .map(Self::Present),
                error: status.error.or(held.error),
            },
        }
    }
}

/// これから作る PR。タイトルは空でない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest {
    title: String,
    body: String,
    base: String,
    draft: bool,
}

impl NewPullRequest {
    /// 作れる PR かを決める。空のタイトルは `PullRequestCreationError::EmptyTitle` で拒む。
    /// 本文は空でもよい。
    pub fn new(
        title: String,
        body: String,
        base: String,
        draft: bool,
    ) -> Result<Self, PullRequestCreationError> {
        if title.is_empty() {
            return Err(PullRequestCreationError::EmptyTitle);
        }
        Ok(Self {
            title,
            body,
            base,
            draft,
        })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// 本文。空のこともある。
    pub fn body(&self) -> &str {
        &self.body
    }

    /// base にするブランチの、origin 上の名前。
    pub fn base(&self) -> &str {
        &self.base
    }

    /// draft で作るか。
    pub fn draft(&self) -> bool {
        self.draft
    }
}

/// PR の base に選べるブランチ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestBases {
    /// origin にあるブランチの、origin 上の名前。名前の順に並ぶ。
    pub branches: Vec<String>,
    /// 初めに選ぶ、リポジトリの既定のブランチ。
    pub default_branch: Option<String>,
}

impl PullRequestBases {
    /// base に選べるブランチと、初めに選ぶブランチを決める。
    /// 選べるのは origin にあるブランチ（`Branch::origin_name` が名前を返すもの）で、origin 上の名前を名前の順に並べる。
    /// 既定のブランチは、選べるブランチの中にあるときだけ、初めに選ぶブランチにする。
    pub fn new(branches: &[Branch], default_branch: Option<String>) -> Self {
        let mut branches: Vec<_> = branches
            .iter()
            .filter_map(Branch::origin_name)
            .map(str::to_owned)
            .collect();
        branches.sort();
        let default_branch = default_branch.filter(|name| branches.contains(name));
        Self {
            branches,
            default_branch,
        }
    }
}

/// PR を作れなかった理由。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PullRequestCreationError {
    /// タイトルが空である。
    #[error("PR のタイトルが空です。")]
    EmptyTitle,
    /// origin が GitHub のリポジトリでない。
    #[error("origin が GitHub のリポジトリではないため、PR を作成できません。")]
    NotGitHubRepository,
    /// head にするブランチが origin に無い。ブランチの push が要る。
    #[error("ブランチ {branch} が origin にありません。ブランチを push してから PR を作成してください。")]
    HeadNotOnOrigin { branch: String },
    /// GitHub が作成を拒んだ、または GitHub に届かなかった。
    #[error("{0}")]
    Host(#[from] GitHostError),
    /// Worktree の Repository を読めなかった。
    #[error("{0}")]
    Repository(#[from] GitError),
}

#[cfg(test)]
#[path = "pull_request_test.rs"]
mod pull_request_tests;
