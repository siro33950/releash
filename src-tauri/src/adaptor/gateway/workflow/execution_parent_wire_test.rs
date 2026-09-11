use super::*;
use serde_json::json;

#[test]
fn test_親参照_永続化済みの3形式を排他的な型へ復元する() {
    // Given
    for (wire, expected) in [
        (
            json!({"parentId": "p"}),
            ExecutionParentRef::sequence_child("p"),
        ),
        (
            json!({"parentId": "p", "fanoutSlot": {"itemIndex": 1, "childIndex": 2}}),
            ExecutionParentRef::fanout_child("p", Some(1), 2),
        ),
        (
            json!({"parentId": "p", "delegate": true}),
            ExecutionParentRef::delegate_child("p"),
        ),
    ] {
        // When
        let parent: ExecutionParentRef = serde_json::from_value(wire.clone()).unwrap();
        // Then
        assert_eq!(parent, expected);
        assert_eq!(serde_json::to_value(parent).unwrap(), wire);
    }
}

#[test]
fn test_親参照_delegateとfanoutの混在は復元できない() {
    // Given
    let wire = json!({"parentId": "p", "delegate": true, "fanoutSlot": {"childIndex": 0}});
    // When / Then
    assert!(serde_json::from_value::<ExecutionParentRef>(wire)
        .unwrap_err()
        .to_string()
        .contains("cannot have a fanout slot"));
}
