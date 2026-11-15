/// check の分類。gh の分類に従う。並びは、PR の面に出す順である。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CheckBucket {
    /// 失敗。
    Fail,
    /// 取り消し。
    Cancel,
    /// 実行中。
    Pending,
    /// skip。
    Skipping,
    /// 通過。
    Pass,
}

/// GitHub Actions の job の ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionsJobId(pub u64);

/// GitHub が PR に対して報告する検査の 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub bucket: CheckBucket,
    /// 開始の時刻（UNIX 秒）。GitHub が報告していないときは無い。
    pub started_at: Option<u64>,
    /// 終了の時刻（UNIX 秒）。GitHub が報告していないときは無い。
    pub completed_at: Option<u64>,
    /// GitHub Actions の job の ID。外部のサービスが報告する check には無い。
    pub job: Option<ActionsJobId>,
}

impl Check {
    /// 失敗までにかかった秒数を決める。
    /// 分類が失敗で、開始と終了の時刻がどちらもあるときだけ、終了から開始を引いた秒数を返す。
    pub fn failure_seconds(&self) -> Option<u64> {
        if self.bucket != CheckBucket::Fail {
            return None;
        }
        self.completed_at?.checked_sub(self.started_at?)
    }

    /// 実行している時間の起点にする、開始の時刻を決める。
    /// 分類が実行中で、開始の時刻があるときだけ返す。
    pub fn running_since(&self) -> Option<u64> {
        (self.bucket == CheckBucket::Pending)
            .then_some(self.started_at)
            .flatten()
    }

    /// ログを開ける job を決める。
    /// 分類が失敗で、GitHub Actions の job であるときだけ、その job の ID を返す。
    pub fn log_job(&self) -> Option<ActionsJobId> {
        (self.bucket == CheckBucket::Fail)
            .then_some(self.job)
            .flatten()
    }
}

/// PR の check の全体。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks(Vec<Check>);

impl Checks {
    /// GitHub が報告した順の check から作る。
    pub fn new(checks: Vec<Check>) -> Self {
        Self(checks)
    }

    /// その分類の check の件数を数える。
    pub fn count(&self, bucket: CheckBucket) -> usize {
        self.0.iter().filter(|check| check.bucket == bucket).count()
    }

    /// 1 行ずつ出す check を決める。
    /// 失敗、取り消し、実行中の check を、この分類の順に並べる。同じ分類の中は、GitHub が報告した順にする。
    pub fn listed(&self) -> Vec<&Check> {
        let mut checks: Vec<_> = self
            .0
            .iter()
            .filter(|check| {
                matches!(
                    check.bucket,
                    CheckBucket::Fail | CheckBucket::Cancel | CheckBucket::Pending
                )
            })
            .collect();
        checks.sort_by_key(|check| check.bucket);
        checks
    }

    /// たたんで出す、通過の check の名前を、GitHub が報告した順に返す。
    pub fn passed_names(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter(|check| check.bucket == CheckBucket::Pass)
            .map(|check| check.name.as_str())
            .collect()
    }

    /// たたんで出す、skip の check の名前を、GitHub が報告した順に返す。
    pub fn skipped_names(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter(|check| check.bucket == CheckBucket::Skipping)
            .map(|check| check.name.as_str())
            .collect()
    }
}

#[cfg(test)]
#[path = "check_test.rs"]
mod check_tests;
