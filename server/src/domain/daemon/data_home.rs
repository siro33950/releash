use crate::domain::failure::TechnicalFailure;
use std::path::PathBuf;

/// サーバの保存先。既定の保存先で起動するときだけ、移行前の旧既定の保存先を一つ持つ。
/// 明示指定された保存先（`--data-dir`、`RELEASH_DATA_DIR`）は旧保存先を持たず、移行の対象にならない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataHome {
    pub(crate) current: PathBuf,
    pub(crate) legacy: Option<PathBuf>,
}

const USER_DEFINITIONS: [&str; 2] = ["workflows", "prompts"];
const RUNTIME_FILES: [&str; 3] = ["desktop.lock", "desktop.sock", "local-event-store.lock"];

pub(crate) fn retained_entry(name: &str) -> bool {
    USER_DEFINITIONS.contains(&name) || RUNTIME_FILES.contains(&name)
}

/// 保存先の直下にある一つの項目。`name` は保存先からの相対パスの先頭の名前である。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataEntry {
    pub(crate) name: String,
    /// ファイルの大きさ。ディレクトリは、その中の全ファイルの大きさの合計。
    pub(crate) bytes: u64,
}

impl DataEntry {
    /// 利用者の定義、実行中のロック・ソケット、中身が空の項目は、移行するサーバのデータでない。
    /// 移行の印（`RELOCATION_MARKER`）もサーバのデータでない。
    pub(crate) fn is_server_data(&self) -> bool {
        self.bytes > 0 && self.name != RELOCATION_MARKER && !retained_entry(&self.name)
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

/// 旧保存先の `client-api.json` から読んだ記録。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyServerRecord {
    /// ファイルが無い。
    Missing,
    /// 読めない、または解釈できない。
    Unreadable,
    Recorded {
        pid: u32,
        process_started_at: u64,
    },
}

/// 記録の pid について OS から得た process の開始時刻。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessLookup {
    /// process 一覧を取得できない。
    Unavailable,
    /// その pid の process が無い。
    NotFound,
    StartedAt(u64),
}

impl LegacyServer {
    /// 記録と process の観測から、旧保存先のサーバの状態を決める。
    /// pid か開始時刻が 0 の記録は確かめられないものとし、process を観測しない。
    pub(crate) fn judge(
        record: LegacyServerRecord,
        lookup: impl FnOnce(u32) -> ProcessLookup,
    ) -> Self {
        let (pid, recorded) = match record {
            LegacyServerRecord::Missing => return Self::Absent,
            LegacyServerRecord::Recorded {
                pid,
                process_started_at,
            } if pid != 0 && process_started_at != 0 => (pid, process_started_at),
            LegacyServerRecord::Unreadable | LegacyServerRecord::Recorded { .. } => {
                return Self::Unverifiable;
            }
        };
        match lookup(pid) {
            ProcessLookup::StartedAt(start) if start == recorded => Self::Running,
            ProcessLookup::NotFound => Self::Stopped,
            ProcessLookup::StartedAt(start) if start != 0 => Self::Stopped,
            ProcessLookup::StartedAt(_) | ProcessLookup::Unavailable => Self::Unverifiable,
        }
    }
}

/// 移行の途中であることを示す、旧保存先に置く印のファイル名。移動を始める前に置き、旧保存先ごと消える。
pub(crate) const RELOCATION_MARKER: &str = ".relocating";

/// 旧保存先の観測結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacySurvey {
    /// 旧保存先の項目。旧保存先が無ければ空。
    pub(crate) entries: Vec<DataEntry>,
    pub(crate) server: LegacyServer,
    /// 旧保存先に `RELOCATION_MARKER` がある。前回の移行が途中で止まっている。
    pub(crate) relocation_started: bool,
}

impl LegacySurvey {
    /// 旧保存先にサーバのデータがあるか。無ければ新保存先を調べずに移行しない。
    pub(crate) fn has_server_data(&self) -> bool {
        self.entries.iter().any(DataEntry::is_server_data)
    }
}

/// 移行を判断するための、新旧の保存先の観測結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelocationSurvey {
    pub(crate) legacy: LegacySurvey,
    /// 新保存先の直下の項目。新保存先が無ければ空。
    pub(crate) current_entries: Vec<DataEntry>,
}

/// 移動元か移動先にある項目の、移動の判断に使う観測。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryShape {
    File {
        bytes: u64,
    },
    Directory,
    /// symlink など、ファイルでもディレクトリでもないもの。
    Other,
}

/// 一つの項目の移し方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveAction {
    /// 移動元が無い。他の process が先に移した。
    Skip,
    /// 移動先へ名前を変えて移す。移動先が空のファイルなら置き換える。
    Rename,
    /// 中身を一つずつ同じ規則で移し、空になった移動元を消す。
    MergeDirectory,
    /// 移動先を上書きすることになるので移さない。
    Refuse,
}

impl MoveAction {
    pub(crate) fn decide(source: Option<EntryShape>, destination: Option<EntryShape>) -> Self {
        match (source, destination) {
            (None, _) => Self::Skip,
            (Some(_), None) => Self::Rename,
            (Some(EntryShape::Directory), Some(EntryShape::Directory)) => Self::MergeDirectory,
            (Some(EntryShape::File { .. }), Some(EntryShape::File { bytes: 0 })) => Self::Rename,
            (Some(_), Some(_)) => Self::Refuse,
        }
    }
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

/// 移行の失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelocationError {
    /// 移行を行わなかった。サーバはこの保存先で起動しない。
    Blocked(RelocationBlock),
    /// 観測か移動に失敗した。
    Technical(TechnicalFailure),
}

impl From<RelocationError> for super::StartupFailureKind {
    fn from(error: RelocationError) -> Self {
        match error {
            RelocationError::Blocked(block) => Self::DataRelocationBlocked(block),
            RelocationError::Technical(failure) => Self::DataRelocationFailed(failure.nature),
        }
    }
}

impl RelocationSurvey {
    /// 観測結果から移行を判断する。
    ///
    /// - 旧保存先にサーバのデータが無ければ（`LegacySurvey::has_server_data`）`NotNeeded`。
    /// - 旧保存先のサーバが `Running` なら `LegacyServerRunning`、`Unverifiable` なら `LegacyServerUnverifiable`。
    /// - 旧保存先に印が無く、新保存先にサーバのデータがあれば `BothHaveData`。
    /// - 印があれば前回の続きとして、旧保存先に残る項目を移す。
    /// - 移す項目は旧保存先の項目のうち、利用者の定義（`workflows`、`prompts`）を除く全部である。
    pub(crate) fn decide(&self) -> Result<RelocationPlan, RelocationBlock> {
        if !self.legacy.has_server_data() {
            return Ok(RelocationPlan::NotNeeded);
        }
        match self.legacy.server {
            LegacyServer::Running => return Err(RelocationBlock::LegacyServerRunning),
            LegacyServer::Unverifiable => return Err(RelocationBlock::LegacyServerUnverifiable),
            LegacyServer::Absent | LegacyServer::Stopped => {}
        }
        if !self.legacy.relocation_started
            && self.current_entries.iter().any(DataEntry::is_server_data)
        {
            return Err(RelocationBlock::BothHaveData);
        }
        let mut entries: Vec<String> = self
            .legacy
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .filter(|name| name != RELOCATION_MARKER && !retained_entry(name))
            .collect();
        entries.sort();
        Ok(RelocationPlan::Move { entries })
    }
}

/// 保存先の観測と移動。adaptor/gateway が実装する。
/// 呼び出し側は、新保存先の起動ロックを持った状態で呼ぶ。同じ保存先に対して同時に呼ばれることはない。
pub(crate) trait DataHomeStore: Send + Sync {
    /// 旧保存先を観測する。旧保存先が無い `DataHome` には、項目が無く、サーバが `Absent` で、印も無いものとして答える。
    fn survey_legacy(&self, home: &DataHome) -> Result<LegacySurvey, TechnicalFailure>;
    /// 新保存先の直下の項目を観測する。新保存先が無ければ空。
    fn current_entries(&self, home: &DataHome) -> Result<Vec<DataEntry>, TechnicalFailure>;
    /// 旧保存先に印を置き、`entries` を一つずつ新保存先へ移し、旧保存先を消す。
    /// 移動は項目ごとに原子的に行う。新保存先に同じ名前のディレクトリがあれば中身を合わせ、同じ名前の空のファイルは置き換える。
    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure>;
}

#[cfg(test)]
#[path = "data_home_test.rs"]
mod data_home_tests;
