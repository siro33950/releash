//! [06] CLI mutating CLI 経路の watcher + dispatcher adapter。
//!
//! `<data_dir>/workflow_pending/pending/` を `notify-debouncer-mini` で監視し、
//! 新規 pending entry を dispatcher adapter に渡す。entry claim / 処理済み
//! マーキング / retry 判定は dispatcher adapter が担う。
//!
//! TTL 境界 (spec [06]): watcher 起動時と各 pickup サイクルの双方で
//! `cleanup_expired` を呼び、古い未処理要求を engine 到達対象から除外する。

use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use tauri::Manager;
use tokio::sync::{mpsc, Mutex};

use crate::agent_sdk::AgentProcessMap;
use crate::agent_status::current_timestamp;
use crate::session::SessionStore;
use crate::workflow::engine::WorkflowEngine;
use crate::workflow::pending_command::{PendingCommandStore, DEFAULT_PENDING_TTL_SECS};
use crate::workflow::pending_command_dispatcher::process_pending_command_entry;

/// pending command watcher を起動し、新規 entry を engine に dispatch するバックグ
/// ラウンドタスクを spawn する。
///
/// production 経路では `tauri::Builder::setup` から呼ぶ。watcher 自体（OS file
/// notify）は spawn された tokio runtime 上に閉じ、ハンドルは外部に返さない
/// （アプリ終了時の drop は tauri runtime に任せる）。
pub fn spawn_pending_command_watcher<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
    data_dir: std::path::PathBuf,
) {
    let store = Arc::new(PendingCommandStore::new(&data_dir));
    let store_for_watcher = store.clone();
    let store_for_processor = store.clone();

    // 起動時 cleanup: TTL 超過の古い pending を engine 到達対象から外す（spec [06]
    // TTL / cleanup 境界: 「起動時 cleanup」「watcher pickup 時 age check」の両方を取る）。
    if let Err(e) = store.cleanup_expired(current_timestamp(), DEFAULT_PENDING_TTL_SECS) {
        log::warn!("pending command initial cleanup failed: {e}");
    }

    // pending ディレクトリは事前に owner-only で作っておく（watcher が watch できる土台）。
    if let Err(e) = store.ensure_dirs() {
        log::error!(
            "Failed to prepare pending command directory {}: {e}",
            store.pending_dir().display()
        );
        return;
    }

    let (tx, rx) = mpsc::unbounded_channel::<()>();

    // notify debouncer は OS の file watch を別スレッドで持つ。tokio タスクへは
    // `mpsc` で「再スキャンせよ」のシグナルだけを送る。
    let debouncer_result = new_debouncer(
        Duration::from_millis(200),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| {
            match res {
                Ok(_events) => {
                    let _ = tx.send(());
                }
                Err(e) => {
                    log::warn!("pending command watcher error: {e:?}");
                }
            }
        },
    );

    let mut debouncer = match debouncer_result {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create pending command debouncer: {e}");
            return;
        }
    };

    if let Err(e) = debouncer
        .watcher()
        .watch(store_for_watcher.pending_dir(), RecursiveMode::NonRecursive)
    {
        log::error!(
            "Failed to watch pending command directory {}: {e}",
            store_for_watcher.pending_dir().display()
        );
        return;
    }

    // debouncer は drop されるまで OS watch を保つ。tokio タスクが debouncer の
    // 所有権を握り、アプリ終了時に runtime が落ちるタイミングで drop される。
    let task = async move {
        // 起動時 1 回スキャンする（アプリ非稼働中に書き込まれた pending を pickup する）。
        process_pending_pickup(&app, store_for_processor.as_ref()).await;

        let _retained_debouncer = debouncer; // keep alive for the task lifetime
        let mut rx = rx;
        let mut rescan_interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                received = rx.recv() => {
                    if received.is_none() {
                        break;
                    }
                    // burst を 1 サイクルに集約する: 残っているシグナルを drain。
                    while rx.try_recv().is_ok() {}
                }
                _ = rescan_interval.tick() => {}
            }
            process_pending_pickup(&app, store_for_processor.as_ref()).await;
        }
    };
    #[cfg(test)]
    tokio::spawn(task);
    #[cfg(not(test))]
    tauri::async_runtime::spawn(task);
}

/// pickup 1 サイクル分の処理: TTL cleanup → list_pending → dispatcher adapter へ受け渡し。
async fn process_pending_pickup<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    store: &PendingCommandStore,
) {
    if let Err(e) = store.cleanup_expired(current_timestamp(), DEFAULT_PENDING_TTL_SECS) {
        log::warn!("pending command cleanup_expired failed: {e}");
    }
    if let Err(e) =
        store.requeue_unexpired_processing(current_timestamp(), DEFAULT_PENDING_TTL_SECS)
    {
        log::warn!("pending command processing orphan requeue failed: {e}");
    }

    let entries = match store.list_pending() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("pending command list_pending failed: {e}");
            return;
        }
    };

    if entries.is_empty() {
        return;
    }

    let engine = match app.try_state::<Arc<WorkflowEngine>>() {
        Some(s) => s.inner().clone(),
        None => {
            log::warn!("pending command pickup skipped: WorkflowEngine state not available");
            return;
        }
    };
    let session_store = match app.try_state::<Arc<SessionStore>>() {
        Some(s) => s.inner().clone(),
        None => {
            log::warn!("pending command pickup skipped: SessionStore state not available");
            return;
        }
    };
    let handles = match app.try_state::<Arc<Mutex<AgentProcessMap>>>() {
        Some(s) => s.inner().clone(),
        None => {
            log::warn!("pending command pickup skipped: AgentProcessMap state not available");
            return;
        }
    };

    for entry in entries {
        process_pending_command_entry(app, &engine, &session_store, &handles, store, entry).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::event::WorkflowEvent;
    use crate::workflow::log::WorkflowEventLog;
    use crate::workflow::pending_command::{CliRequestPayload, PendingCommand};
    use tempfile::TempDir;

    fn make_app() -> tauri::App<tauri::test::MockRuntime> {
        let data_dir =
            std::env::temp_dir().join(format!("releash-pending-watcher-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        tauri::test::mock_builder()
            .manage(crate::session::TestDataDir(data_dir))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    fn make_managed_app(
        data_dir: &std::path::Path,
        engine: Arc<WorkflowEngine>,
        session_store: Arc<SessionStore>,
        handles: Arc<Mutex<AgentProcessMap>>,
    ) -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .manage(crate::session::TestDataDir(data_dir.to_path_buf()))
            .manage(engine)
            .manage(session_store)
            .manage(handles)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    #[tokio::test]
    async fn pickup_does_not_requeue_fresh_processing_claim_before_dispatch_lookup() {
        let app = make_app();
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let command = PendingCommand::new(
            uuid::Uuid::new_v4().to_string(),
            CliRequestPayload::Abort { node_name: None },
            current_timestamp(),
        );
        store.write_pending(&command).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        assert!(claimed.entry.path.exists());

        process_pending_pickup(app.handle(), &store).await;

        let entries = store.list_pending().unwrap();
        assert!(entries.is_empty());
        assert!(claimed.entry.path.exists());
    }

    #[tokio::test]
    async fn pickup_does_not_requeue_expired_processing_orphan() {
        let app = make_app();
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let command = PendingCommand::new(
            uuid::Uuid::new_v4().to_string(),
            CliRequestPayload::Abort { node_name: None },
            current_timestamp() - DEFAULT_PENDING_TTL_SECS - 10.0,
        );
        store.write_pending(&command).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        assert!(claimed.entry.path.exists());

        process_pending_pickup(app.handle(), &store).await;

        assert!(store.list_pending().unwrap().is_empty());
        assert!(claimed.entry.path.exists());
    }

    #[tokio::test]
    async fn pickup_dispatches_pending_file_records_event_and_removes_from_queue() {
        let data_dir = TempDir::new().unwrap();
        let pending_dir = TempDir::new().unwrap();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        engine
            .set_run_store_data_dir(data_dir.path().to_path_buf())
            .await;
        let session_store = Arc::new(SessionStore::default());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let parent_session = crate::session::create_session_internal_with_permission(
            &session_store,
            data_dir.path(),
            "/wt/pending-pickup-production-path",
            None,
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        engine
            .insert_test_running_execution_for_pending_pickup(
                &run_id,
                "/wt/pending-pickup-production-path",
                &parent_session.id,
            )
            .await;
        let app = make_managed_app(
            data_dir.path(),
            engine.clone(),
            session_store.clone(),
            handles.clone(),
        );
        let store = PendingCommandStore::new(pending_dir.path());
        let command = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Abort { node_name: None },
            current_timestamp(),
        );
        store.write_pending(&command).unwrap();

        process_pending_pickup(app.handle(), &store).await;

        assert!(
            store.list_pending().unwrap().is_empty(),
            "accepted command must be removed from pending queue"
        );
        let events = WorkflowEventLog::new(data_dir.path())
            .read_log(&run_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::RunAborted { .. })));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::CliMutationRequested {
                    request_id: id,
                    ..
                } if id == &command.id
            )
        }));
    }

    #[tokio::test]
    async fn spawned_watcher_picks_up_pending_file_event_and_dispatches() {
        let data_dir = TempDir::new().unwrap();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        engine
            .set_run_store_data_dir(data_dir.path().to_path_buf())
            .await;
        let session_store = Arc::new(SessionStore::default());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let parent_session = crate::session::create_session_internal_with_permission(
            &session_store,
            data_dir.path(),
            "/wt/pending-watcher-file-event",
            None,
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        engine
            .insert_test_running_execution_for_pending_pickup(
                &run_id,
                "/wt/pending-watcher-file-event",
                &parent_session.id,
            )
            .await;
        let app = make_managed_app(
            data_dir.path(),
            engine.clone(),
            session_store.clone(),
            handles.clone(),
        );
        let store = PendingCommandStore::new(data_dir.path());

        spawn_pending_command_watcher(app.handle().clone(), data_dir.path().to_path_buf());
        tokio::time::sleep(Duration::from_millis(300)).await;

        let command = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Abort { node_name: None },
            current_timestamp(),
        );
        store.write_pending(&command).unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let events = WorkflowEventLog::new(data_dir.path())
                .read_log(&run_id)
                .unwrap_or_default();
            let dispatched = events.iter().any(|event| {
                matches!(
                    event,
                    WorkflowEvent::CliMutationRequested {
                        request_id: id,
                        ..
                    } if id == &command.id
                )
            });
            if dispatched && store.list_pending().unwrap().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "spawned watcher did not dispatch pending command before timeout"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    async fn spawned_watcher_initial_scan_picks_up_existing_pending_file() {
        let data_dir = TempDir::new().unwrap();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        engine
            .set_run_store_data_dir(data_dir.path().to_path_buf())
            .await;
        let session_store = Arc::new(SessionStore::default());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let parent_session = crate::session::create_session_internal_with_permission(
            &session_store,
            data_dir.path(),
            "/wt/pending-watcher-initial-scan",
            None,
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        engine
            .insert_test_running_execution_for_pending_pickup(
                &run_id,
                "/wt/pending-watcher-initial-scan",
                &parent_session.id,
            )
            .await;
        let app = make_managed_app(
            data_dir.path(),
            engine.clone(),
            session_store.clone(),
            handles.clone(),
        );
        let store = PendingCommandStore::new(data_dir.path());
        let command = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Abort { node_name: None },
            current_timestamp(),
        );
        store.write_pending(&command).unwrap();

        spawn_pending_command_watcher(app.handle().clone(), data_dir.path().to_path_buf());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let events = WorkflowEventLog::new(data_dir.path())
                .read_log(&run_id)
                .unwrap_or_default();
            let cli_requested = events.iter().any(|event| {
                matches!(
                    event,
                    WorkflowEvent::CliMutationRequested {
                        request_id: id,
                        ..
                    } if id == &command.id
                )
            });
            let aborted = events
                .iter()
                .any(|event| matches!(event, WorkflowEvent::RunAborted { .. }));
            if cli_requested && aborted && store.list_pending().unwrap().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "spawned watcher initial scan did not dispatch existing pending command before timeout"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
