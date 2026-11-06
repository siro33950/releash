use super::*;
use crate::domain::daemon::data_home::{DataEntry, DataEntryKind, LegacyServer, RelocationBlock};
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use std::sync::Mutex;

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Survey(DataHome),
    Relocate(DataHome, Vec<String>),
}
struct Store {
    calls: Mutex<Vec<Call>>,
    survey: Result<RelocationSurvey, TechnicalFailure>,
    relocate: Result<(), TechnicalFailure>,
}
impl DataHomeStore for Store {
    fn survey(&self, home: &DataHome) -> Result<RelocationSurvey, TechnicalFailure> {
        self.calls.lock().unwrap().push(Call::Survey(home.clone()));
        self.survey.clone()
    }
    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Relocate(home.clone(), entries.to_vec()));
        self.relocate.clone()
    }
}
fn survey() -> RelocationSurvey {
    RelocationSurvey {
        legacy_entries: vec![DataEntry {
            name: "store".into(),
            kind: DataEntryKind::File,
            bytes: 1,
        }],
        current_entries: vec![],
        legacy_server: LegacyServer::Stopped,
        relocation_started: false,
    }
}
fn failure() -> TechnicalFailure {
    TechnicalFailure {
        nature: TechnicalFailureNature::Transient,
        message: "storage failure".into(),
    }
}
#[test]
fn test_保存先移行_観測してから同じ保存先と判断された項目を渡す() {
    // Given
    for result in [Ok(()), Err(failure())] {
        let store = Arc::new(Store {
            calls: Mutex::new(vec![]),
            survey: Ok(survey()),
            relocate: result.clone(),
        });
        let home = DataHome {
            current: "new".into(),
            legacy: Some("old".into()),
        };
        let usecase = DataHomeRelocationUsecase::new(store.clone());
        // When
        let actual = usecase.run(&home);
        // Then
        assert_eq!(actual, result.map_err(RelocationError::Technical));
        assert_eq!(
            *store.calls.lock().unwrap(),
            vec![
                Call::Survey(home.clone()),
                Call::Relocate(home, vec!["store".into()])
            ]
        );
    }
}
#[test]
fn test_保存先移行_不要と拒否と観測失敗では移動しない() {
    // Given
    for (observed, expected) in [
        (
            Ok(RelocationSurvey {
                legacy_entries: vec![],
                ..survey()
            }),
            Ok(()),
        ),
        (
            Ok(RelocationSurvey {
                legacy_server: LegacyServer::Running,
                ..survey()
            }),
            Err(RelocationError::Blocked(
                RelocationBlock::LegacyServerRunning,
            )),
        ),
        (
            Ok(RelocationSurvey {
                legacy_server: LegacyServer::Unverifiable,
                ..survey()
            }),
            Err(RelocationError::Blocked(
                RelocationBlock::LegacyServerUnverifiable,
            )),
        ),
        (
            Ok(RelocationSurvey {
                current_entries: survey().legacy_entries,
                ..survey()
            }),
            Err(RelocationError::Blocked(RelocationBlock::BothHaveData)),
        ),
        (Err(failure()), Err(RelocationError::Technical(failure()))),
    ] {
        let store = Arc::new(Store {
            calls: Mutex::new(vec![]),
            survey: observed,
            relocate: Ok(()),
        });
        let home = DataHome {
            current: "new".into(),
            legacy: Some("old".into()),
        };
        // When
        let actual = DataHomeRelocationUsecase::new(store.clone()).run(&home);
        // Then
        assert_eq!(actual, expected);
        assert_eq!(*store.calls.lock().unwrap(), vec![Call::Survey(home)]);
    }
}
