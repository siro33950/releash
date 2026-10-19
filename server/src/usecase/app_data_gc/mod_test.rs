pub(crate) mod tests {
    use super::super::*;

    fn live_worktree(path: &str, key: &str) -> LiveWorktree {
        LiveWorktree {
            path: path.to_string(),
            workspace_state_keys: vec![WorkspaceStateKey::for_test(key)],
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
            )),
            workspace_state_records: vec![WorkspaceStateGcRecord {
                path: app_data.join("workspace_state/stale.json"),
                key: WorkspaceStateKey::for_test("stale"),
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
        collect_workspace_keyed_deletions(&request, &mut plan, &mut GcResult::default());
        assert!(plan.candidates.is_empty());
        let report = plan_startup_gc(request.clone()).report;
        assert!(matches!(
            report.notices.as_slice(),
            [GcNotice::RulesSkipped(
                GcSkipReason::RuntimeProtectionIncomplete
            )]
        ));

        request.runtime_protection = RuntimeProtection::complete(LiveWorktreeSet::default());
        collect_workspace_keyed_deletions(&request, &mut plan, &mut GcResult::default());
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
        )
        .with_repository_paths(vec!["/repo".into(), "/unreadable".into()])
        .with_worktree_repositories(HashMap::from([(
            "/arbitrary-linked".into(),
            "/repo".into(),
        )]));
        // When
        let errors = archive_removed_execution_trees(Some(&resolution), &trees).await;
        // Then
        assert!(errors.stopped.is_none());
        assert_eq!(errors.report.errors, 2);
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
            archive_removed_execution_trees(None, &trees)
                .await
                .report
                .errors,
            0
        );
        assert_eq!(trees.pages.lock().unwrap().len(), calls);
    }
}
mod workspace_state_gc_tests {
    use super::super::*;
    use crate::domain::local_event::StreamVersion;
    use crate::domain::workspace_state::WorkspaceStateError;
    use std::sync::Mutex;

    struct States {
        scanned: std::sync::atomic::AtomicBool,
        fail_inventory: bool,
        fail_invalidation: bool,
        invalidated: Mutex<Vec<WorkspaceStateKey>>,
        added: Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl crate::domain::workspace_state::WorkspaceStateRepository for States {
        async fn migrate_legacy(
            &self,
            _: &[String],
            _: &[String],
        ) -> Result<(), WorkspaceStateError> {
            unreachable!()
        }
        async fn load_repository_group(&self, _: &str) -> Result<bool, WorkspaceStateError> {
            unreachable!()
        }
        async fn read_repository_group(
            &self,
            _: &str,
        ) -> Result<(StreamVersion, bool), WorkspaceStateError> {
            unreachable!()
        }
        async fn save_repository_group(
            &self,
            _: &str,
            _: bool,
            _: StreamVersion,
        ) -> Result<(), WorkspaceStateError> {
            unreachable!()
        }
        async fn filter_missing_files(
            &self,
            _: crate::domain::workspace_state::WorkspaceState,
            _: &str,
        ) -> Result<crate::domain::workspace_state::WorkspaceState, WorkspaceStateError> {
            unreachable!()
        }
        async fn read(
            &self,
            _: &str,
        ) -> Result<
            (
                StreamVersion,
                Option<crate::domain::workspace_state::WorkspaceState>,
            ),
            WorkspaceStateError,
        > {
            unreachable!()
        }
        async fn save(
            &self,
            _: &str,
            _: crate::domain::workspace_state::WorkspaceState,
            _: StreamVersion,
        ) -> Result<(), WorkspaceStateError> {
            unreachable!()
        }

        async fn states(
            &self,
        ) -> Result<Vec<(WorkspaceStateKey, StreamVersion)>, WorkspaceStateError> {
            self.scanned
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if self.fail_inventory {
                return Err(WorkspaceStateError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "inventory failed".into(),
                    },
                ));
            }
            Ok(["live", "active", "newly-active", "removed"]
                .into_iter()
                .map(String::from)
                .chain(self.added.lock().unwrap().clone())
                .map(|key| {
                    (
                        WorkspaceStateKey::for_test(key),
                        StreamVersion::new(1).unwrap(),
                    )
                })
                .collect())
        }
        async fn invalidate(
            &self,
            key: &WorkspaceStateKey,
            expected: StreamVersion,
        ) -> Result<bool, WorkspaceStateError> {
            assert_eq!(expected, StreamVersion::new(1).unwrap());
            self.invalidated.lock().unwrap().push(key.clone());
            if self.fail_invalidation {
                Err(WorkspaceStateError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "write failed".into(),
                    },
                ))
            } else {
                Ok(true)
            }
        }
    }
    fn worktrees(keys: &[&str]) -> LiveWorktreeSet {
        LiveWorktreeSet::from_worktrees(keys.iter().map(|key| LiveWorktree {
            path: (*key).into(),
            workspace_state_keys: vec![WorkspaceStateKey::for_test(*key)],
            review_comment_keys: vec![],
        }))
    }
    fn plan(unresolved: bool, initial_complete: bool) -> StartupGcPlan {
        plan_startup_gc(StartupGcRequest {
            app_data_dir: PathBuf::from("/app-data"),
            live_worktrees: Some(LiveWorktreeResolution::new(
                worktrees(&["live"]),
                if unresolved {
                    vec!["/missing".into()]
                } else {
                    vec![]
                },
            )),
            workspace_state_records: vec![],
            review_comment_records: vec![],
            checkpoint_paths: vec![],
            cache_records: vec![],
            legacy_comment_paths: vec![],
            runtime_protection: if initial_complete {
                RuntimeProtection::complete(worktrees(&["active"]))
            } else {
                RuntimeProtection::incomplete()
            },
            now_secs: 0.0,
            retention: RetentionPolicy::default(),
        })
    }
    #[tokio::test]
    async fn test_ui状態gc_候補読取後に作成されたworktreeの状態は無効化しない() {
        use crate::domain::workspace_state::WorkspaceStateRepository;
        // Given
        let states = States {
            scanned: Default::default(),
            fail_inventory: false,
            fail_invalidation: false,
            invalidated: Mutex::default(),
            added: Mutex::default(),
        };
        let inventory = WorkspaceStateCandidates::from(states.states().await);
        states.added.lock().unwrap().push("new-worktree".into());
        let request = plan(false, true).request;
        let runtime = RuntimeProtection::complete(worktrees(&["newly-active"]));
        let mut report = GcResult::default();
        // When
        sweep_workspace_states(
            &plan_startup_gc(request),
            &runtime,
            &states,
            inventory,
            &mut report,
        )
        .await;
        // Then
        assert!(states
            .states()
            .await
            .unwrap()
            .iter()
            .any(|(key, _)| key == &WorkspaceStateKey::for_test("new-worktree")));
        assert_eq!(
            *states.invalidated.lock().unwrap(),
            [WorkspaceStateKey::for_test("removed")]
        );
    }

    #[tokio::test]
    async fn test_ui状態gc_現存と両時点の実行中を守り削除済みだけ無効化する() {
        // Given
        let states = States {
            scanned: Default::default(),
            fail_inventory: false,
            fail_invalidation: false,
            invalidated: Mutex::default(),
            added: Mutex::default(),
        };
        let runtime = RuntimeProtection::complete(worktrees(&["newly-active"]));
        let mut report = GcResult::default();
        // When
        sweep_workspace_states(
            &plan(false, true),
            &runtime,
            &states,
            WorkspaceStateCandidates::from(
                crate::domain::workspace_state::WorkspaceStateRepository::states(&states).await,
            ),
            &mut report,
        )
        .await;
        // Then
        assert_eq!(
            *states.invalidated.lock().unwrap(),
            [WorkspaceStateKey::for_test("removed")]
        );
        assert_eq!(
            report.report.categories[&GcCategory::DeletedWorkspace].deleted,
            1
        );
        assert_eq!(report.report.errors, 0);
    }
    #[tokio::test]
    async fn test_ui状態gc_未解決と不完全な保護は処理せず読取と無効化の失敗を報告する() {
        for (
            unresolved,
            initial_complete,
            sweep_complete,
            fail_inventory,
            fail_invalidation,
            errors,
            attempted,
        ) in [
            (true, true, true, false, false, 0, false),
            (false, false, true, false, false, 0, false),
            (false, true, false, false, false, 0, false),
            (false, true, true, true, false, 1, false),
            (false, true, true, false, true, 1, true),
        ] {
            // Given
            let states = States {
                scanned: Default::default(),
                fail_inventory,
                fail_invalidation,
                invalidated: Mutex::default(),
                added: Mutex::default(),
            };
            let runtime = if sweep_complete {
                RuntimeProtection::complete(worktrees(&["newly-active"]))
            } else {
                RuntimeProtection::incomplete()
            };
            let mut report = GcResult::default();
            // When
            let inventory = WorkspaceStateCandidates::from(
                crate::domain::workspace_state::WorkspaceStateRepository::states(&states).await,
            );
            let request = plan(unresolved, initial_complete).request;
            sweep_workspace_states(
                &plan_startup_gc(request),
                &runtime,
                &states,
                inventory,
                &mut report,
            )
            .await;
            // Then
            assert_eq!(report.report.errors, errors);
            assert_eq!(!states.invalidated.lock().unwrap().is_empty(), attempted);
            assert!(!report
                .report
                .categories
                .contains_key(&GcCategory::DeletedWorkspace));
        }
    }
    struct Inventory {
        states: Arc<States>,
    }
    #[async_trait::async_trait]
    impl StartupGcInventory for Inventory {
        async fn inventory(&self) -> Result<StartupGcRequest, String> {
            assert!(self
                .states
                .scanned
                .load(std::sync::atomic::Ordering::SeqCst));
            let mut request = plan(false, true).request;
            request.runtime_protection = RuntimeProtection::incomplete();
            request.cache_records.push(CacheGcRecord {
                path: PathBuf::from("/app-data/lsp/stale"),
                updated_at: 0.0,
            });
            request.now_secs = 10.0;
            request.retention.cache_secs = 0;
            Ok(request)
        }
        fn runtime_protection(&self, _: CanonicalRuntimeOwners) -> RuntimeProtection {
            RuntimeProtection::complete(worktrees(&["active", "newly-active"]))
        }
    }
    #[derive(Default)]
    struct Owners {
        fail_on: Option<usize>,
        queries: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl LocalEventTransactionRepository for Owners {
        async fn query(
            &self,
            _: LocalEventQuery,
        ) -> Result<LocalEventQueryResult, crate::domain::local_event::LocalEventQueryError>
        {
            let query = self
                .queries
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if self.fail_on == Some(query) {
                return Err(crate::domain::local_event::LocalEventQueryError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "owners unavailable".into(),
                    },
                ));
            }
            Ok(LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(vec![]))
        }
        async fn commit_batch(
            &self,
            _: crate::domain::local_event::LocalAtomicBatch,
        ) -> Result<
            crate::domain::local_event::CommitBatchResult,
            crate::domain::local_event::CommitBatchError,
        > {
            unreachable!()
        }
        async fn resolve_commit(
            &self,
            _: crate::domain::local_event::CommitIdentity,
        ) -> Result<
            crate::domain::local_event::CommitResolution,
            crate::domain::local_event::LocalEventQueryError,
        > {
            unreachable!()
        }
        async fn load_stream(
            &self,
            _: crate::domain::local_event::LoadStreamRequest,
        ) -> Result<
            crate::domain::local_event::DomainEventPage,
            crate::domain::local_event::LocalEventQueryError,
        > {
            unreachable!()
        }
    }
    struct Trees {
        fail_page: bool,
        archived: Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl ExecutionTreeGc for Trees {
        async fn execution_trees(
            &self,
            after: Option<&str>,
        ) -> Result<
            Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>,
            crate::domain::workflow::WorkflowError,
        > {
            if after.is_some() && self.fail_page {
                return Err(crate::domain::workflow::WorkflowError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "next page failed".into(),
                    },
                ));
            }
            Ok(if after.is_some() {
                vec![]
            } else {
                ["ok", "fail", "record-fail"]
                    .into_iter()
                    .filter(|id| *id != "record-fail" || self.fail_page)
                    .map(
                        |id| crate::domain::workflow::ExecutionTreeArchiveCandidate {
                            execution_id: id.into(),
                            worktree_path: "/gone".into(),
                            workspace_identity: "/gone".into(),
                            repository_root: Some("/repo".into()),
                        },
                    )
                    .collect()
            })
        }
        async fn record_repository_root(
            &self,
            id: &str,
            _: &str,
        ) -> Result<(), crate::domain::workflow::WorkflowError> {
            if id == "record-fail" {
                Err(crate::domain::workflow::WorkflowError::external(
                    "record failed",
                ))
            } else {
                Ok(())
            }
        }
        async fn archive_removed_tree(
            &self,
            id: &str,
        ) -> Result<(), crate::domain::workflow::WorkflowError> {
            self.archived.lock().unwrap().push(id.into());
            if id == "fail" {
                Err(crate::domain::workflow::WorkflowError::external(
                    "archive failed",
                ))
            } else {
                Ok(())
            }
        }
    }
    #[derive(Default)]
    struct Files {
        panic_on_measure: bool,
        removed: Mutex<Vec<PathBuf>>,
    }
    impl GcFileSystem for Files {
        fn metadata(&self, _: &Path) -> Result<GcMetadata, GcFileSystemError> {
            unreachable!()
        }
        fn read_dir(&self, _: &Path) -> Result<Vec<PathBuf>, GcFileSystemError> {
            unreachable!()
        }
        fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError> {
            self.removed.lock().unwrap().push(path.into());
            Ok(true)
        }
        fn recursive_size(&self, _: &Path) -> Result<u64, GcFileSystemError> {
            assert!(!self.panic_on_measure, "sweep failed");
            Ok(10)
        }
    }
    #[tokio::test]
    async fn test_起動時gc_候補を先に読みarchiveと無効化とファイル削除の報告をまとめる() {
        // Given
        let states = Arc::new(States {
            scanned: Default::default(),
            fail_inventory: false,
            fail_invalidation: false,
            invalidated: Mutex::default(),
            added: Mutex::default(),
        });
        let trees = Trees {
            fail_page: false,
            archived: Mutex::default(),
        };
        let files = Arc::new(Files::default());
        // When
        let report = execute_startup_gc(
            &Inventory {
                states: states.clone(),
            },
            Arc::new(Owners::default()),
            &trees,
            states.as_ref(),
            files.clone(),
        )
        .await;
        // Then
        assert!(report.stopped.is_none());
        assert_eq!(*trees.archived.lock().unwrap(), ["ok", "fail"]);
        assert_eq!(
            *states.invalidated.lock().unwrap(),
            [WorkspaceStateKey::for_test("removed")]
        );
        assert_eq!(
            *files.removed.lock().unwrap(),
            [PathBuf::from("/app-data/lsp/stale")]
        );
        assert_eq!(report.report.errors, 1);
        assert_eq!(report.report.total_files, 2);
        assert_eq!(report.report.total_bytes, 10);
        assert_eq!(
            report.report.categories[&GcCategory::DeletedWorkspace].deleted,
            1
        );
        assert_eq!(
            report.report.categories[&GcCategory::RegenerableCache].deleted,
            1
        );
        assert!(
            matches!(&report.notices[..], [GcNotice::ArchiveFailed { execution_id, error }] if execution_id == "fail" && error.to_string().contains("archive failed"))
        );
    }
    #[tokio::test]
    async fn test_起動時gc_後続ページの失敗にも先行ページの診断を残す() {
        // Given
        let states = Arc::new(States {
            scanned: Default::default(),
            fail_inventory: false,
            fail_invalidation: false,
            invalidated: Mutex::default(),
            added: Mutex::default(),
        });
        let trees = Trees {
            fail_page: true,
            archived: Mutex::default(),
        };
        let files = Arc::new(Files::default());
        // When
        let failure = execute_startup_gc(
            &Inventory {
                states: states.clone(),
            },
            Arc::new(Owners::default()),
            &trees,
            states.as_ref(),
            files.clone(),
        )
        .await;
        // Then
        assert!(
            matches!(&failure.stopped, Some(GcStopCause::ExecutionTrees(crate::domain::workflow::WorkflowError::Technical(error))) if error.message == "next page failed" && error.nature == crate::domain::failure::TechnicalFailureNature::Transient)
        );
        assert_eq!(failure.report.errors, 2);
        assert!(
            matches!(&failure.notices[..], [GcNotice::ArchiveFailed { execution_id, error }, GcNotice::RepositoryRecordFailed { execution_id: owner, error: owner_error }] if execution_id == "fail" && error.to_string().contains("archive failed") && owner == "record-fail" && owner_error.to_string().contains("record failed"))
        );
        assert!(states.invalidated.lock().unwrap().is_empty());
        assert!(files.removed.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn test_起動時gc_候補読取の失敗は一度報告し後続の削除を続ける() {
        // Given
        let states = Arc::new(States {
            scanned: Default::default(),
            fail_inventory: true,
            fail_invalidation: false,
            invalidated: Mutex::default(),
            added: Mutex::default(),
        });
        let trees = Trees {
            fail_page: false,
            archived: Mutex::default(),
        };
        let files = Arc::new(Files::default());
        // When
        let result = execute_startup_gc(
            &Inventory {
                states: states.clone(),
            },
            Arc::new(Owners::default()),
            &trees,
            states.as_ref(),
            files.clone(),
        )
        .await;
        // Then
        assert!(result.stopped.is_none());
        assert_eq!(result.report.errors, 2);
        assert_eq!(result.report.total_files, 1);
        assert_eq!(result.report.total_bytes, 10);
        assert!(
            matches!(result.notices.as_slice(), [GcNotice::ArchiveFailed { .. }, GcNotice::WorkspaceInventoryFailed(WorkspaceStateError::Technical(error))] if error.message == "inventory failed")
        );
        assert!(states.invalidated.lock().unwrap().is_empty());
        assert_eq!(
            *files.removed.lock().unwrap(),
            [PathBuf::from("/app-data/lsp/stale")]
        );
    }

    #[tokio::test]
    async fn test_起動時gc_保護の読取と再読取の失敗を別々に報告してui状態を保持する() {
        for fail_on in [1, 2] {
            // Given
            let states = Arc::new(States {
                scanned: Default::default(),
                fail_inventory: false,
                fail_invalidation: false,
                invalidated: Mutex::default(),
                added: Mutex::default(),
            });
            let trees = Trees {
                fail_page: false,
                archived: Mutex::default(),
            };
            let files = Arc::new(Files::default());
            // When
            let result = execute_startup_gc(
                &Inventory {
                    states: states.clone(),
                },
                Arc::new(Owners {
                    fail_on: Some(fail_on),
                    ..Default::default()
                }),
                &trees,
                states.as_ref(),
                files.clone(),
            )
            .await;
            // Then
            assert!(result.stopped.is_none());
            assert!(states.invalidated.lock().unwrap().is_empty());
            assert_eq!(result.report.total_files, 1);
            assert_eq!(result.report.total_bytes, 10);
            if fail_on == 1 {
                assert!(
                    matches!(&result.notices[1], GcNotice::CanonicalProtectionFailed { error } if error.contains("owners unavailable"))
                );
                assert!(matches!(
                    &result.notices[2],
                    GcNotice::RulesSkipped(GcSkipReason::RuntimeProtectionIncomplete)
                ));
            } else {
                assert!(
                    matches!(&result.notices[1], GcNotice::CanonicalRevalidationFailed { error } if error.contains("owners unavailable"))
                );
            }
        }
    }

    #[tokio::test]
    async fn test_起動時gc_blocking失敗も技術的失敗と先行する集計を返す() {
        // Given
        let states = Arc::new(States {
            scanned: Default::default(),
            fail_inventory: false,
            fail_invalidation: false,
            invalidated: Mutex::default(),
            added: Mutex::default(),
        });
        let trees = Trees {
            fail_page: false,
            archived: Mutex::default(),
        };
        // When
        let result = execute_startup_gc(
            &Inventory {
                states: states.clone(),
            },
            Arc::new(Owners::default()),
            &trees,
            states.as_ref(),
            Arc::new(Files {
                panic_on_measure: true,
                ..Default::default()
            }),
        )
        .await;
        // Then
        assert!(
            matches!(&result.stopped, Some(GcStopCause::Sweep(error)) if error.nature == crate::domain::failure::TechnicalFailureNature::Other && error.message.contains("sweep failed"))
        );
        assert_eq!(result.report.errors, 1);
        assert_eq!(result.report.total_files, 1);
        assert_eq!(result.report.total_bytes, 0);
        assert!(matches!(
            result.notices.as_slice(),
            [GcNotice::ArchiveFailed { .. }]
        ));
    }
}
