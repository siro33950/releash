use crate::domain::provider_lifecycle::ProviderKind;

const MAX_FIRST_USER_PROMPT_LABEL_CHARS: usize = 80;

pub(crate) fn provider_history_label(
    provider: ProviderKind,
    provider_session_id: &str,
    session_title: Option<&str>,
    first_user_prompt: Option<&str>,
) -> String {
    if let Some(title) = session_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        return title.to_string();
    }
    if let Some(prompt) = first_user_prompt
        .map(one_line_prompt)
        .filter(|prompt| !prompt.is_empty())
    {
        return truncate_prompt(prompt);
    }
    let provider = match provider {
        ProviderKind::Claude => "Claude",
        ProviderKind::Codex => "Codex",
    };
    let short_id = provider_session_id.chars().take(8).collect::<String>();
    format!("{provider} {short_id}…")
}

fn one_line_prompt(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_prompt(prompt: String) -> String {
    if prompt.chars().count() <= MAX_FIRST_USER_PROMPT_LABEL_CHARS {
        return prompt;
    }
    let mut truncated = prompt
        .chars()
        .take(MAX_FIRST_USER_PROMPT_LABEL_CHARS - 1)
        .collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::provider_history_label;
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
