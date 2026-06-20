use crate::domain::pty_session::{entities::PtySession, PtyKind};
use crate::usecase::pty_session::dto::{pty_kind_to_wire, GetOrSpawnPtyResult};
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::{PtyBackendSpawnRequest, PtySessionGateway};

#[allow(clippy::too_many_arguments)]
pub fn spawn<G: PtySessionGateway>(
    manager: &G,
    app: &G::AppContext,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: Option<String>,
    label: Option<String>,
    kind: PtyKind,
) -> Result<(u64, String), UsecaseError> {
    let pty_id = manager.next_pty_id();
    let session_key = uuid::Uuid::new_v4().to_string();
    let runtime = manager.spawn_backend(
        app,
        PtyBackendSpawnRequest {
            pty_id,
            rows,
            cols,
            cwd,
            exec_command: None,
        },
    )?;

    manager.insert_session(
        PtySession::new(pty_id, session_key.clone(), worktree_path, label, kind),
        runtime,
    );
    manager.start_output_reader(app, pty_id)?;

    Ok((pty_id, session_key))
}

#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn<G: PtySessionGateway>(
    manager: &G,
    app: &G::AppContext,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    session_key: Option<String>,
    worktree_path: String,
    label: Option<String>,
    kind: PtyKind,
) -> Result<GetOrSpawnPtyResult, UsecaseError> {
    if let Some(key) = &session_key {
        if let Some(found) = manager.find_by_session_key(key) {
            return Ok(GetOrSpawnPtyResult {
                pty_id: found.snapshot.pty_id,
                session_key: found.snapshot.session_key,
                buffered_output: found.buffered_output,
                is_new: false,
                is_exited: found.snapshot.exited,
                exit_code: found.snapshot.exit_code,
                label: found.snapshot.label,
                kind: pty_kind_to_wire(found.snapshot.kind).to_string(),
            });
        }
    }

    let (pty_id, new_session_key) = spawn(
        manager,
        app,
        rows,
        cols,
        cwd,
        Some(worktree_path),
        label.clone(),
        kind,
    )?;

    Ok(GetOrSpawnPtyResult {
        pty_id,
        session_key: new_session_key,
        buffered_output: String::new(),
        is_new: true,
        is_exited: false,
        exit_code: None,
        label,
        kind: pty_kind_to_wire(kind).to_string(),
    })
}
