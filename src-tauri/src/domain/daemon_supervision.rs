pub(crate) const STARTUP_TIMEOUT_MS: u64 = 30_000;
pub(crate) const QUIT_TIMEOUT_MS: u64 = 15_000;
const RESTORATION_TIMEOUT_MS: u64 = 30_000;
const STABLE_READY_MS: u64 = 60_000;
const RETRY_DELAYS_MS: [u64; 3] = [1_000, 2_000, 4_000];

pub(crate) struct DaemonExit {
    pub success: bool,
    pub shutdown_complete: bool,
    pub reason: String,
}

#[cfg(feature = "desktop")]
#[async_trait::async_trait]
pub(crate) trait DaemonProcessPort: Send + Sync {
    fn monotonic_ms(&self) -> u64;
    async fn wait_for_poll(&self);
    async fn spawn(&self) -> Result<String, String>;
    async fn exited(&self) -> Result<Option<DaemonExit>, String>;
    async fn terminate_and_wait(&self) -> Result<(), String>;
    async fn request_shutdown(&self, intent: StopIntent) -> Result<ShutdownResponse, String>;
}

#[cfg(feature = "desktop")]
#[async_trait::async_trait]
pub(crate) trait DesktopUpdateInstaller: Send + Sync {
    async fn download(&self) -> Result<(), String>;
    async fn install(&self) -> Result<(), String>;
    fn restart(&self) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownResponse {
    Accepted,
    DecisionRequired(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Starting,
    Restoring,
    Ready,
    Backoff,
    Failed,
    Stopping,
    Installing,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureStage {
    Spawn,
    Initialization,
    StartupTimeout,
    UnexpectedExit,
    Identity,
    Shutdown,
    Update,
    Restart,
    Restoration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub stage: FailureStage,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopIntent {
    Quit(i32),
    Restart,
    Update,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesktopAction {
    Wait,
    Show,
    Ready { show_window: bool },
    Exit(i32),
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellOperation {
    RestoreState,
    Supervision,
    ApplySettings,
    Normal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientOperation {
    Recovery,
    RestoreState,
    Normal,
    Shutdown,
}

pub(crate) enum StartupInterruption {
    Identity(Failure),
    Deadline(Option<Failure>),
}

pub(crate) struct DaemonSupervision {
    phase: Phase,
    retries: usize,
    connection_generation: u64,
    restoration_deadline: Option<u64>,
    deadline: u64,
    ready_since: Option<u64>,
    failure: Option<Failure>,
    stop: Option<StopIntent>,
    termination_unconfirmed: bool,
    quit_deadline: Option<u64>,
    quit_finished: bool,
}

impl DaemonSupervision {
    pub fn new(now: u64) -> Self {
        Self {
            phase: Phase::Starting,
            retries: 0,
            connection_generation: 0,
            restoration_deadline: None,
            deadline: now.saturating_add(STARTUP_TIMEOUT_MS),
            ready_since: None,
            failure: None,
            stop: None,
            termination_unconfirmed: false,
            quit_deadline: None,
            quit_finished: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn failure(&self) -> Option<&Failure> {
        self.failure.as_ref()
    }
    pub fn connection_generation(&self) -> u64 {
        self.connection_generation
    }
    pub fn retries(&self) -> usize {
        self.retries
    }
    pub fn stop_intent(&self) -> Option<StopIntent> {
        self.stop
    }
    pub fn connection_admitted(&self) -> bool {
        matches!(self.phase, Phase::Ready | Phase::Restoring)
            || self.restoration_failed()
            || matches!(self.phase, Phase::Stopping | Phase::Failed)
                && self.stop.is_some()
                && self.ready_since.is_some()
    }
    pub fn retry_available(&self) -> bool {
        self.phase == Phase::Failed && self.stop.is_none() && !self.termination_unconfirmed
    }
    pub fn startup_expired(&self, now: u64) -> bool {
        self.phase == Phase::Starting && now >= self.deadline
    }
    pub fn restart_due(&mut self, now: u64) -> bool {
        if self.phase != Phase::Backoff || now < self.deadline {
            return false;
        }
        self.phase = Phase::Starting;
        self.deadline = now.saturating_add(STARTUP_TIMEOUT_MS);
        true
    }
    pub fn connection_lost(&mut self, now: u64) {
        if matches!(self.phase, Phase::Ready | Phase::Restoring) || self.restoration_failed() {
            self.phase = Phase::Starting;
            self.deadline = now.saturating_add(STARTUP_TIMEOUT_MS);
        }
    }
    pub fn connected(&mut self, now: u64) -> bool {
        if self.phase == Phase::Starting && !self.startup_expired(now) {
            self.phase = Phase::Restoring;
            self.connection_generation += 1;
            self.restoration_deadline = None;
            self.ready_since = Some(now);
            return true;
        }
        false
    }
    pub fn restoration_current(&self, generation: u64) -> bool {
        self.phase == Phase::Restoring && self.connection_generation == generation
    }
    pub fn begin_restoration(&mut self, now: u64) {
        if self.phase == Phase::Restoring && self.restoration_deadline.is_none() {
            self.restoration_deadline = Some(now.saturating_add(RESTORATION_TIMEOUT_MS));
        }
    }
    pub fn fail_restoration(&mut self, generation: u64, reason: String) -> bool {
        if !self.restoration_current(generation) {
            return false;
        }
        self.phase = Phase::Failed;
        self.failure = Some(Failure {
            stage: FailureStage::Restoration,
            reason,
        });
        true
    }
    pub fn expire_restoration(&mut self, now: u64) {
        if self.phase == Phase::Restoring
            && self
                .restoration_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            self.fail_restoration(
                self.connection_generation,
                "Desktop state could not be restored within 30 seconds. Retry to reload the current state.".into(),
            );
        }
    }
    pub fn finish_restoration(&mut self, generation: u64, now: u64) -> bool {
        self.expire_restoration(now);
        self.restoration_current(generation) && self.ready(now)
    }
    fn restoration_failed(&self) -> bool {
        self.phase == Phase::Failed
            && self
                .failure
                .as_ref()
                .is_some_and(|failure| failure.stage == FailureStage::Restoration)
    }
    pub fn retry_restoration(&mut self, now: u64) -> bool {
        if !self.retry_available() || !self.restoration_failed() {
            return false;
        }
        self.phase = Phase::Restoring;
        self.connection_generation += 1;
        self.restoration_deadline = Some(now.saturating_add(RESTORATION_TIMEOUT_MS));
        self.failure = None;
        true
    }
    pub fn ready(&mut self, now: u64) -> bool {
        if matches!(self.phase, Phase::Starting | Phase::Restoring) && !self.startup_expired(now) {
            self.phase = Phase::Ready;
            self.ready_since.get_or_insert(now);
            self.failure = None;
            return true;
        }
        false
    }
    pub fn spawn_failed(&mut self, reason: String, now: u64) {
        self.failed_after_exit(
            Failure {
                stage: FailureStage::Spawn,
                reason,
            },
            false,
            now,
        );
    }
    pub fn observe_exit(&mut self, exit: DaemonExit, now: u64) {
        if self.stop.is_some() {
            self.stopped(exit.shutdown_complete || matches!(self.stop, Some(StopIntent::Quit(_))));
        } else {
            self.failed_after_exit(
                Failure {
                    stage: self.exit_stage(),
                    reason: exit.reason,
                },
                !exit.success,
                now,
            );
        }
    }
    pub fn startup_interruption(
        &mut self,
        now: u64,
        last_error: Option<Failure>,
    ) -> Option<StartupInterruption> {
        if self.phase != Phase::Starting {
            return None;
        }
        if let Some(failure) = last_error
            .as_ref()
            .filter(|f| f.stage == FailureStage::Identity)
        {
            self.reject_connection(failure.clone());
            return Some(StartupInterruption::Identity(failure.clone()));
        }
        self.startup_expired(now)
            .then_some(StartupInterruption::Deadline(last_error))
    }
    pub fn startup_terminated(&mut self, interruption: StartupInterruption, now: u64) {
        let (failure, retry) = match interruption {
            StartupInterruption::Identity(failure) => (failure, false),
            StartupInterruption::Deadline(error) => (
                Failure {
                    stage: FailureStage::StartupTimeout,
                    reason: format!(
                        "Daemon did not become ready within 30 seconds. {}",
                        error.map(|f| f.reason).unwrap_or_default()
                    ),
                },
                true,
            ),
        };
        self.failed_after_exit(failure, retry, now);
    }
    pub fn uncoordinated_stop(&mut self) {
        self.stopped(matches!(self.stop, Some(StopIntent::Quit(_))));
    }
    fn failed_after_exit(&mut self, failure: Failure, abnormal_or_timeout: bool, now: u64) {
        if self
            .ready_since
            .take()
            .is_some_and(|since| now.saturating_sub(since) >= STABLE_READY_MS)
        {
            self.retries = 0;
        }
        self.termination_unconfirmed = false;
        self.failure = Some(failure);
        if self.stop.is_some() {
            self.phase = Phase::Failed;
        } else if abnormal_or_timeout && self.retries < RETRY_DELAYS_MS.len() {
            self.deadline = now.saturating_add(RETRY_DELAYS_MS[self.retries]);
            self.retries += 1;
            self.phase = Phase::Backoff;
        } else {
            self.phase = Phase::Failed;
        }
    }
    pub fn retry(&mut self, now: u64) -> bool {
        if !self.retry_available() {
            return false;
        }
        let generation = self.connection_generation;
        *self = Self::new(now);
        self.connection_generation = generation;
        true
    }
    pub fn begin_stop(&mut self, intent: StopIntent) -> bool {
        if matches!(self.phase, Phase::Installing | Phase::Stopping) && self.stop.is_some() {
            if matches!(intent, StopIntent::Quit(_)) {
                self.stop = Some(intent);
            }
            return false;
        }
        if self.stop.is_some()
            && !(matches!(self.phase, Phase::Stopped | Phase::Failed)
                && matches!(intent, StopIntent::Quit(_)))
        {
            return false;
        }
        self.stop = Some(intent);
        self.phase = Phase::Stopping;
        true
    }
    fn stopped(&mut self, completed: bool) {
        if completed && self.stop.is_some() {
            self.phase = Phase::Stopped;
            self.failure = None;
        } else {
            self.phase = Phase::Failed;
            self.failure = Some(Failure {
                stage: FailureStage::Shutdown,
                reason: "Daemon shutdown outcome is unknown; switching is blocked.".into(),
            });
        }
    }
    pub fn begin_update_install(&mut self) -> bool {
        if self.phase != Phase::Stopped || self.stop != Some(StopIntent::Update) {
            return false;
        }
        self.phase = Phase::Installing;
        self.failure = None;
        true
    }
    pub fn finish_update_install(&mut self, failure: Option<String>) -> bool {
        if self.quit_finished {
            return false;
        }
        self.phase = Phase::Stopped;
        self.failure = failure.map(|reason| Failure {
            stage: FailureStage::Update,
            reason,
        });
        self.stop == Some(StopIntent::Update) && self.failure.is_none()
    }
    pub fn reject_connection(&mut self, failure: Failure) {
        self.phase = Phase::Stopping;
        self.failure = Some(failure);
    }
    pub fn shutdown_response(&mut self, response: ShutdownResponse) {
        if self.phase == Phase::Stopped {
            return;
        }
        match response {
            ShutdownResponse::Accepted => {}
            ShutdownResponse::DecisionRequired(reason) => {
                self.failure = Some(Failure {
                    stage: FailureStage::Shutdown,
                    reason,
                });
            }
        }
    }
    pub fn stop_failed(&mut self, stage: FailureStage, reason: String) {
        if self.phase == Phase::Stopped && stage == FailureStage::Shutdown {
            return;
        }
        if self.phase != Phase::Stopped {
            self.phase = Phase::Failed;
            self.termination_unconfirmed = true;
        }
        self.failure = Some(Failure { stage, reason });
    }
    pub fn arm_quit_deadline(&mut self, now: u64) {
        if matches!(self.stop, Some(StopIntent::Quit(_))) && self.quit_deadline.is_none() {
            self.quit_deadline = Some(now.saturating_add(QUIT_TIMEOUT_MS));
        }
    }
    pub fn quit_expired(&self, now: u64) -> bool {
        self.phase != Phase::Stopped && self.quit_deadline.is_some_and(|deadline| now >= deadline)
    }
    pub fn finish_quit_termination(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.stopped(true),
            Err(reason) => self.stop_failed(FailureStage::Shutdown, reason),
        }
        self.quit_finished = true;
    }
    pub fn update_stop_result(&self) -> Result<bool, String> {
        if self.stop != Some(StopIntent::Update) {
            return Err("Update cancelled by Quit.".into());
        }
        if self.phase == Phase::Stopped && self.failure.is_none() {
            return Ok(true);
        }
        if let Some(failure) = &self.failure {
            if self.phase == Phase::Failed {
                return Err(failure.reason.clone());
            }
        }
        Ok(false)
    }
    pub fn desktop_action(
        &self,
        hidden: bool,
        first_ready: bool,
        has_failure_window: bool,
    ) -> DesktopAction {
        if self.quit_finished {
            if let Some(StopIntent::Quit(code)) = self.stop {
                return DesktopAction::Exit(code);
            }
        }
        match self.phase {
            Phase::Ready | Phase::Restoring => DesktopAction::Ready {
                show_window: if first_ready {
                    !hidden
                } else {
                    has_failure_window
                },
            },
            Phase::Stopped => match self.stop {
                Some(StopIntent::Quit(code)) => DesktopAction::Exit(code),
                Some(StopIntent::Restart) if self.failure.is_none() => DesktopAction::Restart,
                _ if self.failure.is_some() => DesktopAction::Show,
                _ => DesktopAction::Wait,
            },
            Phase::Starting if hidden => DesktopAction::Wait,
            Phase::Backoff if hidden && first_ready => DesktopAction::Wait,
            Phase::Installing => DesktopAction::Wait,
            _ => DesktopAction::Show,
        }
    }
    fn exit_stage(&self) -> FailureStage {
        if self.ready_since.is_some() {
            FailureStage::UnexpectedExit
        } else {
            FailureStage::Initialization
        }
    }
    pub fn client_command_admitted(&self, operation: ClientOperation) -> bool {
        self.phase == Phase::Ready
            || self.phase == Phase::Restoring
                && matches!(
                    operation,
                    ClientOperation::RestoreState | ClientOperation::Recovery
                )
            || self.stop.is_some() && operation == ClientOperation::Shutdown
    }
    pub fn shell_command_admitted(&self, operation: ShellOperation, connected: bool) -> bool {
        connected
            && (self.phase == Phase::Ready
                || self.phase == Phase::Restoring && operation == ShellOperation::RestoreState
                || self.connection_admitted() && operation == ShellOperation::ApplySettings)
            || operation == ShellOperation::Supervision
    }
}

pub(crate) fn verify_identity(
    expected_launch: &str,
    launch: &str,
    release: &str,
) -> Result<(), Failure> {
    if expected_launch != launch {
        return Err(Failure {
            stage: FailureStage::Identity,
            reason: "The connection belongs to a different daemon instance.".into(),
        });
    }
    if release != env!("CARGO_PKG_VERSION") {
        return Err(Failure {
            stage: FailureStage::Identity,
            reason: format!(
                "Daemon release {release} does not match UI release {}.",
                env!("CARGO_PKG_VERSION")
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "daemon_supervision_test.rs"]
mod daemon_supervision_tests;
