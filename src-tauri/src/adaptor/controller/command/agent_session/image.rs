use crate::usecase::agent_session::session::{
    prepare_image_attachment_data, prepare_image_attachments_from_paths_usecase, ImageAttachment,
};

#[tauri::command]
pub fn prepare_image_attachment(data: Vec<u8>) -> Result<ImageAttachment, String> {
    prepare_image_attachment_data(data)
}

#[tauri::command]
pub async fn prepare_image_attachments_from_paths(
    paths: Vec<String>,
) -> Result<Vec<ImageAttachment>, String> {
    prepare_image_attachments_from_paths_usecase(paths).await
}
