use super::*;
#[test]
fn test_受信版_初期状態後だけ差分を受理し逆行と別起動を拒否する() {
    // Given
    let mut cursor = Cursor::default();
    let version = Version {
        epoch: "a".into(),
        sequence: 4,
    };
    // When / Then
    assert!(cursor.accept(version.clone(), false).is_err());
    cursor.accept(version.clone(), true).unwrap();
    cursor.accept(version.clone(), false).unwrap();
    assert!(cursor
        .accept(
            Version {
                sequence: 3,
                ..version.clone()
            },
            false
        )
        .is_err());
    let restarted = Version {
        epoch: "b".into(),
        sequence: 0,
    };
    assert!(cursor.accept(restarted.clone(), false).is_err());
    cursor.accept(restarted.clone(), true).unwrap();
    assert_eq!(cursor.version(), Some(&restarted));
}
