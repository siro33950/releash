pub(crate) mod tests {

    use releash_lib::test_support::integration::platform::Deadline;
    use releash_lib::test_support::integration::platform::OperationContext;
    use releash_lib::test_support::integration::platform::OperationStopped;

    use releash_lib::test_support::integration::platform::acquire_worktree_file_lock;
    use releash_lib::test_support::integration::platform::lock_file;
    use releash_lib::test_support::integration::platform::state_file;
    use releash_lib::test_support::integration::platform::FileReviewEventStore;
    use releash_lib::test_support::integration::platform::ReviewActor;
    use releash_lib::test_support::integration::platform::ReviewError;
    use releash_lib::test_support::integration::platform::ReviewTarget;
    use releash_lib::test_support::integration::platform::SystemReviewClock;
    use releash_lib::test_support::integration::platform::UuidReviewIdGenerator;
    use std::time::Duration;
    use std::time::Instant;

    use releash_lib::test_support::integration::platform::ReviewCommentUsecase;
    use releash_lib::test_support::integration::platform::ReviewThreadState;
    use releash_lib::test_support::integration::platform::MAX_REVIEW_TEXT_BYTES;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::sync::Barrier;
    use tempfile::TempDir;

    fn usecase(store: Arc<FileReviewEventStore>) -> ReviewCommentUsecase {
        ReviewCommentUsecase::new(
            store,
            Arc::new(SystemReviewClock),
            Arc::new(UuidReviewIdGenerator),
        )
    }

    fn target() -> ReviewTarget {
        ReviewTarget {
            file_path: None,
            line_number: None,
            end_line: None,
        }
    }

    fn agent(session_id: &str) -> ReviewActor {
        ReviewActor::provider_agent("codex".to_string(), Some(session_id.to_string()))
    }

    #[test]
    pub fn missing_state_file_loads_as_empty_list() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let threads = usecase(store)
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap();

        assert!(threads.is_empty());
    }

    #[test]
    pub fn load_propagates_parse_error_without_overwriting_file() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{not-json").unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let result = usecase(store).create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            "A".to_string(),
        );

        assert!(matches!(result, Err(ReviewError::Serialize(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{not-json");
    }

    #[test]
    pub fn load_rejects_non_uuid_thread_id_before_projection() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"[
  {
    "eventType": "thread_created",
    "eventId": "event-1",
    "threadId": "thread-1",
    "commentId": "comment-1",
    "actor": {
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "sessionId": "legacy-session",
      "displayName": "codex/gpt-5"
    },
    "target": {
      "filePath": null,
      "lineNumber": null,
      "endLine": null
    },
    "content": "Claim",
    "at": 1.0
  }
]"#,
        )
        .unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let result = usecase(store).build_handoff(dir.path(), "wt", "thread-1", "releash");

        assert!(matches!(result, Err(ReviewError::Serialize(_))));
    }

    #[test]
    pub fn same_basename_worktrees_use_distinct_storage_keys() {
        let dir = TempDir::new().unwrap();
        let parent_a = TempDir::new().unwrap();
        let parent_b = TempDir::new().unwrap();
        let wt_a = parent_a.path().join("repo");
        let wt_b = parent_b.path().join("repo");
        std::fs::create_dir(&wt_a).unwrap();
        std::fs::create_dir(&wt_b).unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let usecase = usecase(store);

        usecase
            .create_thread(
                dir.path(),
                &wt_a.to_string_lossy(),
                ReviewActor::human(),
                target(),
                "A".to_string(),
            )
            .unwrap();

        assert!(usecase
            .list_threads(
                dir.path(),
                &wt_b.to_string_lossy(),
                None,
                ReviewActor::human()
            )
            .unwrap()
            .is_empty());
    }

    #[test]
    pub fn existing_lock_file_is_reused_without_ttl_steal() {
        let dir = TempDir::new().unwrap();
        let lock = lock_file(dir.path(), "wt");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();

        let guard = acquire_worktree_file_lock(dir.path(), "wt").unwrap();
        assert!(lock.exists());
        drop(guard);
        assert!(lock.exists());
    }

    #[test]
    pub fn multi_store_writes_keep_all_comments_and_resolve_once() {
        let dir = TempDir::new().unwrap();
        let first_store = Arc::new(FileReviewEventStore::default());
        let first_usecase = usecase(first_store);
        let thread = first_usecase
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();

        let mut handles = Vec::new();
        for content in ["A", "B"] {
            let path = dir.path().to_path_buf();
            let thread_id = thread.id.clone();
            handles.push(std::thread::spawn(move || {
                let store = Arc::new(FileReviewEventStore::default());
                usecase(store)
                    .append_comment(&path, "wt", agent(content), &thread_id, content.to_string())
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let current = first_usecase
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();
        assert_eq!(current.comments.len(), 3);

        let resolved = usecase(Arc::new(FileReviewEventStore::default()))
            .resolve_thread(
                dir.path(),
                "wt",
                agent("s2"),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();
        assert_eq!(resolved.state, ReviewThreadState::Resolved);

        let second = usecase(Arc::new(FileReviewEventStore::default())).resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "again".to_string(),
        );
        assert!(matches!(second, Err(ReviewError::AlreadyResolved(_))));
    }

    #[test]
    pub fn lockless_reads_do_not_observe_torn_json_during_concurrent_writes() {
        let dir = TempDir::new().unwrap();
        let initial_usecase = usecase(Arc::new(FileReviewEventStore::default()));
        let thread = initial_usecase
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();
        let app_data_dir = dir.path().to_path_buf();
        let thread_id = thread.id.clone();
        let start = Arc::new(Barrier::new(2));
        let done = Arc::new(AtomicBool::new(false));
        let writer_start = Arc::clone(&start);
        let writer_done = Arc::clone(&done);
        let writer = std::thread::spawn(move || {
            writer_start.wait();
            for index in 0..30 {
                usecase(Arc::new(FileReviewEventStore::default()))
                    .append_comment(
                        &app_data_dir,
                        "wt",
                        agent(&format!("writer-{index}")),
                        &thread_id,
                        format!("comment-{index}"),
                    )
                    .unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
            writer_done.store(true, Ordering::Release);
        });

        let reader_usecase = usecase(Arc::new(FileReviewEventStore::default()));
        let mut last_list_count = 1;
        let mut last_get_count = 1;
        let mut observed_concurrent_write = false;
        start.wait();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done.load(Ordering::Acquire) && Instant::now() < deadline {
            let threads = reader_usecase
                .list_threads(dir.path(), "wt", None, ReviewActor::human())
                .unwrap();
            assert_eq!(threads.len(), 1);
            let list_count = threads[0].comments.len();
            assert!(
                list_count >= last_list_count,
                "list observed comment count rollback: {last_list_count} -> {list_count}"
            );
            last_list_count = list_count;

            let got = reader_usecase
                .get_thread(dir.path(), "wt", &thread.id)
                .unwrap();
            let get_count = got.comments.len();
            assert!(
                get_count >= last_get_count,
                "get observed comment count rollback: {last_get_count} -> {get_count}"
            );
            if get_count > 1 {
                observed_concurrent_write = true;
            }
            last_get_count = get_count;
            std::thread::yield_now();
        }

        writer.join().unwrap();
        assert!(observed_concurrent_write);
        let final_thread = reader_usecase
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();
        assert_eq!(final_thread.comments.len(), 31);
    }

    #[test]
    pub fn test_comment変更_in_process_lock待ちを期限と取消で終了する() {
        use releash_lib::test_support::integration::platform::Deadline;
        use releash_lib::test_support::integration::platform::OperationContext;
        use releash_lib::test_support::integration::platform::OperationStopped;
        for expire in [false, true] {
            let dir = TempDir::new().unwrap();
            let store = Arc::new(FileReviewEventStore::default());
            let _guard = store.test_file_lock().lock();
            let token = tokio_util::sync::CancellationToken::new();
            let context = OperationContext::new(
                expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(50))),
                Arc::new(token.clone()),
            );
            let (started, ready) = std::sync::mpsc::channel();
            std::thread::scope(|scope| {
                let store = store.clone();
                let dir = dir.path();
                let worker = scope.spawn(move || {
                    releash_lib::test_support::integration::platform::sync_scope(context, || {
                        started.send(()).unwrap();
                        usecase(store).create_thread(
                            dir,
                            "wt",
                            agent("s1"),
                            target(),
                            "Claim".to_string(),
                        )
                    })
                });
                ready.recv().unwrap();
                if !expire {
                    std::thread::sleep(Duration::from_millis(30));
                    token.cancel();
                }
                let result = worker.join().unwrap();
                assert!(
                    matches!(result, Err(ReviewError::Technical(error)) if error == if expire { OperationStopped::Expired.into() } else { OperationStopped::Cancelled.into() })
                );
            });
            assert!(!state_file(dir.path(), "wt").exists());
        }
    }

    #[test]
    pub fn test_comment変更_実ファイルlock待ちを期限と取消で終了する() {
        // Given
        for expire in [false, true] {
            let dir = TempDir::new().unwrap();
            let store = Arc::new(FileReviewEventStore::default());
            let guard = acquire_worktree_file_lock(dir.path(), "wt").unwrap();
            let token = tokio_util::sync::CancellationToken::new();
            let (started, ready) = std::sync::mpsc::channel();
            let (sent, received) = std::sync::mpsc::channel();
            std::thread::scope(|scope| {
                let store = store.clone();
                let path = dir.path();
                let cancellation = token.clone();
                scope.spawn(move || {
                    let started_at = Instant::now();
                    let context = OperationContext::new(
                        expire.then(|| Deadline::new(started_at + Duration::from_millis(100))),
                        Arc::new(cancellation),
                    );
                    started.send(started_at).unwrap();
                    let result = releash_lib::test_support::integration::platform::sync_scope(
                        context,
                        || {
                            usecase(store).create_thread(
                                path,
                                "wt",
                                agent("s1"),
                                target(),
                                "Claim".into(),
                            )
                        },
                    );
                    sent.send(result).unwrap();
                });
                // When / Then
                let started_at = ready.recv_timeout(Duration::from_secs(2)).unwrap();
                if !expire {
                    assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
                    token.cancel();
                }
                let result = received.recv_timeout(Duration::from_secs(2)).unwrap();
                if expire {
                    assert!(started_at.elapsed() >= Duration::from_millis(100));
                }
                assert!(
                    matches!(result, Err(ReviewError::Technical(error)) if error == if expire { OperationStopped::Expired.into() } else { OperationStopped::Cancelled.into() })
                );
                assert!(!state_file(path, "wt").exists());
            });
            drop(guard);
            assert!(acquire_worktree_file_lock(dir.path(), "wt").is_ok());
        }
    }

    #[test]
    pub fn test_comment資源期限_親が無期限でも長い期限でも十秒で終了する() {
        for parent_deadline in [false, true] {
            // Given
            let dir = TempDir::new().unwrap();
            let guard = acquire_worktree_file_lock(dir.path(), "wt").unwrap();
            let start = Instant::now();
            let context = OperationContext::new(
                parent_deadline.then(|| Deadline::new(start + Duration::from_secs(30))),
                Arc::new(tokio_util::sync::CancellationToken::new()),
            );
            let path = dir.path().to_path_buf();
            let (sent, received) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let result =
                    releash_lib::test_support::integration::platform::sync_scope(context, || {
                        acquire_worktree_file_lock(&path, "wt").map(drop)
                    });
                sent.send(result).unwrap();
            });
            // When
            let result = received.recv_timeout(Duration::from_secs(15));
            let elapsed = start.elapsed();
            drop(guard);
            worker.join().unwrap();
            // Then
            assert!(matches!(
                result.unwrap(),
                Err(ReviewError::Technical(
                    releash_lib::test_support::integration::platform::TechnicalFailure {
                        nature: releash_lib::test_support::integration::platform::TechnicalFailureNature::TimedOut,
                        ..
                    }
                ))
            ));
            assert!(elapsed >= Duration::from_secs(10));
            assert!(elapsed < Duration::from_secs(15));
            assert!(acquire_worktree_file_lock(dir.path(), "wt").is_ok());
        }
    }

    #[test]
    pub fn read_only_operations_do_not_wait_for_in_process_write_guard() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let thread = usecase(Arc::clone(&store))
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();
        let _guard = store.test_file_lock().lock();

        let started = Instant::now();
        let threads = usecase(Arc::clone(&store))
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap();
        let got = usecase(Arc::clone(&store))
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(got.id, thread.id);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    pub fn read_only_operations_do_not_wait_for_process_file_lock() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let thread = usecase(Arc::clone(&store))
            .create_thread(dir.path(), "wt", agent("s1"), target(), "Claim".to_string())
            .unwrap();
        let _guard = acquire_worktree_file_lock(dir.path(), "wt").unwrap();

        let started = Instant::now();
        let threads = usecase(Arc::clone(&store))
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap();
        let got = usecase(Arc::clone(&store))
            .get_thread(dir.path(), "wt", &thread.id)
            .unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(got.id, thread.id);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    pub fn invalid_content_does_not_create_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let too_long = "a".repeat(MAX_REVIEW_TEXT_BYTES + 1);

        let result = usecase(store).create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            too_long,
        );

        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
        assert!(!state_file(dir.path(), "wt").exists());
    }

    #[test]
    pub fn invalid_target_does_not_create_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let absolute_path = if cfg!(windows) {
            "C:/repo/src/main.rs"
        } else {
            "/repo/src/main.rs"
        };
        let invalid_targets = [
            ReviewTarget {
                file_path: Some(absolute_path.to_string()),
                line_number: Some(1),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src\\main.rs".to_string()),
                line_number: Some(1),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs\0".to_string()),
                line_number: Some(1),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(0),
                end_line: None,
            },
            ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(5),
                end_line: Some(4),
            },
        ];

        for target in invalid_targets {
            let result = usecase(Arc::clone(&store)).create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target,
                "Claim".to_string(),
            );

            assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
            assert!(!state_file(dir.path(), "wt").exists());
        }
    }

    #[test]
    pub fn nul_content_does_not_create_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        let result = usecase(store).create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            "bad\0content".to_string(),
        );

        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
        assert!(!state_file(dir.path(), "wt").exists());
    }

    #[test]
    pub fn nul_mutations_do_not_update_existing_state_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());
        let usecase = usecase(store);
        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        let file = state_file(dir.path(), "wt");
        let before = std::fs::read_to_string(&file).unwrap();

        let append = usecase.append_comment(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "bad\0comment".to_string(),
        );
        assert!(matches!(append, Err(ReviewError::InvalidInput(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

        let outcome = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "bad\0outcome".to_string(),
            "done".to_string(),
        );
        assert!(matches!(outcome, Err(ReviewError::InvalidInput(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

        let summary = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "bad\0summary".to_string(),
        );
        assert!(matches!(summary, Err(ReviewError::InvalidInput(_))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    pub fn persisted_actor_keeps_existing_session_id_field() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        usecase(store)
            .create_thread(
                dir.path(),
                "wt",
                agent("secret-session"),
                target(),
                "Claim".to_string(),
            )
            .unwrap();

        let json = std::fs::read_to_string(state_file(dir.path(), "wt")).unwrap();
        let events: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(events[0]["actor"]["sessionId"], "secret-session");
    }

    #[test]
    pub fn persisted_human_actor_keeps_session_id_null_field() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        usecase(store)
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();

        let json = std::fs::read_to_string(state_file(dir.path(), "wt")).unwrap();
        let events: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(events[0]["actor"].get("sessionId").is_some());
        assert!(events[0]["actor"]["sessionId"].is_null());
    }

    #[test]
    pub fn loaded_actor_session_id_survives_rewrite() {
        let dir = TempDir::new().unwrap();
        let file = state_file(dir.path(), "wt");
        let thread_id = "018f8f6d-0e6a-7b2c-9d10-111111111111";
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let json = r#"[
  {
    "eventType": "thread_created",
    "eventId": "event-1",
    "threadId": "__THREAD_ID__",
    "commentId": "comment-1",
    "actor": {
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "sessionId": "legacy-session",
      "displayName": "codex/gpt-5"
    },
    "target": {
      "filePath": null,
      "lineNumber": null,
      "endLine": null
    },
    "content": "Claim",
    "at": 1.0
  }
]"#
        .replace("__THREAD_ID__", thread_id);
        std::fs::write(&file, json).unwrap();
        let store = Arc::new(FileReviewEventStore::default());

        usecase(store)
            .append_comment(
                dir.path(),
                "wt",
                agent("new-session"),
                thread_id,
                "Follow-up".to_string(),
            )
            .unwrap();

        let json = std::fs::read_to_string(file).unwrap();
        let events: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(events[0]["actor"]["sessionId"], "legacy-session");
        assert_eq!(events[1]["actor"]["sessionId"], "new-session");
    }
}
