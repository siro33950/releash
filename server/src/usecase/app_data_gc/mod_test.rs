pub(crate) mod tests {
    use super::super::*;

    fn live_worktree(path: &str, key: &str) -> LiveWorktree {
        LiveWorktree {
            path: path.to_string(),
            workspace_state_keys: vec![key.to_string()],
            review_comment_keys: vec![key.to_string()],
        }
    }

    #[test]
    fn canonical_projection_owners_protect_active_session_and_running_workflow_paths() {
        let mut owners = CanonicalRuntimeOwners::default();
        apply_runtime_owner(
            CanonicalRuntimeOwnerView::AgentSession {
                worktree_path: "/worktrees/active".to_string(),
                active: true,
            },
            &mut owners,
        );
        apply_runtime_owner(
            CanonicalRuntimeOwnerView::ActiveWorkflow {
                worktree_path: "/worktrees/running".to_string(),
            },
            &mut owners,
        );

        assert!(owners
            .protected_worktree_paths
            .contains("/worktrees/active"));
        assert!(owners
            .protected_worktree_paths
            .contains("/worktrees/running"));
    }

    #[test]
    fn workspace_cleanup_is_closed_when_runtime_projection_is_incomplete() {
        let app_data = PathBuf::from("/app-data");
        let mut request = StartupGcRequest {
            app_data_dir: app_data.clone(),
            live_worktrees: Some(LiveWorktreeResolution::new(
                LiveWorktreeSet::from_worktrees([live_worktree("/live", "live")]),
                Vec::new(),
                HashSet::new(),
            )),
            workspace_state_records: vec![WorkspaceStateGcRecord {
                path: app_data.join("workspace_state/stale.json"),
                key: "stale".to_string(),
            }],
            review_comment_records: Vec::new(),
            checkpoint_paths: Vec::new(),
            cache_records: Vec::new(),
            legacy_comment_paths: Vec::new(),
            runtime_protection: RuntimeProtection::incomplete(),
            now_secs: 0.0,
            retention: RetentionPolicy::default(),
        };
        let mut plan = DeletionPlan::new(app_data);
        collect_workspace_keyed_deletions(&request, &mut plan);
        assert!(plan.candidates.is_empty());

        request.runtime_protection = RuntimeProtection::complete(LiveWorktreeSet::default());
        collect_workspace_keyed_deletions(&request, &mut plan);
        assert_eq!(plan.candidates.len(), 1);
    }

    #[test]
    fn candidate_containment_rejects_parent_traversal_and_siblings() {
        assert!(candidate_is_contained(
            Path::new("/app-data"),
            Path::new("/app-data/lsp/old")
        ));
        assert!(!candidate_is_contained(
            Path::new("/app-data"),
            Path::new("/app-data/../outside")
        ));
        assert!(!candidate_is_contained(
            Path::new("/app-data"),
            Path::new("/app-data-other/file")
        ));
    }
}

mod execution_tree_gc_tests {
    use super::super::*;
    use crate::domain::workflow::{ExecutionTreeArchiveCandidate, WorkflowError};
    use std::sync::Mutex;

    struct Trees {
        candidates: Vec<ExecutionTreeArchiveCandidate>,
        pages: Mutex<Vec<Option<String>>>,
        archives: Mutex<Vec<String>>,
        owners: Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl ExecutionTreeGc for Trees {
        async fn record_repository_root(&self, id: &str, root: &str) -> Result<(), WorkflowError> {
            self.owners.lock().unwrap().push((id.into(), root.into()));
            if id == "6-record-fail" {
                return Err(WorkflowError::external("repository record failed"));
            }
            Ok(())
        }
        async fn execution_trees(
            &self,
            after: Option<&str>,
        ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError> {
            self.pages.lock().unwrap().push(after.map(str::to_string));
            Ok(self
                .candidates
                .iter()
                .filter(|tree| after.is_none_or(|id| tree.execution_id.as_str() > id))
                .take(2)
                .cloned()
                .collect())
        }
        async fn archive_removed_tree(&self, id: &str) -> Result<(), WorkflowError> {
            self.archives.lock().unwrap().push(id.into());
            if id == "2-fail" {
                return Err(WorkflowError::external("archive failed"));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_gc実行木archive_ページを反復し所属repoだけを判定して失敗後も続ける() {
        // Given
        let trees = Trees {
            candidates: [
                ("1-live", "/live", Some("/repo")),
                ("2-fail", "/gone-fail", Some("/repo")),
                ("3-unreadable", "/gone-unreadable", Some("/unreadable")),
                ("4-removed", "/gone", Some("/repo")),
                ("5-legacy", "/repo-worktrees/gone", None),
                ("6-record-fail", "/gone-owner", Some("/repo")),
                ("7-linked", "/arbitrary-linked", None),
                ("8-removed", "/gone-next", Some("/repo")),
            ]
            .into_iter()
            .map(|(id, path, repo)| ExecutionTreeArchiveCandidate {
                execution_id: id.into(),
                worktree_path: path.into(),
                workspace_identity: path.into(),
                repository_root: repo.map(str::to_string),
            })
            .collect(),
            pages: Mutex::default(),
            archives: Mutex::default(),
            owners: Mutex::default(),
        };
        let resolution = LiveWorktreeResolution::new(
            LiveWorktreeSet::from_worktrees(["/live", "/arbitrary-linked"].map(|path| {
                LiveWorktree {
                    path: path.into(),
                    workspace_state_keys: vec![],
                    review_comment_keys: vec![],
                }
            })),
            vec!["/unreadable".into()],
            HashSet::new(),
        )
        .with_repository_paths(vec!["/repo".into(), "/unreadable".into()])
        .with_worktree_repositories(HashMap::from([(
            "/arbitrary-linked".into(),
            "/repo".into(),
        )]));
        // When
        let errors = archive_removed_execution_trees(Some(&resolution), &trees)
            .await
            .unwrap();
        // Then
        assert_eq!(errors, 2);
        assert_eq!(
            *trees.archives.lock().unwrap(),
            ["2-fail", "4-removed", "5-legacy", "8-removed"]
        );
        assert_eq!(
            *trees.pages.lock().unwrap(),
            [
                None,
                Some("2-fail".into()),
                Some("4-removed".into()),
                Some("6-record-fail".into()),
                Some("8-removed".into())
            ]
        );
        assert_eq!(trees.owners.lock().unwrap().len(), 8);
        assert!(trees
            .owners
            .lock()
            .unwrap()
            .contains(&("7-linked".into(), "/repo".into())));
        let calls = trees.pages.lock().unwrap().len();
        assert_eq!(
            archive_removed_execution_trees(None, &trees).await.unwrap(),
            0
        );
        assert_eq!(trees.pages.lock().unwrap().len(), calls);
    }
}
