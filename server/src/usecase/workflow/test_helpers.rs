use crate::domain::workflow::{ExecutionTreeArchiveRepository, WorkflowError};
pub struct NoopArchiveRepository;

#[async_trait::async_trait]
impl ExecutionTreeArchiveRepository for NoopArchiveRepository {
    async fn location(
        &self,
        id: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveCandidate, WorkflowError> {
        Ok(crate::domain::workflow::ExecutionTreeArchiveCandidate {
            execution_id: id.into(),
            worktree_path: "/tmp/wt".into(),
            workspace_identity: "/tmp/wt".into(),
            repository_root: None,
        })
    }
    fn worktree_identity(&self, path: &str) -> Result<String, WorkflowError> {
        Ok(crate::domain::git::normalize_repo_path(path))
    }
    async fn record_repository_root(&self, _: &str, _: &str, _: f64) -> Result<(), WorkflowError> {
        unreachable!()
    }
    async fn candidate_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }

    async fn worktree_target_page(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>, WorkflowError> {
        unreachable!()
    }
    async fn target(
        &self,
        _: &str,
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveTarget, WorkflowError> {
        unreachable!()
    }

    async fn archive(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _archived_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn restore(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }
}

#[cfg(test)]
use crate::domain::workflow::ExecutionTreeId;
#[cfg(test)]
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowEventRepository};
#[cfg(test)]
use crate::usecase::workflow::*;
use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(test)]
#[derive(Default)]
pub struct FakeFacetRepository {
    pub facets: Mutex<HashMap<(FacetKind, String), String>>,
    pub builtin_keys: Mutex<std::collections::HashSet<(FacetKind, String)>>,
}

#[cfg(test)]
impl FakeFacetRepository {
    pub fn get_saved(&self, kind: FacetKind, key: &str) -> Option<String> {
        self.facets
            .lock()
            .unwrap()
            .get(&(kind, key.to_string()))
            .cloned()
    }
}

#[cfg(test)]
impl FacetRepository for FakeFacetRepository {
    fn get(
        &self,
        kind: FacetKind,
        key: &str,
    ) -> Result<crate::domain::workflow::Facet, WorkflowError> {
        let content = self
            .get_saved(kind, key)
            .ok_or_else(|| WorkflowError::NotFound(key.into()))?;
        if self
            .builtin_keys
            .lock()
            .unwrap()
            .contains(&(kind, key.into()))
        {
            return Ok(crate::domain::workflow::Facet::restore(
                kind,
                crate::domain::workflow::FacetKey::new(key)?,
                "/workflows".into(),
                crate::domain::workflow::WorkflowSource::Builtin,
                content,
            ));
        }
        Ok(facet(kind, key, content))
    }
}

#[cfg(test)]
pub(crate) fn facet(kind: FacetKind, key: &str, content: String) -> crate::domain::workflow::Facet {
    use crate::domain::workflow::{Facet, FacetKey, WorkflowSource};
    Facet::restore(
        kind,
        FacetKey::new(key).unwrap(),
        "/workflows".into(),
        WorkflowSource::File {
            path: std::path::PathBuf::from("/workflows")
                .join(kind.dir_name())
                .join(format!("{key}.md")),
        },
        content,
    )
}

#[cfg(test)]
#[derive(Default)]
pub struct FakeEventRepository {
    pub events: Mutex<Vec<WorkflowEventDraft>>,
    pub reads: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl FakeEventRepository {
    #[cfg(test)]
    pub(crate) fn seed(&self, event: WorkflowEventDraft) {
        self.events.lock().unwrap().push(event);
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl WorkflowEventRepository for FakeEventRepository {
    #[cfg(test)]
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    async fn read(
        &self,
        _execution_id: &ExecutionTreeId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.events.lock().unwrap().clone())
    }
}

#[cfg(test)]
pub(crate) mod startup {
    use crate::domain::workflow::entities::workflow_execution::ExecutionTree;
    use crate::domain::workflow::repository::WorkflowStartupRecord;
    use crate::domain::workflow::repository::WorkflowStartupRepository;
    use crate::domain::workflow::WorkflowError;
    use crate::domain::workflow::{ExecutionTreeLaunch, NodeFact, TreeRootFact};
    use crate::usecase::workflow::startup::*;
    use std::sync::Mutex;

    pub(crate) struct Repository {
        pub(crate) terminal: Mutex<Option<NodeFact>>,
        pub(crate) unreadable: bool,
        pub(crate) fail_load: bool,
    }

    #[async_trait::async_trait]
    impl WorkflowStartupRepository for Repository {
        async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
            Ok(vec!["tree".into()])
        }

        async fn load(
            &self,
            tree_id: &str,
        ) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
            if self.fail_load {
                return Err(WorkflowError::external("read failed"));
            }
            let root = TreeRootFact {
                repository_root: None,
                workspace_identity: "/repo".into(),
                worktree_path: "/repo".into(),
                request: String::new(),
                workflow_name: "old".into(),
                definition: None,
                launched_as: ExecutionTreeLaunch::Workflow,
            };
            let mut execution = ExecutionTree::restore_without_definition(tree_id, &root, 1.0);
            if let Some(fact) = self.terminal.lock().unwrap().as_ref() {
                execution.replay_terminal_fact(fact, 2.0);
            }
            Ok(Some(WorkflowStartupRecord {
                execution,
                definition_error: self
                    .unreadable
                    .then(|| "Workflow definition is unavailable: completion".into()),
            }))
        }
    }

    pub(crate) fn repository() -> Repository {
        Repository {
            terminal: Mutex::new(None),
            unreadable: true,
            fail_load: false,
        }
    }

    pub(crate) struct Startup {
        pub(crate) calls: Mutex<Vec<String>>,
        pub(crate) failure: Option<&'static str>,
        pub(crate) conflict: bool,
        pub(crate) temporary: bool,
    }

    impl Startup {
        pub(crate) fn record(&self, call: String) -> Result<(), WorkflowError> {
            let first = !self.calls.lock().unwrap().contains(&call);
            self.calls.lock().unwrap().push(call.clone());
            if self.failure == Some(call.as_str()) && (!(self.conflict || self.temporary) || first)
            {
                if self.temporary {
                    Err(WorkflowError::Store(
                        crate::domain::failure::StorageFailure::from(
                            crate::domain::local_event::CommitBatchError::QueueBusy,
                        )
                        .with_message(call),
                    ))
                } else if self.conflict {
                    Err(WorkflowError::Conflict(call))
                } else {
                    Err(WorkflowError::external(call))
                }
            } else {
                Ok(())
            }
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStartupRepository for Startup {
        async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
            self.record("list".into())?;
            Ok(["first", "second"].map(String::from).into())
        }

        async fn load(
            &self,
            tree_id: &str,
        ) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
            self.record(format!("load:{tree_id}"))?;
            let mut repository = repository();
            repository.unreadable = false;
            repository.load(tree_id).await
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStartupGateway for Startup {
        fn current_timestamp(&self) -> f64 {
            3.0
        }

        async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError> {
            assert_eq!(timestamp, 3.0);
            tokio::task::yield_now().await;
            self.record(format!("reconcile:{tree_id}"))
        }
    }

    pub(crate) struct ConflictingStartup {
        pub(crate) conflicts: usize,
        pub(crate) calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl WorkflowStartupGateway for ConflictingStartup {
        fn current_timestamp(&self) -> f64 {
            3.0
        }

        async fn reconcile_tree(
            &self,
            _tree_id: &str,
            _timestamp: f64,
        ) -> Result<(), WorkflowError> {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < self.conflicts {
                Err(WorkflowError::Conflict("head advanced".into()))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct FailedStartup(pub(crate) std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl WorkflowStartupGateway for FailedStartup {
        fn current_timestamp(&self) -> f64 {
            3.0
        }

        async fn reconcile_tree(&self, _: &str, _: f64) -> Result<(), WorkflowError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(WorkflowError::external("advancement failed"))
        }
    }

    pub(crate) struct FailingStartupList(pub(crate) WorkflowError);

    #[async_trait::async_trait]
    impl WorkflowStartupRepository for FailingStartupList {
        async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
            Err(self.0.clone())
        }

        async fn load(&self, _: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
            panic!("failed enumeration must not load a tree")
        }
    }
}

use crate::domain::workflow::{WorkflowDefinition, WorkflowDefinitionRepository};
#[derive(Default)]
pub struct FakeDefinitionRepository {
    pub definitions: Mutex<HashMap<String, WorkflowDefinition>>,
    pub read_error: Mutex<Option<String>>,
}

impl FakeDefinitionRepository {
    pub fn insert(&self, name: &str, definition: WorkflowDefinition) {
        self.definitions
            .lock()
            .unwrap()
            .insert(name.to_owned(), definition);
    }
}

impl WorkflowDefinitionRepository for FakeDefinitionRepository {
    fn get(&self, name: &str) -> Result<WorkflowDefinition, WorkflowError> {
        if let Some(message) = self.read_error.lock().unwrap().as_ref() {
            return Err(WorkflowError::external(message));
        }
        self.definitions
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| WorkflowError::NotFound(name.into()))
    }
    fn find(&self, name: Option<&str>) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        match name.map(|name| self.get(name)) {
            None | Some(Err(WorkflowError::NotFound(_))) => Ok(None),
            Some(result) => result.map(Some),
        }
    }
}

#[cfg(test)]
impl FakeDefinitionRepository {
    pub fn seed(&self, name: &str, definition: crate::domain::workflow::WorkflowDefinition) {
        self.insert(name, definition);
    }
    pub fn get_saved(&self, name: &str) -> Option<crate::domain::workflow::WorkflowDefinition> {
        self.definitions.lock().unwrap().get(name).cloned()
    }
}

#[cfg(test)]
pub(crate) fn definition(
    content: crate::domain::workflow::WorkflowDefinitionContent,
) -> WorkflowDefinition {
    WorkflowDefinition::restore(
        "/workflows".into(),
        crate::domain::workflow::WorkflowSource::File {
            path: format!("/workflows/{}.yml", content.name).into(),
        },
        crate::domain::workflow::WorkflowSourceFormat::Yaml,
        Ok(content),
    )
}
