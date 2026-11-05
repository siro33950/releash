use super::*;

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
        store.survey(&home).unwrap().decide().unwrap()
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
fn test_移行_同じ項目の並行移動を一度だけ適用する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let home = DataHome {
        current: root.path().join("new"),
        legacy: Some(root.path().join("old")),
    };
    fs::create_dir_all(home.legacy.as_ref().unwrap()).unwrap();
    fs::write(home.legacy.as_ref().unwrap().join("store"), b"retained").unwrap();
    // When
    std::thread::scope(|scope| {
        let first = scope.spawn(|| LocalDataHomeStore.relocate(&home, &["store".into()]));
        let second = scope.spawn(|| LocalDataHomeStore.relocate(&home, &["store".into()]));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });
    // Then
    assert_eq!(fs::read(home.current.join("store")).unwrap(), b"retained");
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
