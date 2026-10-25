use std::sync::Arc;

use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionInstructionError};
use crate::domain::agent_session::{
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalInputGateway,
};
use crate::domain::comment::Thread;

/// Thread の内容を Session へ引き渡せない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionSendThreadError {
    Instruction(AgentSessionInstructionError),
    ThreadOutsideWorkspace(String),
    Terminal(ProviderAgentTerminalGatewayError),
}

/// Session と Thread の関係を確認して指示を引き渡すドメインサービス。
pub(crate) struct AgentSessionService {
    terminal: Arc<dyn ProviderAgentTerminalGateway>,
    input: Arc<dyn ProviderAgentTerminalInputGateway>,
}

impl AgentSessionService {
    /// Session の稼働状態の確認と指示の入力に使う接続先を持つ。
    pub(crate) fn new(
        terminal: Arc<dyn ProviderAgentTerminalGateway>,
        input: Arc<dyn ProviderAgentTerminalInputGateway>,
    ) -> Self {
        Self { terminal, input }
    }

    /// Open の Session に、同じ Workspace に属する Thread を一つの指示として送る。
    pub(crate) fn send_thread(
        &self,
        session: &AgentSession,
        threads: Vec<Thread>,
    ) -> Result<(), AgentSessionSendThreadError> {
        let owner = session.terminal_surface_owner();
        let presence = self
            .terminal
            .presence(&owner)
            .map_err(AgentSessionSendThreadError::Terminal)?;
        session
            .authorize_instruction(presence)
            .map_err(AgentSessionSendThreadError::Instruction)?;
        self.ensure_thread_worktree(session, &threads)?;
        let prompt = Self::handoff_prompt(threads);
        self.input
            .write_instruction(&owner, &prompt)
            .map_err(AgentSessionSendThreadError::Terminal)
    }

    /// 導入文と各 Thread の引き渡し内容から、一つの指示を決める。
    pub(crate) fn handoff_prompt(threads: Vec<Thread>) -> String {
        let mut prompt = "以下のスレッドの内容を確認してください。".to_string();
        for thread in threads {
            prompt.push_str("\n\n");
            prompt.push_str(&thread.handoff_content());
        }
        prompt
    }

    /// すべての Thread が Session と同じ Workspace に属することを確認する。
    fn ensure_thread_worktree(
        &self,
        session: &AgentSession,
        threads: &[Thread],
    ) -> Result<(), AgentSessionSendThreadError> {
        for thread in threads {
            if thread.worktree_name != session.workspace().as_str() {
                return Err(AgentSessionSendThreadError::ThreadOutsideWorkspace(
                    thread.id.clone(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "agent_session_service_test.rs"]
mod agent_session_service_tests;
