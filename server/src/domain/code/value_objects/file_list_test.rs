use super::*;

#[test]
fn test_名前検索_空入力を拒み元の入力を保持する() {
    // Given / When / Then
    assert!(NameQuery::new("").is_err());
    for input in ["a", " Ab ", " ", "日本"] {
        assert_eq!(NameQuery::new(input).unwrap().as_str(), input);
    }
}

#[test]
fn test_名前検索_文字の順序と個数を守り大小文字と間の文字を無視する() {
    // Given
    let query = NameQuery::new("AbC").unwrap();
    // When / Then
    for name in ["abc", "A_long_Big_Cat", "prefixABCtail"] {
        assert!(query.matches(name), "{name}");
    }
    for name in ["", "ab", "acb", "cba", "unrelated"] {
        assert!(!query.matches(name), "{name}");
    }
    assert!(!NameQuery::new("aa").unwrap().matches("a"));
    assert!(NameQuery::new("aa").unwrap().matches("a_a"));
    assert!(NameQuery::new("日本").unwrap().matches("日々の本"));
}

#[test]
fn test_絞り込み条件_指定の組合せを表し両方なしを拒む() {
    // Given
    let query = NameQuery::new("Ab").unwrap();
    // When / Then
    assert!(FileFilter::new(None, false).is_err());
    for (name, changed, expected) in [
        (Some(query.clone()), false, FileFilter::Name(query.clone())),
        (None, true, FileFilter::Changed),
        (Some(query.clone()), true, FileFilter::NameAndChanged(query)),
    ] {
        let filter = FileFilter::new(name.clone(), changed).unwrap();
        assert_eq!(filter, expected);
        assert_eq!(filter.name(), name.as_ref());
        assert_eq!(filter.changed(), changed);
    }
}
