pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn thresholds_are_inclusive_at_boundary_and_limit_above_it() {
        let thresholds = ReviewThresholds::default();

        assert_eq!(
            thresholds.file_size_limit(thresholds.max_file_size_bytes),
            None
        );
        assert_eq!(
            thresholds.file_size_limit(thresholds.max_file_size_bytes + 1),
            Some(ReviewLimitReason::FileSize)
        );
        assert_eq!(thresholds.line_count_limit(thresholds.max_line_count), None);
        assert_eq!(
            thresholds.line_count_limit(thresholds.max_line_count + 1),
            Some(ReviewLimitReason::LineCount)
        );
        assert_eq!(thresholds.hunk_count_limit(thresholds.max_hunk_count), None);
        assert_eq!(
            thresholds.hunk_count_limit(thresholds.max_hunk_count + 1),
            Some(ReviewLimitReason::HunkCount)
        );
        assert_eq!(
            thresholds.tokenization_limit(thresholds.max_tokenization_chars, 1),
            None
        );
        assert_eq!(
            thresholds.tokenization_limit(thresholds.max_tokenization_chars + 1, 1),
            Some(ReviewLimitReason::Tokenization)
        );
    }

    #[test]
    fn review_base_and_section_reject_unknown_values() {
        assert_eq!(ReviewBase::parse("head").unwrap(), ReviewBase::Head);
        assert_eq!(
            ReviewBase::parse("branch-base").unwrap(),
            ReviewBase::BranchBase
        );
        assert!(ReviewBase::parse("main").is_err());

        assert_eq!(
            ReviewSection::parse("changes").unwrap(),
            ReviewSection::Changes
        );
        assert_eq!(
            ReviewSection::parse("staged").unwrap(),
            ReviewSection::Staged
        );
        assert!(ReviewSection::parse("unstaged").is_err());
    }

    #[test]
    fn review_blob_content_type_classifies_images_without_mime() {
        let png = ReviewBlobContentType::from_path("assets/LOGO.PNG");
        assert!(png.is_image());

        let svg = ReviewBlobContentType::image_from_path("icons/app.svg").unwrap();
        assert!(svg.is_image());

        let binary = ReviewBlobContentType::from_path("archive.bin");
        assert!(!binary.is_image());
        assert_eq!(ReviewBlobContentType::image_from_path("archive.bin"), None);
    }
}

#[test]
fn test_比較元選択_既存の比較を保持する() {
    use super::{ReviewBase as Base, ReviewContentSource as Source, ReviewSection as Section};
    use crate::domain::code::ReviewBlobSide as Side;
    // Given / When / Then
    for (base, section, old, new) in [
        (
            Base::Head,
            Section::Changes,
            Source::Staged,
            Source::WorkingTree,
        ),
        (Base::Head, Section::Staged, Source::Head, Source::Staged),
        (
            Base::BranchBase,
            Section::Changes,
            Source::BranchBase,
            Source::WorkingTree,
        ),
    ] {
        assert_eq!(Source::select(Side::Original, section, base), old);
        assert_eq!(Source::select(Side::Modified, section, base), new);
    }
}

#[test]
fn test_表示分類_欠損と非utf8とnulを区別する() {
    use crate::domain::code::ReviewSideBytes;
    // Given / When / Then
    for (bytes, binary) in [
        (ReviewSideBytes::Missing, false),
        (ReviewSideBytes::Present(b"text".to_vec()), false),
        (ReviewSideBytes::Present(vec![0]), true),
        (ReviewSideBytes::Present(vec![255]), true),
    ] {
        assert_eq!(super::review_bytes_are_binary(&bytes), binary);
    }
}

#[test]
fn test_差分対象_比較とsectionごとの所属を判定する() {
    use super::{ReviewBase as Base, ReviewSection as Section};
    // Given / When / Then
    assert!(super::review_target_is_available(
        Section::Changes,
        Base::BranchBase,
        true,
        false,
        false
    ));
    assert!(!super::review_target_is_available(
        Section::Staged,
        Base::BranchBase,
        true,
        true,
        true
    ));
    assert!(super::review_target_is_available(
        Section::Staged,
        Base::Head,
        false,
        true,
        false
    ));
    assert!(!super::review_target_is_available(
        Section::Changes,
        Base::Head,
        true,
        true,
        false
    ));
}

#[test]
fn test_表示分類_画像と属性でのbinaryをdomainが決める() {
    use super::ReviewPresentation as Presentation;
    use crate::domain::code::ReviewSideBytes;
    // Given / When / Then
    let missing = ReviewSideBytes::Missing;
    assert_eq!(
        Presentation::classify("file.png", false, &missing, &missing),
        Presentation::Image
    );
    assert_eq!(
        Presentation::classify("file.rs", true, &missing, &missing),
        Presentation::Binary
    );
    assert_eq!(
        Presentation::classify("file.rs", false, &missing, &missing),
        Presentation::Text
    );
}

#[test]
fn test_初期表示_同じ内容はfileで追加と変更はdiffになる() {
    use super::ReviewPresentation;
    use crate::domain::code::ReviewSideBytes;
    // Given / When / Then
    let before = ReviewSideBytes::Present(b"before".to_vec());
    let after = ReviewSideBytes::Present(b"after".to_vec());
    assert_eq!(
        ReviewPresentation::initial_file_mode(&before, &before),
        super::FileMode::File
    );
    assert_eq!(
        ReviewPresentation::initial_file_mode(&before, &after),
        super::FileMode::Diff
    );
    assert_eq!(
        ReviewPresentation::initial_file_mode(
            &ReviewSideBytes::Missing,
            &ReviewSideBytes::Present(Vec::new())
        ),
        super::FileMode::Diff
    );
}

#[test]
fn test_表示選択_既知の値を型に変換し未知の値を拒否する() {
    // Given / When / Then
    for mode in [super::FileMode::File, super::FileMode::Diff] {
        assert_eq!(super::FileMode::parse(mode.as_str()).unwrap(), mode);
    }
    assert!(super::FileMode::parse("unknown").is_err());
}

#[test]
fn test_属性バイナリの初期表示_比較の変更有無を表示選択にする() {
    // Given / When / Then
    assert_eq!(super::FileMode::for_change(false), super::FileMode::File);
    assert_eq!(super::FileMode::for_change(true), super::FileMode::Diff);
}
