use super::*;

#[test]
fn test_画像参照_必要項目とsideとversionを検証する() {
    // Given
    let reference =
        "blob?worktree=%2Frepo&path=日本語.png&side=modified&section=changes&base=head&version=7";
    // When
    let blob = parse_blob_reference(reference).unwrap();
    // Then
    assert_eq!(blob.worktree, "/repo");
    assert_eq!(blob.path, "日本語.png");
    assert_eq!(blob.version, 7);
    assert_eq!(blob.side, crate::domain::code::ReviewBlobSide::Modified);
    for invalid in [
        "",
        "review-blob://localhost/blob",
        "blob?path=a.png",
        &reference.replace("modified", "invalid"),
        &reference.replace("version=7", "version=no"),
    ] {
        let error = parse_blob_reference(invalid)
            .err()
            .expect("invalid blob reference");
        assert_eq!(
            crate::adaptor::presenter::connect::command_error(error.into()).code,
            connectrpc::ErrorCode::InvalidArgument,
        );
    }
}

#[test]
fn test_画像参照_古いversionは画像を読まずに拒否する() {
    // Given
    let usecase =
        crate::usecase::review_usecase::tests_support::review_usecase_with_snapshot_version(8);
    let reference = parse_blob_reference(
        "blob?worktree=%2Frepo&path=a.png&side=modified&section=changes&base=head&version=7",
    )
    .unwrap();
    // When
    let error = usecase
        .read_review_blob_bytes(
            &reference.worktree,
            &reference.path,
            reference.side,
            &reference.section,
            &reference.base,
            reference.version,
        )
        .unwrap_err();
    // Then
    assert!(matches!(
        error,
        crate::usecase::code_error::CodeUsecaseError::Code(
            crate::domain::code::CodeError::StaleReviewBlobVersion {
                requested: 7,
                current: 8
            }
        )
    ));
}
