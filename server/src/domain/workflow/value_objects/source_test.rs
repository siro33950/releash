use super::*;

#[test]
fn test_取得元_builtinだけが組み込みである() {
    // Given
    let file = WorkflowSource::File {
        path: "review.yml".into(),
    };
    // When / Then
    assert!(WorkflowSource::Builtin.is_builtin());
    assert!(!file.is_builtin());
}
