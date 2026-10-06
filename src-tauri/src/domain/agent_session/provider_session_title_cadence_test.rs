mod provider_session_title_cadence_tests {
    use super::super::should_read_provider_session_title;

    #[test]
    fn test_provider_session_title_cadence_タイトル未取得なら毎tick読む() {
        for tick in 0..=30 {
            assert!(should_read_provider_session_title(tick, false));
        }
    }

    #[test]
    fn test_provider_session_title_cadence_タイトル取得済みなら15tickごとに読む() {
        for tick in 0..=30 {
            assert_eq!(
                should_read_provider_session_title(tick, true),
                matches!(tick, 0 | 15 | 30)
            );
        }
    }
}
