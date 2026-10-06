mod provider_history_label_tests {
    use super::super::provider_history_label;
    use crate::domain::provider_lifecycle::ProviderKind;

    #[test]
    fn test_provider_history_label_前後空白を落としたproviderタイトルを優先する() {
        assert_eq!(
            provider_history_label(
                ProviderKind::Claude,
                "4f3a9b21-1234",
                Some("  Release review  "),
                Some("Must not be shown"),
            ),
            "Release review"
        );
    }

    #[test]
    fn test_provider_history_label_空タイトルなら最初のユーザープロンプトを表示する() {
        assert_eq!(
            provider_history_label(
                ProviderKind::Codex,
                "abcdef123456",
                Some(" \t "),
                Some("  Fix the release workflow  "),
            ),
            "Fix the release workflow"
        );
    }

    #[test]
    fn test_provider_history_label_空文字なら各段から次の段へ落とす() {
        assert_eq!(
            provider_history_label(
                ProviderKind::Codex,
                "abcdef123456",
                Some(""),
                Some("Fix the release workflow"),
            ),
            "Fix the release workflow"
        );
        assert_eq!(
            provider_history_label(ProviderKind::Claude, "4f3a9b21-1234", None, Some(""),),
            "Claude 4f3a9b21…"
        );
    }

    #[test]
    fn test_provider_history_label_空タイトルと空プロンプトならprovider名と短縮idへ落とす() {
        assert_eq!(
            provider_history_label(
                ProviderKind::Claude,
                "4f3a9b21-1234",
                Some(" \t "),
                Some("\n \t"),
            ),
            "Claude 4f3a9b21…"
        );
        assert_eq!(
            provider_history_label(ProviderKind::Codex, "abcdef123456", None, None),
            "Codex abcdef12…"
        );
    }

    #[test]
    fn test_provider_history_label_長い複数行プロンプトを一行化して切り詰める() {
        let prompt = format!("  first line\n\n  second\tline {}  ", "x".repeat(100));

        let label =
            provider_history_label(ProviderKind::Claude, "4f3a9b21-1234", None, Some(&prompt));

        assert_eq!(label.chars().count(), 80);
        assert_eq!(label, format!("first line second line {}…", "x".repeat(56)));
        assert!(!label.contains('\n'));
    }
}
