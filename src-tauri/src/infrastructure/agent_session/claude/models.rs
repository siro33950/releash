use crate::domain::agent_session::gateway::{
    AgentBackend, AgentBackendError, AgentSessionRuntime, ForkSessionRequest, SessionSpec,
};
use crate::domain::agent_session::value_objects::{
    BackendCapabilities, ModelDescriptor, ModelId, SkillEntry,
};

pub(crate) const CLAUDE_BACKEND_ID: &str = "claude";

const CLAUDE_FIXED_MODELS: &[(&str, &str)] = &[
    ("claude-opus-5", "Opus 5"),
    ("claude-fable-5", "Fable 5"),
    ("claude-sonnet-5", "Sonnet 5"),
    ("claude-haiku-4-5-20251001", "Haiku 4.5"),
];

#[derive(Debug, Clone)]
pub(crate) struct ClaudeBackend {
    cli_path: String,
}

impl ClaudeBackend {
    pub(crate) fn new(cli_path: Option<String>) -> Self {
        Self {
            cli_path: cli_path.unwrap_or_else(|| "claude".to_string()),
        }
    }

    pub(crate) fn cli_path(&self) -> &str {
        &self.cli_path
    }
}

#[async_trait::async_trait]
impl AgentBackend for ClaudeBackend {
    fn id(&self) -> &str {
        CLAUDE_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Claude"
    }

    fn available_models(&self) -> Vec<ModelDescriptor> {
        CLAUDE_FIXED_MODELS
            .iter()
            .map(|(id, display_name)| ModelDescriptor {
                id: ModelId::parse(*id)
                    .expect("CLAUDE_FIXED_MODELS must contain valid model identifiers"),
                display_name: (*display_name).to_string(),
            })
            .collect()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities { steering: false }
    }

    async fn open_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
        let runtime =
            super::session::ClaudeSessionRuntime::open(self.cli_path().to_string(), spec).await?;
        Ok(Box::new(runtime))
    }

    async fn archive_session(
        &self,
        _backend_session_id: &str,
        _cwd: &str,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn unarchive_session(
        &self,
        _backend_session_id: &str,
        _cwd: &str,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn fork_session(
        &self,
        _req: ForkSessionRequest,
    ) -> Result<Option<String>, AgentBackendError> {
        Ok(None)
    }

    async fn skill_catalog(
        &self,
        cwd: &std::path::Path,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, AgentBackendError> {
        Ok(super::skills::scan_claude_agent_skills(cwd, query, limit))
    }

    async fn fuzzy_file_search(
        &self,
        _root: &std::path::Path,
        _query: &str,
        _limit: usize,
    ) -> Result<Option<Vec<String>>, AgentBackendError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_models_固定順と表示名を返す() {
        let backend = ClaudeBackend::new(None);
        let models = backend.available_models();

        let ids = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        let names = models
            .iter()
            .map(|model| model.display_name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                "claude-opus-5",
                "claude-fable-5",
                "claude-sonnet-5",
                "claude-haiku-4-5-20251001",
            ]
        );
        assert_eq!(names, vec!["Opus 5", "Fable 5", "Sonnet 5", "Haiku 4.5"]);
    }

    #[test]
    fn test_claude_fixed_models_are_valid_model_ids() {
        for (id, _) in CLAUDE_FIXED_MODELS {
            ModelId::parse(*id).unwrap();
        }
    }
}
