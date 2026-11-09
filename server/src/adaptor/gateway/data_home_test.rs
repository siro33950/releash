use super::*;
use crate::domain::daemon::data_home::RelocationSurvey;

fn survey(home: &DataHome) -> RelocationSurvey {
    RelocationSurvey {
        legacy: LocalDataHomeStore.survey_legacy(home).unwrap(),
        current_entries: LocalDataHomeStore.current_entries(home).unwrap(),
    }
}

#[test]
fn test_移行_空ファイルを置換しディレクトリを統合する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("new"),
        legacy: Some(root.path().join("old")),
    };
    let old = home.legacy.as_ref().unwrap();
    fs::create_dir_all(old.join("logs")).unwrap();
    fs::create_dir_all(home.current.join("logs")).unwrap();
    fs::write(old.join("store"), b"data").unwrap();
    fs::write(home.current.join("store"), []).unwrap();
    fs::write(old.join("logs/old"), b"log").unwrap();
    // When
    let store = LocalDataHomeStore;
    let crate::domain::daemon::data_home::RelocationPlan::Move { entries } =
        survey(&home).decide().unwrap()
    else {
        panic!("move");
    };
    store.relocate(&home, &entries).unwrap();
    store.relocate(&home, &entries).unwrap();
    // Then
    assert_eq!(fs::read(home.current.join("store")).unwrap(), b"data");
    assert_eq!(fs::read(home.current.join("logs/old")).unwrap(), b"log");
    assert!(!old.exists());
}

#[test]
fn test_移行_衝突で止まり印と両方のデータを保持する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("new"),
        legacy: Some(root.path().join("old")),
    };
    let old = home.legacy.as_ref().unwrap();
    fs::create_dir_all(old).unwrap();
    fs::create_dir_all(&home.current).unwrap();
    fs::write(old.join("store"), b"old").unwrap();
    fs::write(home.current.join("store"), b"new").unwrap();
    // When / Then
    assert!(LocalDataHomeStore
        .relocate(&home, &["store".into()])
        .is_err());
    assert_eq!(fs::read(old.join("store")).unwrap(), b"old");
    assert_eq!(fs::read(home.current.join("store")).unwrap(), b"new");
    assert!(old.join(RELOCATION_MARKER).exists());
}

#[test]
fn test_観測_破損したサーバ記録は停止扱いにしない() {
    // Given
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("client-api.json"), b"invalid").unwrap();
    // When / Then
    assert_eq!(server(root.path()), LegacyServer::Unverifiable);
}

#[test]
fn test_移行_保存先の外を指す項目を拒否する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("new"),
        legacy: Some(root.path().join("old")),
    };
    fs::create_dir_all(home.legacy.as_ref().unwrap()).unwrap();
    // When / Then
    for name in ["/", "..", "../outside", "nested/file"] {
        assert!(LocalDataHomeStore.relocate(&home, &[name.into()]).is_err());
    }
}

#[test]
fn test_移行_途中失敗後に残りを移してデータと印を回収する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("new"),
        legacy: Some(root.path().join("old")),
    };
    let old = home.legacy.as_ref().unwrap();
    fs::create_dir_all(old).unwrap();
    fs::create_dir_all(&home.current).unwrap();
    fs::write(old.join("first"), b"first").unwrap();
    fs::write(old.join("second"), b"second").unwrap();
    fs::write(home.current.join("second"), b"conflict").unwrap();
    assert!(LocalDataHomeStore
        .relocate(&home, &["first".into(), "second".into()])
        .is_err());
    assert_eq!(fs::read(home.current.join("first")).unwrap(), b"first");
    assert!(old.join(RELOCATION_MARKER).exists());
    fs::remove_file(home.current.join("second")).unwrap();
    // When
    let crate::domain::daemon::data_home::RelocationPlan::Move { entries } =
        survey(&home).decide().unwrap()
    else {
        panic!("move")
    };
    LocalDataHomeStore.relocate(&home, &entries).unwrap();
    // Then
    assert_eq!(fs::read(home.current.join("first")).unwrap(), b"first");
    assert_eq!(fs::read(home.current.join("second")).unwrap(), b"second");
    assert!(!old.exists());
}

#[test]
fn test_観測_旧保存先がなければ現在の内容を走査しない() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("new");
    fs::write(&current, b"not a directory").unwrap();
    // When / Then
    for legacy in [None, Some(root.path().join("missing"))] {
        let survey = LocalDataHomeStore
            .survey_legacy(&DataHome {
                current: current.clone(),
                legacy,
            })
            .unwrap();
        assert!(survey.entries.is_empty());
        assert_eq!(survey.server, LegacyServer::Absent);
        assert!(!survey.relocation_started);
    }
}

#[test]
fn test_新保存先観測_存在しなければ空() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("missing"),
        legacy: None,
    };
    // When / Then
    assert!(LocalDataHomeStore
        .current_entries(&home)
        .unwrap()
        .is_empty());
}

#[test]
fn test_保存先観測_直下の名前と再帰的な容量と移行印を返す() {
    // Given
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("logs")).unwrap();
    fs::write(root.path().join("logs/one"), b"abc").unwrap();
    fs::write(root.path().join("logs/two"), b"de").unwrap();
    fs::write(root.path().join("empty"), []).unwrap();
    fs::write(root.path().join(RELOCATION_MARKER), []).unwrap();
    let home = DataHome {
        current: root.path().to_owned(),
        legacy: Some(root.path().to_owned()),
    };
    // When
    let legacy = LocalDataHomeStore.survey_legacy(&home).unwrap();
    let mut current = LocalDataHomeStore.current_entries(&home).unwrap();
    let mut entries = legacy.entries;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    current.sort_by(|a, b| a.name.cmp(&b.name));
    // Then
    assert_eq!(entries, current);
    assert_eq!(
        entries,
        vec![
            DataEntry {
                name: RELOCATION_MARKER.into(),
                bytes: 0
            },
            DataEntry {
                name: "empty".into(),
                bytes: 0
            },
            DataEntry {
                name: "logs".into(),
                bytes: 5
            },
        ]
    );
    assert_eq!(legacy.server, LegacyServer::Absent);
    assert!(legacy.relocation_started);
}

#[test]
fn test_保存先観測_ディレクトリでなければ失敗を返す() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"data").unwrap();
    let home = DataHome {
        current: path.clone(),
        legacy: Some(path),
    };
    // When / Then
    assert!(LocalDataHomeStore.survey_legacy(&home).is_err());
    assert!(LocalDataHomeStore.current_entries(&home).is_err());
}

#[cfg(unix)]
#[test]
fn test_移行_利用者定義を走査せずロックとソケットを元の場所に残す() {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("new"),
        legacy: Some(root.path().join("old")),
    };
    let old = home.legacy.as_ref().unwrap();
    for directory in [old, &home.current] {
        fs::create_dir_all(directory.join("workflows")).unwrap();
        fs::set_permissions(
            directory.join("workflows"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        fs::write(directory.join("desktop.lock"), b"held").unwrap();
        fs::write(directory.join("local-event-store.lock"), b"held").unwrap();
    }
    let _old_socket = UnixListener::bind(old.join("desktop.sock")).unwrap();
    let _new_socket = UnixListener::bind(home.current.join("desktop.sock")).unwrap();
    fs::write(old.join("store"), b"data").unwrap();
    // When
    let result = survey(&home).decide();
    for directory in [old, &home.current] {
        fs::set_permissions(
            directory.join("workflows"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    let crate::domain::daemon::data_home::RelocationPlan::Move { entries } = result.unwrap() else {
        panic!("move")
    };
    LocalDataHomeStore.relocate(&home, &entries).unwrap();
    // Then
    assert_eq!(fs::read(home.current.join("store")).unwrap(), b"data");
    for directory in [old, &home.current] {
        assert_eq!(fs::read(directory.join("desktop.lock")).unwrap(), b"held");
        assert_eq!(
            fs::read(directory.join("local-event-store.lock")).unwrap(),
            b"held"
        );
        std::os::unix::net::UnixStream::connect(directory.join("desktop.sock")).unwrap();
    }
}
