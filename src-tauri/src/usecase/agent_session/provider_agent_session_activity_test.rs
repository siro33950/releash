use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{ProviderAgentSessionActivityUsecase, ProviderAgentSessionChangeNotifier};
use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::{
    TerminalActivity, TerminalSurfaceOwner, TERMINAL_ACTIVITY_RUNNING_WINDOW,
};

#[derive(Default)]
struct RecordingNotifier {
    notified: Mutex<Vec<String>>,
}

impl ProviderAgentSessionChangeNotifier for RecordingNotifier {
    fn provider_agent_session_changed(&self, worktree_path: &str) {
        self.notified
            .lock()
            .unwrap()
            .push(worktree_path.to_string());
    }
}

struct SessionSurfaceLookup {
    session_key: String,
    worktree_path: String,
    lookups: Mutex<usize>,
}

impl ProviderAgentTerminalObservationGateway for SessionSurfaceLookup {
    fn owner_for_runtime_generation(
        &self,
        _session_key: &str,
        _runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner> {
        None
    }

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)> {
        Vec::new()
    }

    fn session_exit_code(&self, _owner: &TerminalSurfaceOwner) -> Option<i32> {
        None
    }

    fn session_activity(&self, _owner: &TerminalSurfaceOwner) -> TerminalActivity {
        TerminalActivity::Idle
    }

    fn session_worktree_path(&self, session_key: &str) -> Option<String> {
        *self.lookups.lock().unwrap() += 1;
        (session_key == self.session_key).then(|| self.worktree_path.clone())
    }
}

struct ActivityFixture {
    usecase: ProviderAgentSessionActivityUsecase,
    notifier: Arc<RecordingNotifier>,
    terminal: Arc<SessionSurfaceLookup>,
}

fn fixture(session_key: &str, worktree_path: &str) -> ActivityFixture {
    let notifier = Arc::new(RecordingNotifier::default());
    let terminal = Arc::new(SessionSurfaceLookup {
        session_key: session_key.to_string(),
        worktree_path: worktree_path.to_string(),
        lookups: Mutex::new(0),
    });
    ActivityFixture {
        usecase: ProviderAgentSessionActivityUsecase::new(
            terminal.clone(),
            notifier.clone(),
            tokio::runtime::Handle::current(),
        ),
        notifier,
        terminal,
    }
}

#[tokio::test(start_paused = true)]
async fn test_provider_agent_session_activity_idleからrunningエッジで通知する() {
    let ActivityFixture {
        usecase,
        notifier,
        terminal,
    } = fixture("session-key", "/repo/worktree");

    usecase.observe_output("session-key");

    assert_eq!(
        notifier.notified.lock().unwrap().as_slice(),
        &["/repo/worktree"]
    );

    usecase.observe_output("session-key");
    assert_eq!(
        notifier.notified.lock().unwrap().len(),
        1,
        "running継続中の出力では再通知しない"
    );
    assert_eq!(
        *terminal.lookups.lock().unwrap(),
        1,
        "worktree解決は追跡エントリ生成時の1回だけ"
    );
}

#[tokio::test(start_paused = true)]
async fn test_provider_agent_session_activity_無出力3秒のdebounceでidle遷移を通知する() {
    let ActivityFixture {
        usecase, notifier, ..
    } = fixture("session-key", "/repo/worktree");

    usecase.observe_output("session-key");
    tokio::time::sleep(TERMINAL_ACTIVITY_RUNNING_WINDOW + Duration::from_millis(10)).await;
    tokio::task::yield_now().await;

    assert_eq!(
        notifier.notified.lock().unwrap().as_slice(),
        &["/repo/worktree", "/repo/worktree"],
        "running遷移とidle遷移で1回ずつ通知する"
    );
}

#[tokio::test(start_paused = true)]
async fn test_provider_agent_session_activity_出力継続中はdebounceをリセットしてidle遷移しない() {
    let ActivityFixture {
        usecase, notifier, ..
    } = fixture("session-key", "/repo/worktree");

    usecase.observe_output("session-key");
    tokio::time::sleep(TERMINAL_ACTIVITY_RUNNING_WINDOW / 2).await;
    usecase.observe_output("session-key");
    tokio::time::sleep(TERMINAL_ACTIVITY_RUNNING_WINDOW / 2).await;
    tokio::task::yield_now().await;

    assert_eq!(
        notifier.notified.lock().unwrap().len(),
        1,
        "最初のrunning遷移以外は通知しない"
    );

    tokio::time::sleep(TERMINAL_ACTIVITY_RUNNING_WINDOW).await;
    tokio::task::yield_now().await;
    assert_eq!(
        notifier.notified.lock().unwrap().len(),
        2,
        "最後の出力から3秒経過でidle遷移を通知する"
    );
}

#[tokio::test(start_paused = true)]
async fn test_provider_agent_session_activity_session所有でないsurfaceは追跡しない() {
    let ActivityFixture {
        usecase,
        notifier,
        terminal,
    } = fixture("session-key", "/repo/worktree");

    usecase.observe_output("workspace-key");
    usecase.observe_output("workspace-key");

    assert!(notifier.notified.lock().unwrap().is_empty());
    assert_eq!(
        *terminal.lookups.lock().unwrap(),
        1,
        "session所有でない判定は初回だけ照会して以後は追跡から除外する"
    );
}

#[tokio::test(start_paused = true)]
async fn test_provider_agent_session_activity_running中のexitはidle遷移として通知する() {
    let ActivityFixture {
        usecase, notifier, ..
    } = fixture("session-key", "/repo/worktree");

    usecase.observe_output("session-key");
    usecase.observe_exit("session-key");

    assert_eq!(
        notifier.notified.lock().unwrap().as_slice(),
        &["/repo/worktree", "/repo/worktree"]
    );

    tokio::time::sleep(TERMINAL_ACTIVITY_RUNNING_WINDOW + Duration::from_millis(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        notifier.notified.lock().unwrap().len(),
        2,
        "exit後のdebounce watcherは再通知しない"
    );
}
