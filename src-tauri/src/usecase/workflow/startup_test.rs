use super::*;
use crate::domain::workflow::entities::workflow_execution::ExecutionTree;
use crate::domain::workflow::repository::WorkflowStartupRecord;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionTreeLaunch, NodeFact, NodeFactMeta, NodeKindName, TreeRootFact,
};
use std::sync::Mutex;

struct Repository {
    terminal: Mutex<Option<NodeFact>>,
    unreadable: bool,
    fail_load: bool,
    fail_append: bool,
    appended: Mutex<Vec<(NodeFact, f64)>>,
}

impl WorkflowStartupRepository for Repository {
    fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        Ok(vec!["tree".into()])
    }

    fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
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
            root: NodeFactMeta {
                tree_id: tree_id.into(),
                node_execution_id: "root".into(),
                parent_id: None,
                node_name: "main".into(),
                kind: NodeKindName::Command,
                attempt: 1,
            },
            definition_error: self
                .unreadable
                .then(|| "Workflow definition is unavailable: completion".into()),
        }))
    }
    fn append(
        &self,
        root: &NodeFactMeta,
        fact: &NodeFact,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        assert_eq!(root.tree_id, "tree");
        assert_eq!(root.node_execution_id, "root");
        if self.fail_append {
            return Err(WorkflowError::external("append failed"));
        }
        self.appended
            .lock()
            .unwrap()
            .push((fact.clone(), timestamp));
        *self.terminal.lock().unwrap() = Some(fact.clone());
        Ok(())
    }
}

fn repository() -> Repository {
    Repository {
        terminal: Mutex::new(None),
        unreadable: true,
        fail_load: false,
        fail_append: false,
        appended: Mutex::new(Vec::new()),
    }
}

#[test]
fn test_起動時abort_理由付き事実を保存し再実行では追記しない() {
    // Given
    let repository = repository();
    // When
    abort_unavailable_definition(&repository, "tree", 3.0).unwrap();
    abort_unavailable_definition(&repository, "tree", 4.0).unwrap();
    // Then
    assert_eq!(
        *repository.appended.lock().unwrap(),
        vec![(
            NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
                reason: Some("Workflow definition is unavailable: completion".into())
            }),
            3.0
        )]
    );
}

#[test]
fn test_起動時abort_読める定義と既存の終端事実には追記しない() {
    // Given
    for terminal in [
        None,
        Some(NodeFact::ExecutionCompleted),
        Some(NodeFact::AbortRequested(Default::default())),
    ] {
        let mut repository = repository();
        repository.unreadable = terminal.is_some();
        *repository.terminal.lock().unwrap() = terminal;
        // When
        abort_unavailable_definition(&repository, "tree", 3.0).unwrap();
        // Then
        assert!(repository.appended.lock().unwrap().is_empty());
    }
}

#[test]
fn test_起動時abort_読取と追記の失敗を返し再試行で保存できる() {
    // Given
    for fail_load in [true, false] {
        let mut repository = repository();
        repository.fail_load = fail_load;
        repository.fail_append = !fail_load;
        // When / Then
        assert!(abort_unavailable_definition(&repository, "tree", 3.0).is_err());
        assert!(repository.appended.lock().unwrap().is_empty());
        repository.fail_load = false;
        repository.fail_append = false;
        abort_unavailable_definition(&repository, "tree", 4.0).unwrap();
        assert_eq!(repository.appended.lock().unwrap().len(), 1);
    }
}

struct Startup {
    calls: Mutex<Vec<String>>,
    failure: Option<&'static str>,
}

impl Startup {
    fn record(&self, call: String) -> Result<(), WorkflowError> {
        self.calls.lock().unwrap().push(call.clone());
        if self.failure == Some(call.as_str()) {
            Err(WorkflowError::external(call))
        } else {
            Ok(())
        }
    }
}

impl WorkflowStartupRepository for Startup {
    fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        self.record("list".into())?;
        Ok(["registered", "reserved", "first", "second"]
            .map(String::from)
            .into())
    }

    fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        self.record(format!("load:{tree_id}"))?;
        repository().load(tree_id)
    }

    fn append(
        &self,
        root: &NodeFactMeta,
        fact: &NodeFact,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        assert!(matches!(fact, NodeFact::AbortRequested(_)));
        assert_eq!(timestamp, 3.0);
        self.record(format!("append:{}", root.tree_id))
    }
}

#[async_trait::async_trait]
impl WorkflowStartupGateway for Startup {
    fn current_timestamp(&self) -> f64 {
        3.0
    }

    async fn is_registered_or_reserved(&self, tree_id: &str) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push(format!("registered:{tree_id}"));
        matches!(tree_id, "registered" | "reserved")
    }

    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError> {
        assert_eq!(timestamp, 3.0);
        tokio::task::yield_now().await;
        self.record(format!("reconcile:{tree_id}"))
    }
}

#[tokio::test]
async fn test_起動時復旧_列挙とabort保存とreconciliationの順序を所有する() {
    // Given
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: None,
    });
    let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());

    // When
    let (first, second) = tokio::join!(usecase.execute(), usecase.execute());
    first.unwrap();
    second.unwrap();

    // Then
    let pass = [
        "list",
        "registered:registered",
        "registered:reserved",
        "registered:first",
        "load:first",
        "append:first",
        "reconcile:first",
        "registered:second",
        "load:second",
        "append:second",
        "reconcile:second",
    ];
    assert_eq!(*startup.calls.lock().unwrap(), pass.repeat(2));
}

#[tokio::test]
async fn test_起動時復旧_abort失敗したtreeを再生せず後続を処理して最初の失敗を返す() {
    for failure in ["load:first", "append:first", "reconcile:first"] {
        // Given
        let startup = Arc::new(Startup {
            calls: Mutex::new(Vec::new()),
            failure: Some(failure),
        });
        let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());

        // When
        let error = usecase.execute().await.unwrap_err();

        // Then
        assert!(error.to_string().contains(failure));
        let calls = startup.calls.lock().unwrap();
        assert_eq!(
            calls.contains(&"reconcile:first".into()),
            failure == "reconcile:first"
        );
        assert_eq!(
            &calls[calls.len() - 4..],
            [
                "registered:second",
                "load:second",
                "append:second",
                "reconcile:second"
            ]
        );
    }
}

#[tokio::test]
async fn test_起動時復旧_列挙失敗は後続操作を呼ばず返す() {
    // Given
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: Some("list"),
    });
    let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());
    // When / Then
    assert!(usecase
        .execute()
        .await
        .unwrap_err()
        .to_string()
        .contains("list"));
    assert_eq!(*startup.calls.lock().unwrap(), ["list"]);
}
