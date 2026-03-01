const DEFAULT_REVIEW_PROMPT: &str = include_str!("../resources/prompts/review.txt");

#[tauri::command]
pub fn get_review_prompt() -> String {
    DEFAULT_REVIEW_PROMPT.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_prompt_is_not_empty() {
        let prompt = get_review_prompt();
        assert!(!prompt.is_empty());
        assert!(prompt.contains("You are a code reviewer"));
    }
}
