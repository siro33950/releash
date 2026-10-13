use super::*;

#[test]
fn test_ディレクトリ状態_子の変更の優先度で決まる() {
    // Given
    let statuses = ["new", "modified", "deleted", "conflicted"];
    // When / Then
    for count in 0..=statuses.len() {
        assert_eq!(
            aggregate_status(statuses[..count].iter().copied()),
            ["none", "new", "modified", "deleted", "conflicted"][count]
        );
    }
}

#[test]
fn test_ディレクトリ状態_renameも変更として色を付ける() {
    // Given / When / Then
    assert_eq!(
        aggregate_status(["none", "renamed"].into_iter()),
        "modified"
    );
}
