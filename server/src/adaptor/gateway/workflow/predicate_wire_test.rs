use super::*;

#[test]
fn test_述語の形式変換_空の合成とネストした空の合成は同じ不適合になる() {
    // Given
    for value in [
        serde_json::json!({"and": []}),
        serde_json::json!({"or": []}),
        serde_json::json!({"or": ["passed", {"and": []}]}),
        serde_json::json!({"and": ["passed", {"or": []}]}),
    ] {
        // When
        let error = parse_predicate(&value).unwrap_err();
        // Then
        assert_eq!(error, PredicateShapeError::Empty);
        assert_eq!(
            error.to_string(),
            "predicate and/or must contain at least one element"
        );
    }
}

#[test]
fn test_述語の形式変換_借用parserと直接deserializeで不正shapeを拒否する() {
    // Given
    for (value, expected) in [
        (serde_json::json!({"and": []}), PredicateShapeError::Empty),
        (serde_json::json!({"or": []}), PredicateShapeError::Empty),
        (
            serde_json::json!({"or": "passed"}),
            PredicateShapeError::ExpectedArray,
        ),
        (
            serde_json::json!({"and": null}),
            PredicateShapeError::ExpectedArray,
        ),
        (
            serde_json::json!({"and": ["passed"], "or": ["clean"]}),
            PredicateShapeError::InvalidOperator,
        ),
        (
            serde_json::json!({"not": "passed"}),
            PredicateShapeError::InvalidOperator,
        ),
        (serde_json::json!({}), PredicateShapeError::InvalidOperator),
        (
            serde_json::json!(true),
            PredicateShapeError::InvalidPredicate,
        ),
        (serde_json::json!(42), PredicateShapeError::InvalidPredicate),
        (
            serde_json::json!(null),
            PredicateShapeError::InvalidPredicate,
        ),
        (
            serde_json::json!(["passed"]),
            PredicateShapeError::InvalidPredicate,
        ),
    ] {
        for value in [
            value.clone(),
            serde_json::json!({"or": ["passed", {"and": [value]}]}),
        ] {
            // When
            let parsed = parse_predicate(&value).unwrap_err();
            let deserialized = serde_json::from_value::<Predicate<String>>(value).unwrap_err();
            // Then
            assert_eq!(parsed, expected);
            assert_eq!(deserialized.to_string(), expected.to_string());
        }
    }
}

#[test]
fn test_述語の形式変換_単一参照と論理構造を往復して保持する() {
    // Given
    for value in [
        serde_json::json!("passed"),
        serde_json::json!({"and": ["passed"]}),
        serde_json::json!({"or": ["clean", "skipped"]}),
        serde_json::json!({"and": ["passed", {"or": ["clean", "skipped"]}]}),
    ] {
        // When
        let parsed = parse_predicate(&value).unwrap();
        let predicate: Predicate<String> = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed, predicate);
        let actual = serde_json::to_value(predicate).unwrap();
        // Then
        assert_eq!(actual, value);
    }
}
