use crate::domain::daemon::data_home::{DataHome, DataHomeStore, RelocationSurvey};
use crate::domain::failure::TechnicalFailure;

/// ファイルシステム上の保存先を観測し、項目を `rename` で移す `DataHomeStore`。
/// 旧保存先のサーバの状態は、旧保存先の `client-api.json` と process の開始時刻で判定する。
pub(crate) struct LocalDataHomeStore;

impl DataHomeStore for LocalDataHomeStore {
    fn survey(&self, home: &DataHome) -> Result<RelocationSurvey, TechnicalFailure> {
        let _ = home;
        todo!()
    }

    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure> {
        let _ = (home, entries);
        todo!()
    }
}
