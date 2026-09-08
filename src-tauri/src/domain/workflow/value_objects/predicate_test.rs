use super::Predicate::{self, And, Or, Ref};
use super::PredicateError;

#[test]
fn test_述語構築_空の合成は不適合を返す() {
    // Given
    let elements: Vec<Predicate<&str>> = Vec::new();
    // When
    let and = Predicate::and(elements.clone());
    let or = Predicate::or(elements);
    // Then
    assert_eq!(and, Err(PredicateError::Empty));
    assert_eq!(or, Err(PredicateError::Empty));
}

#[test]
fn test_述語構築_一要素以上の合成は演算子と要素を保持する() {
    // Given
    for elements in [
        vec![Ref("passed")],
        vec![Ref("passed"), Ref("clean")],
        vec![Ref("passed"), Or(vec![Ref("clean"), Ref("skipped")])],
    ] {
        // When
        let and = Predicate::and(elements.clone());
        let or = Predicate::or(elements.clone());
        // Then
        assert_eq!(and, Ok(And(elements.clone())));
        assert_eq!(or, Ok(Or(elements)));
    }
}

#[test]
fn test_述語評価_単一参照と要素一つの合成は参照値を返す() {
    // Given
    for value in [false, true] {
        for predicate in [Ref(value), And(vec![Ref(value)]), Or(vec![Ref(value)])] {
            // When
            let actual = predicate.evaluate(&mut |reference| *reference);
            // Then
            assert_eq!(actual, value);
        }
    }
}

#[test]
fn test_述語評価_合成と混在ネストの真理値表() {
    // Given
    for passed in [false, true] {
        for clean in [false, true] {
            for skipped in [false, true] {
                let values = [passed, clean, skipped];
                let cases: [(Predicate<usize>, bool); 3] = [
                    (
                        And(vec![Ref(0), Ref(1), Ref(2)]),
                        passed && clean && skipped,
                    ),
                    (Or(vec![Ref(0), Ref(1), Ref(2)]), passed || clean || skipped),
                    (
                        And(vec![Ref(0), Or(vec![Ref(1), Ref(2)])]),
                        passed && (clean || skipped),
                    ),
                ];
                for (predicate, expected) in cases {
                    // When
                    let actual = predicate.evaluate(&mut |reference| values[*reference]);
                    // Then
                    assert_eq!(actual, expected, "{predicate:?}: {values:?}");
                }
            }
        }
    }
}

#[test]
fn test_述語検査_短絡位置を含む全参照を検査して不適合を保持する() {
    // Given
    let predicate = Or(vec![
        Ref("passed"),
        And(vec![Ref("invalid"), Ref("missing")]),
    ]);
    let mut visited = Vec::new();
    // When
    let errors = predicate.validate(&mut |reference| {
        visited.push(*reference);
        match *reference {
            "passed" => Ok(()),
            other => Err((other, "required booleanではない")),
        }
    });
    // Then
    assert_eq!(visited, ["passed", "invalid", "missing"]);
    assert_eq!(
        errors,
        [
            ("invalid", "required booleanではない"),
            ("missing", "required booleanではない")
        ]
    );
}

#[test]
fn test_述語評価_参照型と値の取得方法を呼び出し側が決める() {
    // Given
    #[derive(Debug)]
    struct Reference {
        index: usize,
    }
    let predicate = And(vec![
        Ref(Reference { index: 0 }),
        Ref(Reference { index: 1 }),
    ]);
    let values = [Some(true), None];
    // When
    let errors = predicate.validate(&mut |reference| {
        (reference.index < values.len())
            .then_some(())
            .ok_or(reference.index)
    });
    let actual = predicate.evaluate(&mut |reference| values[reference.index].unwrap_or(false));
    // Then
    assert!(errors.is_empty());
    assert!(!actual);
}

#[test]
fn test_述語評価_論理演算は不要な参照取得を短絡する() {
    // Given
    for predicate in [
        And(vec![Ref(false), Ref(true)]),
        Or(vec![Ref(true), Ref(false)]),
    ] {
        let mut visited = Vec::new();
        // When
        predicate.evaluate(&mut |reference| {
            visited.push(*reference);
            *reference
        });
        // Then
        assert_eq!(visited.len(), 1);
    }
}
