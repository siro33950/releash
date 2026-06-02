//! repository ドメインの gateway 実装（domain trait の具体実装）。
//!
//! 各 `*Gateway` 構造体は domain trait（および Query 側の usecase port
//! `BranchCardQuery`）を実装する。これらを usecase / query service へ合成する
//! DI 配線（composition root）は controller の責務であり、本モジュールには
//! 置かない（[`crate::adaptor::controller::wiring`]）。

pub(crate) mod branch;
pub(crate) mod branch_card;
pub(crate) mod git_config;
pub(crate) mod log;
pub(crate) mod notify;
pub(crate) mod repo_paths;
pub(crate) mod status;
pub(crate) mod util;
pub(crate) mod worktree;
