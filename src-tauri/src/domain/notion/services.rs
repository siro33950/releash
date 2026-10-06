#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Spec #1302 keeps Notion branch derivation as a Rust-owned rule without production wiring."
    )
)]
pub(crate) fn notion_branch_name(
    branch_name_property: &str,
    page_id: Option<&str>,
    prefix: Option<&str>,
) -> String {
    let sanitized = sanitize_branch_name_property(branch_name_property);
    if !sanitized.is_empty() {
        return with_prefix(sanitized, prefix);
    }

    if let Some(page_id) = page_id.filter(|value| !value.is_empty()) {
        let short_id: String = page_id.chars().filter(|ch| *ch != '-').take(8).collect();
        return with_prefix(format!("notion/{short_id}"), prefix);
    }

    with_prefix("notion-task".to_string(), prefix)
}

pub(crate) fn notion_task_title_branch_name(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_separator = false;

    for ch in title.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('-');
            previous_was_separator = true;
        }
    }

    let slug = slug.trim_matches('-').chars().take(40).collect::<String>();
    format!("feat/{slug}")
}

fn sanitize_branch_name_property(value: &str) -> String {
    let whitespace_collapsed = collapse_whitespace_to_dash(value.trim());
    let allowed_only: String = whitespace_collapsed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-'))
        .collect();
    let dash_collapsed = collapse_repeated_dash(&allowed_only);
    dash_collapsed
        .trim_matches(|ch| matches!(ch, '-' | '/'))
        .to_string()
}

fn collapse_whitespace_to_dash(value: &str) -> String {
    let mut result = String::new();
    let mut previous_was_whitespace = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !previous_was_whitespace {
                result.push('-');
            }
            previous_was_whitespace = true;
        } else {
            result.push(ch);
            previous_was_whitespace = false;
        }
    }
    result
}

fn collapse_repeated_dash(value: &str) -> String {
    let mut result = String::new();
    let mut previous_was_dash = false;
    for ch in value.chars() {
        if ch == '-' {
            if !previous_was_dash {
                result.push(ch);
            }
            previous_was_dash = true;
        } else {
            result.push(ch);
            previous_was_dash = false;
        }
    }
    result
}

fn with_prefix(value: String, prefix: Option<&str>) -> String {
    match prefix.filter(|prefix| !prefix.is_empty()) {
        Some(prefix) if !value.starts_with(prefix) => format!("{prefix}{value}"),
        _ => value,
    }
}

#[cfg(test)]
#[path = "services_test.rs"]
pub(crate) mod services_tests;
