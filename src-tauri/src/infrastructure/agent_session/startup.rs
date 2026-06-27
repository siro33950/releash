use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::infrastructure::agent_session::runtime::{CleanupGate, OrphanCleanupReport};

#[cfg(all(unix, test))]
static STARTUP_ORPHAN_CLEANUP_TELEMETRY_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(all(unix, test))]
static STARTUP_ORPHAN_CLEANUP_SUCCESS_TELEMETRY_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(unix)]
fn record_startup_orphan_cleanup(report: &OrphanCleanupReport, failed: bool) {
    crate::other::telemetry::record_orphan_cleanup_counts(
        report.scanned,
        report.processed,
        report.skipped,
        report.failures,
        failed,
    );
    #[cfg(test)]
    {
        let status = crate::other::telemetry::orphan_cleanup_status(report.failures, failed);
        STARTUP_ORPHAN_CLEANUP_TELEMETRY_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if status == crate::other::telemetry::orphan_cleanup_status(0, false) {
            STARTUP_ORPHAN_CLEANUP_SUCCESS_TELEMETRY_CALLS
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[cfg(unix)]
pub(crate) fn spawn_startup_orphan_cleanup<F>(
    data_dir: PathBuf,
    cleanup_gate: Arc<CleanupGate>,
    cleanup_fn: F,
) where
    F: FnOnce(&Path) -> OrphanCleanupReport + Send + 'static,
{
    let thread_gate = Arc::clone(&cleanup_gate);
    let spawn_result = std::thread::Builder::new()
        .name("releash-startup-orphan-cleanup".to_string())
        .spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cleanup_fn(&data_dir)));
            let (report, failed) = match result {
                Ok(report) => (report, false),
                Err(_) => {
                    log::warn!("startup orphan cleanup panicked");
                    (OrphanCleanupReport::default(), true)
                }
            };
            let status =
                crate::other::telemetry::orphan_cleanup_status(report.failures, failed).as_str();
            log::info!(
                "startup orphan cleanup finished status={status} scanned={} processed={} skipped={} failures={}",
                report.scanned,
                report.processed,
                report.skipped,
                report.failures
            );
            record_startup_orphan_cleanup(&report, failed);
            thread_gate.open();
        });
    if let Err(e) = spawn_result {
        let report = OrphanCleanupReport::default();
        log::warn!("failed to start startup orphan cleanup thread: {e}");
        record_startup_orphan_cleanup(&report, true);
        cleanup_gate.open();
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn spawn_startup_orphan_cleanup_is_non_blocking_and_records_after_completion() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::sync::Arc;
        use std::time::Duration;

        let data_dir = tempfile::tempdir().unwrap();
        let gate = Arc::new(crate::infrastructure::agent_session::runtime::CleanupGate::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (returned_tx, returned_rx) = mpsc::channel();
        let calls_for_cleanup = Arc::clone(&calls);
        let gate_for_spawn = Arc::clone(&gate);
        let data_dir_for_spawn = data_dir.path().to_path_buf();
        let telemetry_calls_before =
            super::STARTUP_ORPHAN_CLEANUP_TELEMETRY_CALLS.load(Ordering::SeqCst);
        let success_calls_before =
            super::STARTUP_ORPHAN_CLEANUP_SUCCESS_TELEMETRY_CALLS.load(Ordering::SeqCst);

        std::thread::spawn(move || {
            super::spawn_startup_orphan_cleanup(data_dir_for_spawn, gate_for_spawn, move |_| {
                calls_for_cleanup.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                crate::infrastructure::agent_session::runtime::OrphanCleanupReport {
                    scanned: 1,
                    processed: 0,
                    skipped: 0,
                    failures: 0,
                }
            });
            returned_tx.send(()).unwrap();
        });

        returned_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("startup cleanup launcher must return before cleanup finishes");
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("fake cleanup should start exactly once");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!gate.is_open());

        release_tx.send(()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(1), gate.wait_until_open()).await
            })
            .expect("cleanup completion must open the gate");

        assert_eq!(
            super::STARTUP_ORPHAN_CLEANUP_TELEMETRY_CALLS.load(Ordering::SeqCst),
            telemetry_calls_before + 1
        );
        assert_eq!(
            super::STARTUP_ORPHAN_CLEANUP_SUCCESS_TELEMETRY_CALLS.load(Ordering::SeqCst),
            success_calls_before + 1
        );
    }
}
