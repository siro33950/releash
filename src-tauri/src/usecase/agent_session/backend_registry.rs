use std::sync::Arc;

use serde::Serialize;

use crate::domain::agent_session::gateway::AgentBackend;
use crate::domain::agent_session::value_objects::{ModelDescriptor, ModelId};
use crate::usecase::agent_session::session::{ModelInfo, SessionBackendResolver};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub available_models: Vec<ModelInfo>,
    pub capabilities: BackendCapabilitiesInfo,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackendCapabilitiesInfo {
    pub steering: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendListResult {
    pub backends: Vec<BackendInfo>,
    pub default_id: Option<String>,
}

pub struct AgentBackendRegistry {
    backends: Vec<RegisteredBackend>,
    default_id: Option<String>,
}

struct RegisteredBackend {
    id: String,
    backend: Arc<dyn AgentBackend>,
    available: bool,
}

impl AgentBackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
            default_id: None,
        }
    }

    pub fn register(&mut self, backend: Arc<dyn AgentBackend>) {
        let id = backend.id().to_string();
        if self.backends.iter().any(|entry| entry.id == id) {
            return;
        }
        self.backends.push(RegisteredBackend {
            id,
            backend,
            available: true,
        });
    }

    #[cfg(test)]
    #[allow(dead_code)] // issues-1301 G-3: retained for backend availability scenario tests.
    pub fn set_available(&mut self, id: &str, available: bool) {
        if let Some(entry) = self.backends.iter_mut().find(|entry| entry.id == id) {
            entry.available = available;
        }
    }

    pub fn set_default(&mut self, id: Option<String>) {
        self.default_id = id;
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn AgentBackend>> {
        self.backends
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| Arc::clone(&entry.backend))
    }

    pub fn backend_for_optional_id(
        &self,
        backend_id: Option<&str>,
    ) -> Result<Arc<dyn AgentBackend>, String> {
        let id = match backend_id {
            Some(id) => id.to_string(),
            None => self.resolve_default_id()?,
        };
        self.get(&id)
            .ok_or_else(|| format!("バックエンド '{id}' がレジストリに登録されていません"))
    }

    pub fn list(&self) -> Vec<BackendInfo> {
        self.backends
            .iter()
            .map(|entry| BackendInfo {
                id: entry.id.clone(),
                name: entry.backend.name().to_string(),
                available: entry.available,
                available_models: self.available_models(&entry.id).unwrap_or_default(),
                capabilities: BackendCapabilitiesInfo {
                    steering: entry.backend.capabilities().steering,
                },
            })
            .collect()
    }

    pub fn list_result(&self) -> BackendListResult {
        BackendListResult {
            backends: self.list(),
            default_id: self.resolve_default_id().ok(),
        }
    }

    pub fn available_models(&self, backend_id: &str) -> Result<Vec<ModelInfo>, String> {
        let backend = self.get(backend_id).ok_or_else(|| {
            format!("バックエンド '{backend_id}' がレジストリに登録されていません")
        })?;
        Ok(backend
            .available_models()
            .into_iter()
            .map(|model| model_info(backend_id, model))
            .collect())
    }

    pub fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        self.available_models(backend_id)?
            .into_iter()
            .next()
            .map(|model| model.model_id)
            .ok_or_else(|| {
                format!("バックエンド '{backend_id}' に既定モデルがありません（モデル一覧が空）")
            })
    }

    pub fn resolve_model_entry(&self, entry_id: &str) -> Result<ModelInfo, String> {
        if let Some((backend_id, model_id)) = entry_id.split_once(':') {
            let parsed = ModelId::parse(model_id)?;
            return self.model_for_backend(backend_id, parsed.as_str());
        }
        let parsed = ModelId::parse(entry_id)?;
        let mut matches = Vec::new();
        for entry in self.backends.iter().filter(|entry| entry.available) {
            if entry
                .backend
                .available_models()
                .iter()
                .any(|model| model.id.as_str() == parsed.as_str())
            {
                matches.push(entry.id.clone());
            }
        }
        match matches.as_slice() {
            [] => Err(format!(
                "モデル '{entry_id}' はどのバックエンドにも登録されていません"
            )),
            [backend_id] => self.model_for_backend(backend_id, parsed.as_str()),
            _ => Err(format!(
                "モデル '{entry_id}' が複数のバックエンドに登録されているため一意特定できません: {}",
                matches.join(", ")
            )),
        }
    }

    fn model_for_backend(&self, backend_id: &str, model_id: &str) -> Result<ModelInfo, String> {
        let backend = self.get(backend_id).ok_or_else(|| {
            format!("バックエンド '{backend_id}' がレジストリに登録されていません")
        })?;
        let model = backend
            .available_models()
            .into_iter()
            .find(|model| model.id.as_str() == model_id)
            .ok_or_else(|| {
                format!("モデル '{model_id}' はバックエンド '{backend_id}' に登録されていません")
            })?;
        Ok(model_info(backend_id, model))
    }

    pub fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        match backend_id {
            Some(id) => {
                if self.get(&id).is_none() {
                    return Err(format!(
                        "バックエンド '{}' がレジストリに登録されていません",
                        id
                    ));
                }
                Ok(id)
            }
            None => self.resolve_default_id(),
        }
    }

    pub fn resolve_default_id(&self) -> Result<String, String> {
        if let Some(default_id) = &self.default_id {
            if self
                .backends
                .iter()
                .any(|entry| &entry.id == default_id && entry.available)
            {
                return Ok(default_id.clone());
            }
            return Err(format!(
                "デフォルトバックエンド '{}' がレジストリに登録されていないか利用不可です",
                default_id
            ));
        }
        self.backends
            .iter()
            .find(|entry| entry.available)
            .map(|entry| entry.id.clone())
            .ok_or_else(|| "利用可能なバックエンドが登録されていません".to_string())
    }
}

impl SessionBackendResolver for AgentBackendRegistry {
    #[cfg(test)]
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        AgentBackendRegistry::resolve_backend_id(self, backend_id)
    }

    fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        AgentBackendRegistry::default_model_for(self, backend_id)
    }

    fn backend_exists(&self, backend_id: &str) -> bool {
        self.get(backend_id).is_some()
    }

    #[cfg(test)]
    fn resolve_default_id(&self) -> Result<String, String> {
        AgentBackendRegistry::resolve_default_id(self)
    }
}

fn model_info(backend_id: &str, model: ModelDescriptor) -> ModelInfo {
    let model_id = model.id.as_str().to_string();
    ModelInfo {
        id: format!("{backend_id}:{model_id}"),
        display_name: model.display_name,
        backend: backend_id.to_string(),
        model_id,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use async_trait::async_trait;

    use crate::domain::agent_session::gateway::{
        AgentBackendError, AgentSessionRuntime, ForkSessionRequest, SessionSpec,
    };
    use crate::domain::agent_session::value_objects::{BackendCapabilities, SkillEntry};

    use super::*;

    struct MockBackend {
        id: &'static str,
        name: &'static str,
        models: Vec<&'static str>,
    }

    #[async_trait]
    impl AgentBackend for MockBackend {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.name
        }

        fn available_models(&self) -> Vec<ModelDescriptor> {
            self.models
                .iter()
                .map(|model| ModelDescriptor {
                    id: ModelId::parse(*model).unwrap(),
                    display_name: format!("display {model}"),
                })
                .collect()
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities { steering: false }
        }

        async fn open_session(
            &self,
            _spec: SessionSpec,
        ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
            Err(AgentBackendError::Unavailable("test".to_string()))
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
            _cwd: &Path,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, AgentBackendError> {
            Ok(Vec::new())
        }

        async fn fuzzy_file_search(
            &self,
            _root: &Path,
            _query: &str,
            _limit: usize,
        ) -> Result<Option<Vec<String>>, AgentBackendError> {
            Ok(None)
        }
    }

    fn backend(id: &'static str, models: Vec<&'static str>) -> Arc<dyn AgentBackend> {
        Arc::new(MockBackend {
            id,
            name: id,
            models,
        })
    }

    #[test]
    fn test_registry_uses_backend_owned_models() {
        let mut registry = AgentBackendRegistry::new();
        registry.register(backend("claude", vec!["sonnet", "opus"]));

        let models = registry.available_models("claude").unwrap();

        assert_eq!(models[0].id, "claude:sonnet");
        assert_eq!(registry.default_model_for("claude").unwrap(), "sonnet");
    }

    #[test]
    fn test_registry_resolves_bare_model_only_when_unique() {
        let mut registry = AgentBackendRegistry::new();
        registry.register(backend("claude", vec!["shared"]));
        registry.register(backend("codex", vec!["shared"]));

        assert!(registry.resolve_model_entry("shared").is_err());
        assert_eq!(
            registry
                .resolve_model_entry("codex:shared")
                .unwrap()
                .backend,
            "codex"
        );
    }

    #[test]
    fn test_registry_default_uses_configured_available_backend() {
        let mut registry = AgentBackendRegistry::new();
        registry.register(backend("claude", vec!["sonnet"]));
        registry.register(backend("codex", vec!["gpt"]));
        registry.set_default(Some("codex".to_string()));

        assert_eq!(registry.resolve_default_id().unwrap(), "codex");
    }
}
