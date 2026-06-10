use crate::infrastructure::agent_session::runtime::ImageAttachment;

#[tauri::command]
pub fn prepare_image_attachment(data: Vec<u8>) -> Result<ImageAttachment, String> {
    crate::infrastructure::agent_session::runtime::prepare_image_attachment(data)
}

#[tauri::command]
pub async fn prepare_image_attachments_from_paths(
    paths: Vec<String>,
) -> Result<Vec<ImageAttachment>, String> {
    crate::infrastructure::agent_session::runtime::prepare_image_attachments_from_paths(paths).await
}
