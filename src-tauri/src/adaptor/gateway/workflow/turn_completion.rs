use crate::adaptor::gateway::workflow::domain_mapping::transition_rule_to_domain;
use crate::adaptor::gateway::workflow::schema::TransitionRule;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::usecase::agent_session::session::MessagePart;

/// Evaluates legacy workflow transition rules through the domain transition service.
pub(crate) fn evaluate_auto_rules(
    text: &str,
    rules: &[TransitionRule],
) -> Option<(String, String)> {
    let rules: Vec<_> = rules.iter().map(transition_rule_to_domain).collect();
    workflow_transition::evaluate_auto_rules(text, &rules)
}

/// Extracts user-visible text from agent message parts for workflow turn completion.
pub(crate) fn extract_text_from_parts(parts: &[MessagePart]) -> String {
    let mut text = String::new();
    for part in parts {
        if let MessagePart::Text { content, .. } = part {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(content);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_auto_rules_matches_first_rule() {
        let rules = vec![
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        let result = evaluate_auto_rules("<decision>NEEDS_FIX</decision>", &rules);
        assert_eq!(
            result,
            Some(("implement".to_string(), "NEEDS_FIX".to_string()))
        );
    }

    #[test]
    fn evaluate_auto_rules_matches_second_rule() {
        let rules = vec![
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        assert_eq!(
            evaluate_auto_rules("<decision>LGTM</decision>", &rules),
            Some(("report".to_string(), "LGTM".to_string()))
        );
    }

    #[test]
    fn evaluate_auto_rules_no_match_returns_none() {
        let rules = vec![
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        assert_eq!(
            evaluate_auto_rules("The code looks okay but needs minor refactoring", &rules),
            None
        );
    }

    #[test]
    fn evaluate_auto_rules_first_match_wins() {
        let rules = vec![
            TransitionRule {
                r#match: "FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "review".to_string(),
            },
        ];

        assert_eq!(
            evaluate_auto_rules("<decision>NEEDS_FIX</decision>", &rules),
            Some(("implement".to_string(), "FIX".to_string()))
        );
    }

    #[test]
    fn evaluate_auto_rules_regex_pattern() {
        let rules = vec![TransitionRule {
            r#match: r"<decision>(LGTM|APPROVED)</decision>".to_string(),
            next: "report".to_string(),
        }];

        assert_eq!(
            evaluate_auto_rules("Review complete. <decision>APPROVED</decision>", &rules),
            Some((
                "report".to_string(),
                r"<decision>(LGTM|APPROVED)</decision>".to_string()
            ))
        );
    }

    #[test]
    fn evaluate_auto_rules_skips_invalid_regex_rules() {
        let rules = vec![
            TransitionRule {
                r#match: "[invalid".to_string(),
                next: "bad".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        assert_eq!(
            evaluate_auto_rules("LGTM", &rules),
            Some(("report".to_string(), "LGTM".to_string()))
        );
    }

    #[test]
    fn evaluate_auto_rules_empty_rules_returns_none() {
        let rules: Vec<TransitionRule> = vec![];

        assert_eq!(evaluate_auto_rules("any text", &rules), None);
    }

    #[test]
    fn evaluate_auto_rules_all_invalid_regex_returns_none() {
        let rules = vec![TransitionRule {
            r#match: "[invalid".to_string(),
            next: "bad".to_string(),
        }];

        assert_eq!(evaluate_auto_rules("anything", &rules), None);
    }

    #[test]
    fn extract_text_from_parts_combines_text_parts() {
        let parts = vec![
            MessagePart::Thinking {
                content: "thinking...".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "First line".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "Second line".to_string(),
                parent_tool_use_id: None,
            },
        ];

        assert_eq!(extract_text_from_parts(&parts), "First line\nSecond line");
    }

    #[test]
    fn extract_text_from_parts_empty() {
        let parts: Vec<MessagePart> = vec![];

        assert_eq!(extract_text_from_parts(&parts), "");
    }
}
