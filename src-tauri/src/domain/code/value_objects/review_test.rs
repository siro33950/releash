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
