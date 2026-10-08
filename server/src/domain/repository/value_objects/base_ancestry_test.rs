mod base_ancestry_tests {
    use super::super::*;

    #[test]
    fn test_merge済み判定_merge_commit経由で取り込まれた場合だけ真になる() {
        for (in_base_history, on_base_first_parent, expected) in [
            (true, false, true),
            (true, true, false),
            (false, false, false),
            (false, true, false),
        ] {
            // Given
            let ancestry = BaseAncestry {
                in_base_history,
                on_base_first_parent,
            };
            // When / Then
            assert_eq!(ancestry.is_merged(), expected);
        }
    }
}
