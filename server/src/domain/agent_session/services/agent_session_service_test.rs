use std::sync::Mutex;

use super::*;
use crate::domain::agent_session::aggregates::{AgentSessionTreeLocation, ManagedPtyPresence};
use crate::domain::comment::{Author, Comment, Resolve, State, Target};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;

struct RecordingTerminal {
    presence: Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError>,
    input_error: Option<ProviderAgentTerminalGatewayError>,
    presence_owners: Mutex<Vec<TerminalSurfaceOwner>>,
    writes: Mutex<Vec<(TerminalSurfaceOwner, String)>>,
}

fn terminal(presence: ManagedPtyPresence) -> RecordingTerminal {
    RecordingTerminal {
        presence: Ok(presence),
        input_error: None,
        presence_owners: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
    }
}

impl ProviderAgentTerminalGateway for RecordingTerminal {
    fn spawn(
        &self,
        _owner: TerminalSurfaceOwner,
        _worktree_path: &str,
        _process: TerminalProcessLaunch,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        panic!("unexpected spawn")
    }

    fn presence(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError> {
        self.presence_owners.lock().unwrap().push(owner.clone());
        self.presence.clone()
    }

    fn stop_preserving_checkpoint(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        panic!("unexpected stop")
    }

    fn delete(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        panic!("unexpected delete")
    }

    fn is_current_runtime_generation(
        &self,
        _owner: &TerminalSurfaceOwner,
        _runtime_generation: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError> {
        panic!("unexpected runtime generation check")
    }
}

impl ProviderAgentTerminalInputGateway for RecordingTerminal {
    fn write_instruction(
        &self,
        owner: &TerminalSurfaceOwner,
        instruction: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.writes
            .lock()
            .unwrap()
            .push((owner.clone(), instruction.to_string()));
        match &self.input_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

fn session() -> AgentSession {
    AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        AgentSessionTreeLocation::session_tree_root("session").unwrap(),
    )
    .unwrap()
}

fn thread(id: &str, target: Target) -> Thread {
    Thread {
        id: id.into(),
        worktree_name: "/repo".into(),
        target,
        state: State::Open,
        comments: vec![
            Comment {
                id: format!("comment-{id}"),
                thread_id: id.into(),
                author: Author::human(),
                content: format!("本文-{id}\n**確認**"),
                created_at: 1.0,
            },
            Comment {
                id: format!("reply-{id}"),
                thread_id: id.into(),
                author: Author::provider_agent("codex".into(), Some("session".into())),
                content: "後続Comment".into(),
                created_at: 2.0,
            },
        ],
        resolve: None,
        created_at: 1.0,
        updated_at: 2.0,
        version: 2,
        can_resolve: true,
    }
}

fn target() -> Target {
    Target {
        file_path: None,
        line_number: None,
        end_line: None,
        side: None,
        end_side: None,
    }
}

#[test]
fn test_thread送信_open以外のsessionには入力しない() {
    // Given
    for (presence, archived) in [
        (ManagedPtyPresence::ConfirmedAbsent, false),
        (ManagedPtyPresence::Unknown, false),
        (ManagedPtyPresence::Live, true),
    ] {
        let mut session = session();
        if archived {
            session.archive().unwrap();
        }
        let terminal = Arc::new(terminal(presence));
        let service = AgentSessionService::new(terminal.clone(), terminal.clone());
        // When
        let result = service.send_thread(&session, vec![thread("one", target())]);
        // Then
        assert_eq!(
            result,
            Err(AgentSessionSendThreadError::Instruction(
                AgentSessionInstructionError::NotOpen
            ))
        );
        assert!(terminal.writes.lock().unwrap().is_empty());
    }
}

#[test]
fn test_thread送信_後続のthreadが別workspaceなら全件入力しない() {
    // Given
    let session = session();
    let terminal = Arc::new(terminal(ManagedPtyPresence::Live));
    let service = AgentSessionService::new(terminal.clone(), terminal.clone());
    let mut outside = thread("outside", target());
    outside.worktree_name = session.worktree_path().into();
    // When
    let result = service.send_thread(&session, vec![thread("one", target()), outside]);
    // Then
    assert_eq!(
        result,
        Err(AgentSessionSendThreadError::ThreadOutsideWorkspace(
            "outside".into()
        ))
    );
    assert!(terminal.writes.lock().unwrap().is_empty());
}

#[test]
fn test_thread送信_一件の位置と本文と確認方法を一回で入力する() {
    // Given
    for (file_path, line_number, end_line, location) in [
        (None, None, None, "General"),
        (Some("src/a.rs"), None, None, "src/a.rs"),
        (Some("src/a.rs"), Some(2), None, "src/a.rs:2"),
        (Some("src/a.rs"), Some(2), Some(4), "src/a.rs:2-4"),
    ] {
        let session = session();
        let terminal = Arc::new(terminal(ManagedPtyPresence::Live));
        let service = AgentSessionService::new(terminal.clone(), terminal.clone());
        let thread = thread(
            "one",
            Target {
                file_path: file_path.map(str::to_string),
                line_number,
                end_line,
                ..target()
            },
        );
        // When
        service.send_thread(&session, vec![thread]).unwrap();
        // Then
        assert_eq!(
            *terminal.presence_owners.lock().unwrap(),
            vec![session.terminal_surface_owner()]
        );
        assert_eq!(
            *terminal.writes.lock().unwrap(),
            vec![(session.terminal_surface_owner(), format!(
                "以下のスレッドの内容を確認してください。\n\n位置: {location}\nThread ID: one\n本文:\n本文-one\n**確認**\n\nそれ以降のCommentは `releash review get --session-id \"$RELEASH_SESSION_ID\" one` で確認してください。"
            ))]
        );
    }
}

#[test]
fn test_thread送信_未解決と解決済みを指定順に一つのプロンプトで送る() {
    // Given
    let session = session();
    let terminal = Arc::new(terminal(ManagedPtyPresence::Live));
    let service = AgentSessionService::new(terminal.clone(), terminal.clone());
    let file = thread(
        "file",
        Target {
            file_path: Some("src/a.rs".into()),
            line_number: Some(3),
            end_line: Some(5),
            ..target()
        },
    );
    let mut general = thread("general", target());
    general.state = State::Resolved;
    general.can_resolve = false;
    general.resolve = Some(Resolve {
        actor: Author::human(),
        outcome: "resolved".into(),
        summary: "完了".into(),
        resolved_at: 3.0,
    });
    // When
    service.send_thread(&session, vec![general, file]).unwrap();
    // Then
    assert_eq!(
        *terminal.writes.lock().unwrap(),
        vec![(session.terminal_surface_owner(), concat!(
            "以下のスレッドの内容を確認してください。\n\n",
            "位置: General\nThread ID: general\n本文:\n本文-general\n**確認**\n\n",
            "それ以降のCommentは `releash review get --session-id \"$RELEASH_SESSION_ID\" general` で確認してください。\n\n",
            "位置: src/a.rs:3-5\nThread ID: file\n本文:\n本文-file\n**確認**\n\n",
            "それ以降のCommentは `releash review get --session-id \"$RELEASH_SESSION_ID\" file` で確認してください。"
        ).into())]
    );
}

#[test]
fn test_thread送信_端末の確認と入力の失敗を保持する() {
    // Given
    for fail_presence in [true, false] {
        let session = session();
        let error = ProviderAgentTerminalGatewayError::StaleAttachment;
        let mut terminal = terminal(ManagedPtyPresence::Live);
        if fail_presence {
            terminal.presence = Err(error.clone());
        } else {
            terminal.input_error = Some(error.clone());
        }
        let terminal = Arc::new(terminal);
        let service = AgentSessionService::new(terminal.clone(), terminal.clone());
        // When
        let result = service.send_thread(&session, vec![thread("one", target())]);
        // Then
        assert_eq!(result, Err(AgentSessionSendThreadError::Terminal(error)));
        assert_eq!(
            terminal.writes.lock().unwrap().len(),
            usize::from(!fail_presence)
        );
    }
}
