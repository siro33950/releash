use parking_lot::Mutex;
use tauri::{
    menu::{AboutMetadata, Menu, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
    App, Emitter, Manager, Wry,
};

pub mod ids {
    // File
    pub const OPEN_FOLDER: &str = "open-folder";

    // View
    pub const THEME_DARK: &str = "theme-dark";
    pub const THEME_LIGHT: &str = "theme-light";
    pub const INCREASE_FONT_SIZE: &str = "increase-font-size";
    pub const DECREASE_FONT_SIZE: &str = "decrease-font-size";
    pub const RESET_FONT_SIZE: &str = "reset-font-size";

    // Git
    pub const GIT_STAGE_ALL: &str = "git-stage-all";
    pub const GIT_UNSTAGE_ALL: &str = "git-unstage-all";
    pub const GIT_CREATE_BRANCH: &str = "git-create-branch";

    // Terminal
    pub const NEW_TERMINAL: &str = "new-terminal";

    // Worktree
    pub const BACK_TO_KANBAN: &str = "back-to-kanban";

    // Remote
    pub const REMOTE_START_SERVER: &str = "remote-start-server";
    pub const REMOTE_STOP_SERVER: &str = "remote-stop-server";
    pub const REMOTE_SHOW_QR: &str = "remote-show-qr";

    // App
    pub const SETTINGS: &str = "settings";
}

pub struct MenuItemsState {
    pub worktree_items: Vec<tauri::menu::MenuItem<Wry>>,
}

pub fn setup_menu(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();

    // ---- Worktree-dependent items (collected for enable/disable) ----
    let mut worktree_items: Vec<tauri::menu::MenuItem<Wry>> = Vec::new();

    // ---- App (Releash) menu ----
    let settings_item = MenuItemBuilder::with_id(ids::SETTINGS, "Settings...")
        .accelerator("CmdOrCtrl+,")
        .build(handle)?;
    let app_menu = SubmenuBuilder::new(handle, "Releash")
        .about(Some(AboutMetadata {
            name: Some("Releash".into()),
            ..Default::default()
        }))
        .separator()
        .item(&settings_item)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    // ---- File menu ----
    let open_folder = MenuItemBuilder::with_id(ids::OPEN_FOLDER, "Open Folder...")
        .accelerator("CmdOrCtrl+O")
        .build(handle)?;

    let file_menu = SubmenuBuilder::new(handle, "File")
        .item(&open_folder)
        .build()?;

    // ---- Edit menu ----
    let edit_menu = SubmenuBuilder::new(handle, "Edit")
        .item(&PredefinedMenuItem::undo(handle, None)?)
        .item(&PredefinedMenuItem::redo(handle, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(handle, None)?)
        .item(&PredefinedMenuItem::copy(handle, None)?)
        .item(&PredefinedMenuItem::paste(handle, None)?)
        .item(&PredefinedMenuItem::select_all(handle, None)?)
        .build()?;

    // ---- View menu ----
    let theme_dark = MenuItemBuilder::with_id(ids::THEME_DARK, "Dark").build(handle)?;
    let theme_light = MenuItemBuilder::with_id(ids::THEME_LIGHT, "Light").build(handle)?;
    let theme_submenu = SubmenuBuilder::new(handle, "Theme")
        .item(&theme_dark)
        .item(&theme_light)
        .build()?;

    let increase_font = MenuItemBuilder::with_id(ids::INCREASE_FONT_SIZE, "Increase Font Size")
        .accelerator("CmdOrCtrl+=")
        .build(handle)?;
    let decrease_font = MenuItemBuilder::with_id(ids::DECREASE_FONT_SIZE, "Decrease Font Size")
        .accelerator("CmdOrCtrl+-")
        .build(handle)?;
    let reset_font = MenuItemBuilder::with_id(ids::RESET_FONT_SIZE, "Reset Font Size")
        .accelerator("CmdOrCtrl+0")
        .build(handle)?;

    worktree_items.extend([
        increase_font.clone(),
        decrease_font.clone(),
        reset_font.clone(),
    ]);

    let view_menu = SubmenuBuilder::new(handle, "View")
        .item(&theme_submenu)
        .separator()
        .item(&increase_font)
        .item(&decrease_font)
        .item(&reset_font)
        .build()?;

    // ---- Git menu ----
    let git_stage_all = MenuItemBuilder::with_id(ids::GIT_STAGE_ALL, "Stage All").build(handle)?;
    let git_unstage_all =
        MenuItemBuilder::with_id(ids::GIT_UNSTAGE_ALL, "Unstage All").build(handle)?;
    let git_create_branch =
        MenuItemBuilder::with_id(ids::GIT_CREATE_BRANCH, "Create Branch...").build(handle)?;

    worktree_items.extend([
        git_stage_all.clone(),
        git_unstage_all.clone(),
        git_create_branch.clone(),
    ]);

    let git_menu = SubmenuBuilder::new(handle, "Git")
        .item(&git_stage_all)
        .item(&git_unstage_all)
        .separator()
        .item(&git_create_branch)
        .build()?;

    // ---- Terminal menu ----
    let new_terminal = MenuItemBuilder::with_id(ids::NEW_TERMINAL, "New Terminal")
        .accelerator("Ctrl+`")
        .build(handle)?;
    worktree_items.push(new_terminal.clone());

    let terminal_menu = SubmenuBuilder::new(handle, "Terminal")
        .item(&new_terminal)
        .build()?;

    // ---- Worktree menu ----
    let back_to_kanban =
        MenuItemBuilder::with_id(ids::BACK_TO_KANBAN, "Back to Kanban").build(handle)?;

    let worktree_menu = SubmenuBuilder::new(handle, "Worktree")
        .item(&back_to_kanban)
        .build()?;

    // ---- Remote menu ----
    let remote_start =
        MenuItemBuilder::with_id(ids::REMOTE_START_SERVER, "Start Server").build(handle)?;
    let remote_stop =
        MenuItemBuilder::with_id(ids::REMOTE_STOP_SERVER, "Stop Server").build(handle)?;
    let remote_qr = MenuItemBuilder::with_id(ids::REMOTE_SHOW_QR, "Show QR Code").build(handle)?;

    let remote_menu = SubmenuBuilder::new(handle, "Remote")
        .item(&remote_start)
        .item(&remote_stop)
        .separator()
        .item(&remote_qr)
        .build()?;

    // ---- Window menu ----
    let window_menu = SubmenuBuilder::new(handle, "Window")
        .minimize()
        .separator()
        .close_window()
        .build()?;

    // ---- Assemble full menu bar ----
    let menu = Menu::with_items(
        handle,
        &[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &git_menu,
            &terminal_menu,
            &worktree_menu,
            &remote_menu,
            &window_menu,
        ],
    )?;

    app.set_menu(menu)?;

    // Store worktree-dependent items for enable/disable
    app.manage(Mutex::new(MenuItemsState { worktree_items }));

    // ---- Menu event handler ----
    app.on_menu_event(move |app_handle, event| {
        let id = event.id().as_ref();
        app_handle.emit("menu-event", id).ok();
    });

    Ok(())
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
