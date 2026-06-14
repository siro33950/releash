#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDictationPresentationRequest {
    pub supported: bool,
    pub listening: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentDictationPresentation {
    pub enabled: bool,
    pub label: String,
    pub title: String,
    pub status: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDictationDraftRequest {
    pub base_value: String,
    pub transcript: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentDictationDraft {
    pub value: String,
    pub caret: usize,
}

pub(crate) fn present_agent_dictation_inner(
    request: &AgentDictationPresentationRequest,
) -> AgentDictationPresentation {
    if !request.supported {
        return AgentDictationPresentation {
            enabled: false,
            label: "Dictation unavailable".to_string(),
            title: "Voice dictation is unavailable in this WebView".to_string(),
            status: request.error.clone(),
        };
    }

    if request.listening {
        return AgentDictationPresentation {
            enabled: true,
            label: "Stop dictation".to_string(),
            title: "Stop voice dictation".to_string(),
            status: Some("Listening".to_string()),
        };
    }

    AgentDictationPresentation {
        enabled: true,
        label: "Start dictation".to_string(),
        title: "Start voice dictation".to_string(),
        status: request.error.clone(),
    }
}

pub(crate) fn compose_agent_dictation_draft_inner(
    request: &AgentDictationDraftRequest,
) -> AgentDictationDraft {
    let transcript = request.transcript.trim();
    if transcript.is_empty() {
        return AgentDictationDraft {
            value: request.base_value.clone(),
            caret: request.base_value.chars().count(),
        };
    }

    let mut value = request.base_value.trim_end().to_string();
    if !value.is_empty() {
        value.push(' ');
    }
    value.push_str(transcript);
    let caret = value.chars().count();
    AgentDictationDraft { value, caret }
}

#[tauri::command]
pub fn present_agent_dictation(
    request: AgentDictationPresentationRequest,
) -> AgentDictationPresentation {
    present_agent_dictation_inner(&request)
}

#[tauri::command]
pub fn compose_agent_dictation_draft(request: AgentDictationDraftRequest) -> AgentDictationDraft {
    compose_agent_dictation_draft_inner(&request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presents_unavailable_dictation() {
        let result = present_agent_dictation_inner(&AgentDictationPresentationRequest {
            supported: false,
            listening: false,
            error: Some("missing speech api".to_string()),
        });

        assert!(!result.enabled);
        assert_eq!(result.status.as_deref(), Some("missing speech api"));
    }

    #[test]
    fn presents_listening_dictation() {
        let result = present_agent_dictation_inner(&AgentDictationPresentationRequest {
            supported: true,
            listening: true,
            error: None,
        });

        assert!(result.enabled);
        assert_eq!(result.status.as_deref(), Some("Listening"));
    }

    #[test]
    fn appends_dictation_transcript_with_spacing() {
        let result = compose_agent_dictation_draft_inner(&AgentDictationDraftRequest {
            base_value: "Review this".to_string(),
            transcript: "  and write tests  ".to_string(),
        });

        assert_eq!(result.value, "Review this and write tests");
        assert_eq!(result.caret, result.value.chars().count());
    }

    #[test]
    fn leaves_empty_transcript_unchanged() {
        let result = compose_agent_dictation_draft_inner(&AgentDictationDraftRequest {
            base_value: "Keep draft".to_string(),
            transcript: " ".to_string(),
        });

        assert_eq!(result.value, "Keep draft");
        assert_eq!(result.caret, "Keep draft".chars().count());
    }

    #[test]
    fn returns_caret_as_unicode_character_position() {
        let result = compose_agent_dictation_draft_inner(&AgentDictationDraftRequest {
            base_value: "確認".to_string(),
            transcript: "  追加  ".to_string(),
        });

        assert_eq!(result.value, "確認 追加");
        assert_eq!(result.caret, 5);
    }
}
