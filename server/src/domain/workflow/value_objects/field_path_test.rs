use super::*;

#[test]
fn test_field_path_段0個を保持できる() {
    // Given
    let segments: Vec<String> = Vec::new();

    // When
    let path = FieldPath::new(segments);

    // Then
    assert!(path.is_empty());
    assert_eq!(path.segments(), &[] as &[String]);
}

#[test]
fn test_field_path_段の文字種を検査せず保持する() {
    // Given
    let segments = ["a", "legacy flag", "日本語", "field!", "-field"];

    // When
    let path = FieldPath::new(segments);

    // Then
    assert_eq!(
        path.segments(),
        ["a", "legacy flag", "日本語", "field!", "-field"]
    );
}

#[test]
fn test_field_path_参照表面は不正な文字種の段を拒否する() {
    for invalid in ["-field", "_field", "日本語", "field!", "field value"] {
        // Given / When
        let result = FieldPath::from_reference(&format!("root.{invalid}"));

        // Then
        assert_eq!(
            result,
            Err(FieldPathError::InvalidSegment {
                position: 1,
                value: invalid.to_string(),
            })
        );

        let path = FieldPath::new([invalid]);
        assert_eq!(
            path.to_reference("root"),
            Err(FieldPathError::InvalidSegment {
                position: 1,
                value: invalid.to_string(),
            })
        );
    }
}

#[test]
fn test_field_path_参照表記を往復できる() {
    for reference in ["root", "root.field", "root.outer.inner.leaf"] {
        // Given / When
        let (root, path) = FieldPath::from_reference(reference).unwrap();
        let restored = path.to_reference(&root).unwrap();

        // Then
        assert_eq!(restored, reference);
    }
}

#[test]
fn test_field_path_空文字と空白と空の段を拒否する() {
    for invalid in ["", "root field", "root\tfield", "root..field", "root."] {
        // Given / When
        let result = FieldPath::from_reference(invalid);

        // Then
        assert!(result.is_err(), "{invalid:?} must be rejected");
    }
}

#[test]
fn test_field_path_3段以上を保持できる() {
    // Given / When
    let (_, path) = FieldPath::from_reference("root.one.two.three.four").unwrap();

    // Then
    assert_eq!(path.segments(), ["one", "two", "three", "four"]);
}

#[test]
fn test_field_path_field部分のドット表記を構築できる() {
    // Given / When
    let path = FieldPath::from_dotted("outer.inner.leaf").unwrap();
    let legacy_path = FieldPath::from_dotted("legacy flag.enabled").unwrap();

    // Then
    assert_eq!(path.segments(), ["outer", "inner", "leaf"]);
    assert_eq!(path.as_string(), "outer.inner.leaf");
    assert_eq!(legacy_path.segments(), ["legacy flag", "enabled"]);
    assert!(FieldPath::from_dotted("").is_err());
    assert!(FieldPath::from_dotted("outer..leaf").is_err());
}
