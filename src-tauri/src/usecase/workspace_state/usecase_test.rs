use super::*;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use parking_lot::Mutex;

struct Store {
    fail: bool,
    calls: Mutex<Vec<String>>,
}
impl WorkspaceStateRepository for Store {
    fn load(&self, _: &str, _: &str) -> Option<WorkspaceState> {
        unreachable!()
    }
    fn set(&self, name: &str, _: WorkspaceState) {
        self.calls.lock().push(format!("set:{name}"));
    }
    fn save(&self, name: &str) -> Result<(), WorkspaceStateError> {
        self.calls.lock().push(format!("save:{name}"));
        if self.fail {
            Err(WorkspaceStateError::Message("failed".into()))
        } else {
            Ok(())
        }
    }
}

#[test]
fn test_表示状態保存_保存成功後だけ対象名を通知する() {
    for fail in [false, true] {
        // Given
        let store = Store {
            fail,
            calls: Default::default(),
        };
        let publisher = crate::usecase::state_subscription::StateSubscriptionPublisher::for_test();
        let mut changes = publisher.subscribe_changes();
        let state = WorkspaceState {
            version: 1,
            tabs: WorkspaceTabsState {
                editors: vec![],
                active_editor_path: None,
            },
            layout: WorkspaceLayoutState {
                center_tab: "editor".into(),
                active_view: "git".into(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
                right_bottom_active_tab: None,
                selected_diff_file: None,
            },
        };
        // When
        let result = save_workspace_state(&store, Some(&publisher), "worktree", state);
        // Then
        assert_eq!(*store.calls.lock(), ["set:worktree", "save:worktree"]);
        assert_eq!(result.is_err(), fail);
        if fail {
            assert!(changes.try_recv().is_err());
        } else {
            assert_eq!(
                changes.try_recv().unwrap(),
                crate::domain::state_subscription::StateChangeSource::WorkspaceState(
                    "worktree".into()
                )
            );
        }
    }
}
