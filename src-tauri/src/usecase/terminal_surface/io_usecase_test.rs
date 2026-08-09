use super::*;
use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
};
use crate::domain::terminal_surface::gateway::{
    TerminalRuntimeSpawnRequest, TerminalSurfaceGatewayError, TerminalSurfaceRepository,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use parking_lot::Mutex;

struct FakePtyGateway {
    writes: Mutex<Vec<(String, String)>>,
}

impl FakePtyGateway {
    fn new() -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl TerminalSurfaceRepository for FakePtyGateway {
    fn find_summary_by_session_key(
        &self,
        _session_key: &str,
    ) -> Option<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        None
    }

    fn list_summaries(
        &self,
    ) -> Vec<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        Vec::new()
    }
}

impl TerminalSurfaceGateway for FakePtyGateway {
    fn next_runtime_generation(&self) -> u64 {
        1
    }

    fn spawn_runtime(
        &self,
        _request: TerminalRuntimeSpawnRequest,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn insert_surface(&self, _surface: TerminalSurface) {}

    fn start_output_reader(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn snapshot(&self, _runtime_generation: u64) -> Option<TerminalSurface> {
        None
    }

    fn select_kill_targets_by_worktree(&self, _worktree_path: &str) -> Vec<u64> {
        Vec::new()
    }

    fn remove_surface(&self, _runtime_generation: u64) -> Option<TerminalSurface> {
        None
    }

    fn reserve_spawn_slot(
        &self,
        session_key: &str,
        _worktree_path: Option<&str>,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError> {
        Ok(TerminalSurfaceSpawnReservation {
            session_key: session_key.to_string(),
            worktree_path: None,
        })
    }

    fn complete_spawn_slot(&self, _reservation: &TerminalSurfaceSpawnReservation) {}

    fn rollback_spawn_slot(&self, _reservation: &TerminalSurfaceSpawnReservation) {}

    fn activate_input_attachment(&self, _session_key: &str, _attachment_id: &str) {}

    fn deactivate_input_attachment(&self, _session_key: &str, _attachment_id: &str) {}

    fn write_attached(
        &self,
        session_key: &str,
        _attachment_id: &str,
        _sequence: u64,
        data: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.write(session_key, data)
    }

    fn write(&self, session_key: &str, data: &str) -> Result<(), TerminalSurfaceGatewayError> {
        self.writes
            .lock()
            .push((session_key.to_string(), data.to_string()));
        Ok(())
    }

    fn resize(
        &self,
        _session_key: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn request_runtime_stop(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn remove_runtime(&self, _runtime_generation: u64) {}
}

#[test]
fn test_ターミナル画面_パス入力_引用符処理して結合後に書き込む() {
    let gateway = FakePtyGateway::new();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();

    write_paths(
        &gateway,
        &owner,
        &[
            "/tmp/a.txt".to_string(),
            "/tmp/my file.txt".to_string(),
            "/tmp/it's.txt".to_string(),
        ],
    )
    .unwrap();

    assert_eq!(
        *gateway.writes.lock(),
        vec![(
            owner.stable_key(),
            "/tmp/a.txt '/tmp/my file.txt' '/tmp/it'\\''s.txt'".to_string()
        )]
    );
}

#[test]
fn test_ターミナル画面_パス入力_空配列では何もしない() {
    let gateway = FakePtyGateway::new();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();

    write_paths(&gateway, &owner, &[]).unwrap();

    assert!(gateway.writes.lock().is_empty());
}
