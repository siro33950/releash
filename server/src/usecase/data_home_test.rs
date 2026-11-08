use super::*;
use crate::domain::daemon::data_home::{DataEntry, LegacyServer, LegacySurvey, RelocationBlock};
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use std::sync::Mutex;

#[derive(Debug, PartialEq, Eq)]
enum Call {
    SurveyLegacy(DataHome),
    CurrentEntries(DataHome),
    Relocate(DataHome, Vec<String>),
}
struct Store {
    calls: Mutex<Vec<Call>>,
    legacy: Result<LegacySurvey, TechnicalFailure>,
    current: Result<Vec<DataEntry>, TechnicalFailure>,
    relocate: Result<(), TechnicalFailure>,
}
impl DataHomeStore for Store {
    fn survey_legacy(&self, home: &DataHome) -> Result<LegacySurvey, TechnicalFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::SurveyLegacy(home.clone()));
        self.legacy.clone()
    }
    fn current_entries(&self, home: &DataHome) -> Result<Vec<DataEntry>, TechnicalFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::CurrentEntries(home.clone()));
        self.current.clone()
    }
    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Relocate(home.clone(), entries.to_vec()));
        self.relocate.clone()
    }
}
fn entry(name: &str, bytes: u64) -> DataEntry {
    DataEntry {
        name: name.into(),
        bytes,
    }
}
fn store() -> Store {
    Store {
        calls: Mutex::new(vec![]),
        legacy: Ok(LegacySurvey {
            entries: vec![entry("z", 1), entry("workflows", 1), entry("a", 0)],
            server: LegacyServer::Stopped,
            relocation_started: false,
        }),
        current: Ok(vec![]),
        relocate: Ok(()),
    }
}
fn home() -> DataHome {
    DataHome {
        current: "new".into(),
        legacy: Some("old".into()),
    }
}
fn failure() -> TechnicalFailure {
    TechnicalFailure {
        nature: TechnicalFailureNature::Transient,
        message: "storage failure".into(),
    }
}
#[test]
fn test_保存先移行_旧観測と新観測の順に同じ保存先を渡して判断された項目を移す() {
    // Given
    for result in [Ok(()), Err(failure())] {
        let store = Arc::new(Store {
            relocate: result.clone(),
            ..store()
        });
        let home = home();
        // When
        let actual = DataHomeRelocationUsecase::new(store.clone()).run(&home);
        // Then
        assert_eq!(actual, result.map_err(RelocationError::Technical));
        assert_eq!(
            *store.calls.lock().unwrap(),
            vec![
                Call::SurveyLegacy(home.clone()),
                Call::CurrentEntries(home.clone()),
                Call::Relocate(home, vec!["a".into(), "z".into()])
            ]
        );
    }
}
#[test]
fn test_保存先移行_旧データなしでは新保存先を観測せず移動しない() {
    // Given
    for entries in [
        vec![],
        vec![
            entry("workflows", 1),
            entry("prompts", 1),
            entry(".relocating", 1),
            entry("empty", 0),
        ],
    ] {
        let store = Arc::new(Store {
            legacy: Ok(LegacySurvey {
                entries,
                server: LegacyServer::Unverifiable,
                relocation_started: true,
            }),
            current: Err(failure()),
            ..store()
        });
        let home = home();
        // When
        let actual = DataHomeRelocationUsecase::new(store.clone()).run(&home);
        // Then
        assert_eq!(actual, Ok(()));
        assert_eq!(*store.calls.lock().unwrap(), vec![Call::SurveyLegacy(home)]);
    }
}
#[test]
fn test_保存先移行_各観測の失敗を返して後続を呼ばない() {
    // Given
    for legacy_fails in [true, false] {
        let mut store = store();
        if legacy_fails {
            store.legacy = Err(failure());
        } else {
            store.current = Err(failure());
        }
        let store = Arc::new(store);
        let home = home();
        // When
        let actual = DataHomeRelocationUsecase::new(store.clone()).run(&home);
        // Then
        assert_eq!(actual, Err(RelocationError::Technical(failure())));
        let mut expected = vec![Call::SurveyLegacy(home.clone())];
        if !legacy_fails {
            expected.push(Call::CurrentEntries(home));
        }
        assert_eq!(*store.calls.lock().unwrap(), expected);
    }
}
#[test]
fn test_保存先移行_観測結果の拒否理由を返して移動しない() {
    // Given
    for (server, current, block) in [
        (
            LegacyServer::Running,
            vec![],
            RelocationBlock::LegacyServerRunning,
        ),
        (
            LegacyServer::Unverifiable,
            vec![],
            RelocationBlock::LegacyServerUnverifiable,
        ),
        (
            LegacyServer::Stopped,
            vec![entry("other", 1)],
            RelocationBlock::BothHaveData,
        ),
    ] {
        let mut store = store();
        store.legacy.as_mut().unwrap().server = server;
        store.current = Ok(current);
        let store = Arc::new(store);
        let home = home();
        // When
        let actual = DataHomeRelocationUsecase::new(store.clone()).run(&home);
        // Then
        assert_eq!(actual, Err(RelocationError::Blocked(block)));
        assert_eq!(
            *store.calls.lock().unwrap(),
            vec![Call::SurveyLegacy(home.clone()), Call::CurrentEntries(home)]
        );
    }
}
