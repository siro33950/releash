//! `<app_data_dir>/review-comments/` ディレクトリの file watcher。
//!
//! CLI (`releash review create` / `releash review comment` 等) は独立プロセスで
//! `tauri::AppHandle` を持たないため、Tauri コマンド経由の `app.emit` で
//! デスクトップ UI へ変更通知することができない。代わりに本 watcher が
//! `review-comments` 配下の `*.events.json` の変更を検知し、デスクトップ側で
//! `review-comments-changed` イベントを発火することで、CLI 経由・Agent 経由・
//! 外部プロセス経由いずれの書き込みも UI に反映されるようにする。
//!
//! payload はワイルドカード `"*"` 固定。フロントエンド (`useDiffComments`)
//! 側で payload が `"*"` または自分の `worktreeName` に一致するときに
//! reload する仕様。watcher は worktree 名を逆引きしない。
//!
//! debounce は 500ms。`src-tauri/src/watcher.rs` の `start_git_dir_watching`
//! と同じ値を採用している。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use tauri::Emitter;

fn review_events_signature(dir: &Path) -> Vec<(String, u64, Option<SystemTime>)> {
    let mut signature = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return signature;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.ends_with(".events.json") {
            continue;
        }

        let metadata = entry.metadata().ok();
        let len = metadata.as_ref().map_or(0, std::fs::Metadata::len);
        let modified = metadata.and_then(|m| m.modified().ok());
        signature.push((file_name.to_string(), len, modified));
    }

    signature.sort_by(|a, b| a.0.cmp(&b.0));
    signature
}

/// `review-comments` ディレクトリの file watcher を起動する。
///
/// production 経路では `tauri::Builder::setup` から呼ぶ。watcher (OS file
/// notify) は spawn されたタスク上に閉じ、ハンドルは外部に返さない（アプリ
/// 終了時の drop は tauri runtime に任せる）。
///
/// `app_data_dir` 配下に `review-comments/` が無ければ作成する。作成に失敗
/// したり debouncer の生成に失敗した場合は watcher は spawn せず警告ログを
/// 出すのみ（既存 Tauri コマンド経由の `emit_changed` は引き続き機能する）。
pub fn spawn_review_comments_watcher<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
    app_data_dir: PathBuf,
) {
    let dir = super::state_dir(&app_data_dir);

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!(
            "Failed to prepare review-comments directory {}: {e}",
            dir.display()
        );
        return;
    }

    let emit_app = app.clone();
    let watched_dir = dir.clone();
    let debouncer_result = new_debouncer(
        Duration::from_millis(500),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| match res {
            Ok(events) => {
                // `*.events.json` の変更のみを emit 対象にする（lock ファイル等の
                // 副次更新は無視）。
                let relevant = events.iter().any(|event| {
                    event.path == watched_dir
                        || event
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.ends_with(".events.json"))
                });
                if !relevant {
                    return;
                }
                if let Err(e) = emit_app.emit("review-comments-changed", "*") {
                    log::warn!("Failed to emit review-comments-changed: {e}");
                }
            }
            Err(e) => {
                log::warn!("review-comments watcher error: {e:?}");
            }
        },
    );

    let mut debouncer = match debouncer_result {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create review-comments debouncer: {e}");
            return;
        }
    };

    if let Err(e) = debouncer.watcher().watch(&dir, RecursiveMode::NonRecursive) {
        log::error!(
            "Failed to watch review-comments directory {}: {e}",
            dir.display()
        );
        return;
    }

    // debouncer は drop されるまで OS watch を保つ。tokio タスクが debouncer の
    // 所有権を握り、アプリ終了時に runtime が落ちるタイミングで drop される。
    let poll_app = app.clone();
    let poll_dir = dir.clone();
    let task = async move {
        let _retained_debouncer = debouncer;
        let mut last_signature = review_events_signature(&poll_dir);
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            interval.tick().await;
            let next_signature = review_events_signature(&poll_dir);
            if next_signature != last_signature {
                last_signature = next_signature;
                if let Err(e) = poll_app.emit("review-comments-changed", "*") {
                    log::warn!("Failed to emit review-comments-changed from poll fallback: {e}");
                }
            }
        }
    };
    #[cfg(test)]
    tokio::spawn(task);
    #[cfg(not(test))]
    tauri::async_runtime::spawn(task);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::Listener;
    use tempfile::TempDir;

    fn make_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock app must build")
    }

    /// listener コールバックは notify-rs debouncer の同期スレッド経由で
    /// emit される可能性があるため、tokio runtime 非依存の `std::sync::Mutex`
    /// で集めること。
    fn install_listener(
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
    ) -> Arc<Mutex<Vec<String>>> {
        let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let received_for_listener = received.clone();
        app.listen("review-comments-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });
        received
    }

    /// Rule: `review-comments/` 配下の `*.events.json` 変更を検知すると
    /// `review-comments-changed` イベントが payload `"*"` で発火する。
    #[tokio::test]
    async fn emits_review_comments_changed_when_events_json_is_written() {
        let data_dir = TempDir::new().unwrap();
        let app = make_app();
        let received = install_listener(app.handle());

        app.emit("review-comments-changed", "*").unwrap();
        assert!(
            !received.lock().unwrap().is_empty(),
            "mock app listener must receive direct review-comments-changed emit"
        );
        received.lock().unwrap().clear();

        spawn_review_comments_watcher(app.handle().clone(), data_dir.path().to_path_buf());
        // watcher の watch 開始が反映されるまで少し待つ
        tokio::time::sleep(Duration::from_millis(100)).await;

        let target = super::super::state_dir(data_dir.path()).join("dummy.events.json");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        let mut attempt = 0usize;
        loop {
            if !received.lock().unwrap().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "watcher did not emit review-comments-changed within deadline"
            );
            std::fs::write(&target, format!("[{attempt}]")).unwrap();
            attempt += 1;
            tokio::time::sleep(Duration::from_millis(700)).await;
        }

        let payloads = received.lock().unwrap().clone();
        // Tauri の event payload は JSON 文字列としてシリアライズされるため `"\"*\""` になる。
        assert!(
            payloads.iter().any(|p| p == "\"*\"" || p == "*"),
            "expected wildcard payload, got {payloads:?}"
        );
    }

    /// Rule: `*.events.json` 以外のファイル変更（例: `.lock` ファイル）は
    /// emit 対象にしない。
    #[tokio::test]
    async fn ignores_non_events_json_changes() {
        let data_dir = TempDir::new().unwrap();
        let app = make_app();
        let received = install_listener(app.handle());

        spawn_review_comments_watcher(app.handle().clone(), data_dir.path().to_path_buf());
        tokio::time::sleep(Duration::from_millis(100)).await;

        let target = super::super::state_dir(data_dir.path()).join("dummy.events.lock");
        std::fs::write(&target, b"lock").unwrap();

        // debounce 後にも emit されないことを確認する。
        tokio::time::sleep(Duration::from_millis(900)).await;
        let payloads = received.lock().unwrap().clone();
        assert!(
            payloads.is_empty(),
            "expected no emit for non-events.json change, got {payloads:?}"
        );
    }
}
