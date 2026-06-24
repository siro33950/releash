use std::sync::Arc;

use crate::domain::agent_session::SkillEntry;

const CODEX_BACKEND_ID: &str = "codex";

#[async_trait::async_trait]
pub(crate) trait CodexSkillCatalogGateway: Send + Sync {
    async fn list_app_server_skills(
        &self,
        cwd: &str,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String>;

    async fn scan_local_skills(
        &self,
        cwd: &str,
        backend_id: Option<&str>,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String>;
}

#[derive(Clone)]
pub(crate) struct AgentSessionUsecase {
    skill_catalog: Arc<dyn CodexSkillCatalogGateway>,
}

impl AgentSessionUsecase {
    pub(crate) fn new(skill_catalog: Arc<dyn CodexSkillCatalogGateway>) -> Self {
        Self { skill_catalog }
    }

    pub(crate) async fn scan_agent_skills(
        &self,
        cwd: String,
        backend_id: Option<String>,
        query: Option<String>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String> {
        self.skill_catalog
            .scan_local_skills(&cwd, backend_id.as_deref(), query.as_deref(), limit)
            .await
    }

    pub(crate) async fn read_codex_skill_catalog(
        &self,
        cwd: String,
        query: Option<String>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String> {
        match self
            .skill_catalog
            .list_app_server_skills(&cwd, query.as_deref(), limit)
            .await
        {
            Ok(skills) => Ok(skills),
            Err(_) => {
                self.skill_catalog
                    .scan_local_skills(&cwd, Some(CODEX_BACKEND_ID), query.as_deref(), limit)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeSkillCatalogGateway {
        app_server_result: Mutex<Option<Result<Vec<SkillEntry>, String>>>,
        scan_result: Mutex<Result<Vec<SkillEntry>, String>>,
        scan_backend_ids: Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl CodexSkillCatalogGateway for FakeSkillCatalogGateway {
        async fn list_app_server_skills(
            &self,
            _cwd: &str,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, String> {
            self.app_server_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(Vec::new()))
        }

        async fn scan_local_skills(
            &self,
            _cwd: &str,
            backend_id: Option<&str>,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, String> {
            self.scan_backend_ids
                .lock()
                .unwrap()
                .push(backend_id.map(ToString::to_string));
            self.scan_result.lock().unwrap().clone()
        }
    }

    fn skill(name: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            description: format!("{name} description"),
            scope: "project".to_string(),
        }
    }

    #[tokio::test]
    async fn read_codex_skill_catalog_returns_app_server_skills() {
        let gateway = Arc::new(FakeSkillCatalogGateway {
            app_server_result: Mutex::new(Some(Ok(vec![skill("review")]))),
            scan_result: Mutex::new(Ok(vec![skill("fallback")])),
            scan_backend_ids: Mutex::new(Vec::new()),
        });
        let usecase = AgentSessionUsecase::new(gateway.clone());

        let result = usecase
            .read_codex_skill_catalog("/repo".to_string(), None, None)
            .await
            .unwrap();

        assert_eq!(result, vec![skill("review")]);
        assert!(gateway.scan_backend_ids.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn read_codex_skill_catalog_falls_back_to_local_codex_scan() {
        let gateway = Arc::new(FakeSkillCatalogGateway {
            app_server_result: Mutex::new(Some(Err("app-server unavailable".to_string()))),
            scan_result: Mutex::new(Ok(vec![skill("fallback")])),
            scan_backend_ids: Mutex::new(Vec::new()),
        });
        let usecase = AgentSessionUsecase::new(gateway.clone());

        let result = usecase
            .read_codex_skill_catalog("/repo".to_string(), Some("fall".to_string()), Some(10))
            .await
            .unwrap();

        assert_eq!(result, vec![skill("fallback")]);
        assert_eq!(
            gateway.scan_backend_ids.lock().unwrap().as_slice(),
            &[Some(CODEX_BACKEND_ID.to_string())]
        );
    }
}
