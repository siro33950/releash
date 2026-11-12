use std::sync::Arc;

use crate::common::retry::{self, AttemptProgress, RetryBackoff, RetryLimiter};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionLifecycleEvent};
use crate::domain::workflow::WorkflowError;
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};
use crate::domain::workspace_tree::{WorkspaceIdentity, WorkspaceTreeRepository};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionUsecase};

/// Session の増減に伴うタブ配置の取得、変更、保存、通知を行う。
#[derive(Clone)]
pub struct SessionTabsUsecase {
    repository: Arc<dyn WorkspaceStateRepository>,
    trees: Arc<dyn WorkspaceTreeRepository>,
    publisher: StateSubscriptionUsecase,
}

impl SessionTabsUsecase {
    /// タブ配置と実行木の取得先、および配置変更の通知先を持つ。
    pub fn new(
        repository: Arc<dyn WorkspaceStateRepository>,
        trees: Arc<dyn WorkspaceTreeRepository>,
        publisher: StateSubscriptionUsecase,
    ) -> Self {
        Self {
            repository,
            trees,
            publisher,
        }
    }

    /// Session のタブを追加して保存し、配置の購読へ通知する。
    pub async fn add_session(&self, session: &AgentSession) -> Result<(), WorkspaceStateError> {
        self.update(session.workspace().as_str(), |state| {
            state.add_session_tab(session.id(), session.tree_location().node_execution_id());
        })
        .await
    }

    /// 削除する Session のタブを除いて保存し、配置の購読へ通知する。
    pub async fn remove_session(&self, session: &AgentSession) -> Result<(), WorkspaceStateError> {
        self.update(session.workspace().as_str(), |state| {
            state.remove_session_tab(session.id());
        })
        .await
    }

    /// Session に起きた事実にタブ配置を追従させて保存し、配置の購読へ通知する。
    /// タブ配置を変える事実（`WorkspaceState::follows_session_event`）が無ければ、タブ配置を読み込まない。
    pub async fn follow_session(
        &self,
        session: &AgentSession,
        events: &[AgentSessionLifecycleEvent],
    ) -> Result<(), WorkspaceStateError> {
        if !events.iter().any(WorkspaceState::follows_session_event) {
            return Ok(());
        }
        self.update(session.workspace().as_str(), |state| {
            for event in events {
                state.follow_session(session, event);
            }
        })
        .await
    }

    /// Restore した実行木の Session タブを追加して保存し、配置の購読へ通知する。
    pub async fn restore_execution(
        &self,
        workspace: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let tree = self
            .trees
            .load_execution_tree(&WorkspaceIdentity::new(workspace), execution_id)
            .await?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.into()))?;
        self.update(workspace, |state| state.add_execution_tabs(&tree))
            .await
            .map_err(workflow_error)
    }

    /// Archive する実行木の Session タブを除いて保存し、配置の購読へ通知する。
    pub async fn archive_execution(
        &self,
        workspace: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let tree = self
            .trees
            .load_execution_tree(&WorkspaceIdentity::new(workspace), execution_id)
            .await?
            .ok_or_else(|| WorkflowError::NotFound(execution_id.into()))?;
        self.update(workspace, |state| state.remove_execution_tabs(&tree))
            .await
            .map_err(workflow_error)
    }

    async fn update(
        &self,
        workspace: &str,
        change: impl Fn(&mut WorkspaceState),
    ) -> Result<(), WorkspaceStateError> {
        let changed = retry::attempts(
            RetryBackoff::CONFLICT,
            &RetryLimiter::new(),
            conflict_progress,
            |_| async {
                let (version, saved) = self.repository.read(workspace).await?;
                let mut state = saved.unwrap_or_default().initialize_panes();
                let before = state.clone();
                change(&mut state);
                if state == before {
                    return Ok(false);
                }
                self.repository.save(workspace, state, version).await?;
                Ok(true)
            },
        )
        .await?;
        if changed {
            self.publisher
                .notify(StateChangeSource::WorkspaceState(workspace.into()));
        }
        Ok(())
    }
}

fn conflict_progress(error: &WorkspaceStateError) -> Option<AttemptProgress> {
    super::usecase::conflict_progress(error)
}

fn workflow_error(error: WorkspaceStateError) -> WorkflowError {
    match error {
        WorkspaceStateError::Technical(error) => WorkflowError::Technical(error),
        WorkspaceStateError::VersionConflict => WorkflowError::Conflict(error.to_string()),
        WorkspaceStateError::InvalidLayout => WorkflowError::CorruptStoredState(error.to_string()),
    }
}

#[cfg(test)]
#[path = "session_tabs_test.rs"]
mod session_tabs_tests;
