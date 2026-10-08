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
        Ok(crate::domain::repository::normalize_repo_path(path))
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
    async fn legacy_session_archive_page(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
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
    fn legacy_archives(
        &self,
    ) -> Result<Vec<crate::domain::workflow::ExecutionTreeArchiveRecord>, WorkflowError> {
        Ok(Vec::new())
    }
    fn finish_legacy_migration(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn archive(
        &self,
        _execution_id: &crate::domain::workflow::ExecutionTreeId,
        _archived_at: f64,
        _reason: &str,
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

    async fn archive_snapshot_for(
        &self,
        _execution_ids: &[String],
    ) -> Result<crate::domain::workflow::ExecutionTreeArchiveSnapshot, WorkflowError> {
        Ok(crate::domain::workflow::ExecutionTreeArchiveSnapshot {
            records: Vec::new(),
        })
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
    fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        Ok(self
            .facets
            .lock()
            .unwrap()
            .keys()
            .filter(|(candidate, _)| *candidate == kind)
            .map(|(_, key)| key.clone())
            .collect())
    }

    fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
        self.facets
            .lock()
            .unwrap()
            .get(&(kind, key.to_string()))
            .cloned()
            .ok_or_else(|| WorkflowError::NotFound(key.to_string()))
    }

    fn save(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        _is_new: bool,
    ) -> Result<(), WorkflowError> {
        self.facets
            .lock()
            .unwrap()
            .insert((kind, key.to_string()), content.to_string());
        Ok(())
    }

    fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.facets.lock().unwrap().remove(&(kind, key.to_string()));
        Ok(())
    }

    fn list_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
        Ok(self
            .list(kind)?
            .into_iter()
            .map(|key| FacetSummary {
                key,
                kind: match kind {
                    FacetKind::Policy => "policy",
                    FacetKind::Knowledge => "knowledge",
                    FacetKind::Instruction => "instruction",
                }
                .into(),
                description: String::new(),
                builtin: false,
            })
            .collect())
    }
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
    use crate::domain::workflow::{ExecutionOrigin, ExecutionTreeLaunch, NodeFact, TreeRootFact};
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
                created_from: ExecutionOrigin::Cli,
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

use crate::domain::workflow::{WorkflowDefinition, WorkflowDefinitionRepository, WorkflowSummary};
#[derive(Default)]
pub struct FakeDefinitionRepository {
    pub definitions: Mutex<HashMap<String, WorkflowDefinition>>,
    pub read_error: Mutex<Option<String>>,
    pub deleted: Mutex<Vec<String>>,
}

impl FakeDefinitionRepository {
    pub fn insert(&self, definition: WorkflowDefinition) {
        self.definitions
            .lock()
            .unwrap()
            .insert(definition.name.clone(), definition);
    }
}

impl WorkflowDefinitionRepository for FakeDefinitionRepository {
    fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
        let mut summaries = self
            .definitions
            .lock()
            .unwrap()
            .values()
            .map(|definition| WorkflowSummary {
                failure: None,
                name: definition.name.clone(),
                description: definition.description.clone(),
                builtin: definition.builtin,
                is_running: running_names.contains(&definition.name),
                source_format: crate::domain::workflow::WorkflowSourceFormat::Yaml,
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(summaries)
    }

    fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        if let Some(message) = self.read_error.lock().unwrap().as_ref() {
            return Err(WorkflowError::external(message));
        }
        Ok(self.definitions.lock().unwrap().get(file_stem).cloned())
    }

    fn save(
        &self,
        definition: WorkflowDefinition,
        _original_name: Option<&str>,
    ) -> Result<(), WorkflowError> {
        self.definitions
            .lock()
            .unwrap()
            .insert(definition.name.clone(), definition);
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), WorkflowError> {
        self.deleted.lock().unwrap().push(name.to_string());
        self.definitions.lock().unwrap().remove(name);
        Ok(())
    }
}

#[cfg(test)]
impl FakeDefinitionRepository {
    pub fn seed(&self, definition: crate::domain::workflow::WorkflowDefinition) {
        self.insert(definition);
    }
    pub fn get_saved(&self, name: &str) -> Option<crate::domain::workflow::WorkflowDefinition> {
        self.definitions.lock().unwrap().get(name).cloned()
    }
}
