use crate::domain::failure::TechnicalFailure;
use std::path::{Path, PathBuf};

/// サーバの保存先。既定の保存先で起動するときだけ、移行前の旧既定の保存先を一つ持つ。
/// 明示指定された保存先（`--data-dir`、`RELEASH_DATA_DIR`）は旧保存先を持たず、移行の対象にならない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataHome {
    pub(crate) current: PathBuf,
    pub(crate) legacy: Option<PathBuf>,
}

impl DataHome {
    /// 起動する保存先が既定の保存先かを決め、既定なら旧既定の保存先を持たせる。
    /// 比較は実体のパスで行う。既定の保存先を解決できないときは旧保存先を持たない。
    pub(crate) fn for_startup(
        current: PathBuf,
        default_current: Option<&Path>,
        default_legacy: Option<&Path>,
    ) -> Self {
        let _ = (current, default_current, default_legacy);
        todo!()
    }
}

/// 保存先の直下にある一つの項目。`name` は保存先からの相対パスの先頭の名前である。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataEntry {
    pub(crate) name: String,
    pub(crate) kind: DataEntryKind,
    /// ファイルの大きさ。ディレクトリは、その中の全ファイルの大きさの合計。
    pub(crate) bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataEntryKind {
    File,
    Directory,
}

impl DataEntry {
    /// 利用者の定義（`workflows`、`prompts`）と、中身が空のファイル、中身が空のディレクトリは、サーバのデータでない。
    /// 移行の印（`RELOCATION_MARKER`）もサーバのデータでない。
    pub(crate) fn is_server_data(&self) -> bool {
        todo!()
    }
}

/// 旧保存先に残る `client-api.json` から判定した、旧保存先のサーバの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyServer {
    /// `client-api.json` が無い。
    Absent,
    /// 記録はあるが、その process は終了している。
    Stopped,
    /// 記録の process が動いている。
    Running,
    /// process の生存を確かめられない。
    Unverifiable,
}

/// 移行の途中であることを示す、旧保存先に置く印のファイル名。移動を始める前に置き、旧保存先ごと消える。
pub(crate) const RELOCATION_MARKER: &str = ".relocating";

/// 移行を判断するための、新旧の保存先の観測結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelocationSurvey {
    /// 旧保存先の項目。旧保存先が無ければ空。
    pub(crate) legacy_entries: Vec<DataEntry>,
    /// 新保存先の項目。新保存先が無ければ空。
    pub(crate) current_entries: Vec<DataEntry>,
    pub(crate) legacy_server: LegacyServer,
    /// 旧保存先に `RELOCATION_MARKER` がある。前回の移行が途中で止まっている。
    pub(crate) relocation_started: bool,
}

/// 移行の判断の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelocationPlan {
    /// 移すものが無い。旧保存先が無い、旧保存先にサーバのデータが無い、または保存先が既定でない。
    NotNeeded,
    /// 旧保存先の項目を名前の順に新保存先へ移し、旧保存先を消す。
    Move { entries: Vec<String> },
}

/// 移行を行わない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelocationBlock {
    /// 旧保存先のサーバが動いている。
    LegacyServerRunning,
    /// 旧保存先のサーバの停止を確かめられない。
    LegacyServerUnverifiable,
    /// 新旧の両方にサーバのデータがある。
    BothHaveData,
}

impl RelocationSurvey {
    /// 観測結果から移行を判断する。
    ///
    /// - 旧保存先にサーバのデータが無ければ `NotNeeded`。
    /// - 旧保存先のサーバが `Running` なら `LegacyServerRunning`、`Unverifiable` なら `LegacyServerUnverifiable`。
    /// - 旧保存先に印が無く、新保存先にサーバのデータがあれば `BothHaveData`。
    /// - 印があれば前回の続きとして、旧保存先に残る項目を移す。
    /// - 移す項目は旧保存先の項目のうち、利用者の定義（`workflows`、`prompts`）を除く全部である。
    pub(crate) fn decide(&self) -> Result<RelocationPlan, RelocationBlock> {
        todo!()
    }
}

/// 保存先の観測と移動。adaptor/gateway が実装する。
pub(crate) trait DataHomeStore: Send + Sync {
    /// 新旧の保存先を観測する。旧保存先が無い `DataHome` には、旧保存先が無いものとして答える。
    fn survey(&self, home: &DataHome) -> Result<RelocationSurvey, TechnicalFailure>;
    /// 旧保存先に印を置き、`entries` を一つずつ新保存先へ移し、旧保存先を消す。
    /// 移動は項目ごとに原子的に行う。同じ項目を二つの process が同時に移しても、先に移した方が有効で、
    /// 後の方は失敗にしない。新保存先に同じ名前のディレクトリがあれば中身を合わせ、同じ名前の空のファイルは置き換える。
    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure>;
}

#[cfg(test)]
#[path = "data_home_test.rs"]
mod data_home_tests;
