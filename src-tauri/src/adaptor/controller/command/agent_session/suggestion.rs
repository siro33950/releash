use std::sync::Arc;

use crate::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::session::{AgentPromptSuggestion, AgentPromptSuggestionUsecase};

#[tauri::command]
pub fn build_agent_prompt_suggestion(
    app: tauri::AppHandle,
    prompt_suggestion_usecase: tauri::State<'_, Arc<AgentPromptSuggestionUsecase>>,
    chat_session_id: String,
) -> Result<Option<AgentPromptSuggestion>, String> {
    let data_dir = resolve_data_dir(&app)?;
    prompt_suggestion_usecase.build(&data_dir, &chat_session_id)
}
