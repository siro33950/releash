use crate::infrastructure::agent_session::runtime::SlashCommandEntry;

#[tauri::command]
pub async fn scan_slash_commands(cwd: String) -> Result<Vec<SlashCommandEntry>, String> {
    crate::infrastructure::agent_session::runtime::scan_slash_commands(cwd).await
}
