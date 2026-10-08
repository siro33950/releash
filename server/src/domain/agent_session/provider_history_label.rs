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
#[path = "provider_history_label_test.rs"]
mod provider_history_label_tests;
