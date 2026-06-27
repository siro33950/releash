use parking_lot::Mutex;

use crate::infrastructure::platform::menu::MenuItemsState;

pub(super) const COMMAND_NAMES: &[&str] = &["set_menu_items_enabled"];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![set_menu_items_enabled]
}

#[tauri::command]
pub fn set_menu_items_enabled(
    state: tauri::State<'_, Mutex<MenuItemsState>>,
    enabled: bool,
) -> Result<(), String> {
    let guard = state.lock();
    for item in &guard.worktree_items {
        item.set_enabled(enabled).map_err(|e| e.to_string())?;
    }
    Ok(())
}
