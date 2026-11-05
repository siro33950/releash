use crate::domain::daemon::data_home::{
    DataHome, DataHomeStore, RelocationBlock, RelocationPlan, RelocationSurvey,
};
use crate::domain::failure::TechnicalFailure;
use std::sync::Arc;

/// 旧既定の保存先のデータを、サーバを起動する前に新既定の保存先へ移す手順。
pub(crate) struct DataHomeRelocationUsecase {
    store: Arc<dyn DataHomeStore>,
}

/// 移行の手順の失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelocationError {
    /// 移行を行わなかった。サーバはこの保存先で起動しない。
    Blocked(RelocationBlock),
    /// 観測か移動に失敗した。
    Technical(TechnicalFailure),
}

impl DataHomeRelocationUsecase {
    pub(crate) fn new(store: Arc<dyn DataHomeStore>) -> Self {
        Self { store }
    }

    /// 保存先を観測し、判断に従って移す。移すものが無ければ何もしない。
    pub(crate) fn run(&self, home: &DataHome) -> Result<(), RelocationError> {
        let survey: RelocationSurvey = self.store.survey(home).map_err(RelocationError::Technical)?;
        match survey.decide().map_err(RelocationError::Blocked)? {
            RelocationPlan::NotNeeded => Ok(()),
            RelocationPlan::Move { entries } => self
                .store
                .relocate(home, &entries)
                .map_err(RelocationError::Technical),
        }
    }
}

#[cfg(test)]
#[path = "data_home_test.rs"]
mod data_home_tests;
