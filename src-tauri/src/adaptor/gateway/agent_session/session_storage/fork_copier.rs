#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use super::layout::{
    attachments_dir_in_dir, index_file_in_dir, messages_dir_in_dir, meta_file_in_dir, session_dir,
    sessions_dir, tool_outputs_dir_in_dir, validate_meta, write_json_pretty_atomic,
};
#[cfg(test)]
use super::private_context::write_private_context_to_dir;
#[cfg(test)]
use super::stored_session_v1::write_message_index_v1;
use super::FileSessionStorage;
#[cfg(test)]
use crate::usecase::agent_session::session::SessionMeta;

impl FileSessionStorage {
    #[cfg(test)]
    pub fn fork_session_layout(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        forked_meta: &SessionMeta,
    ) -> Result<(), String> {
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
        let parent_dir = session_dir(app_data_dir, session_id)?;
        let forked_meta = validate_meta(forked_meta.clone(), &forked_meta.id)?;
        let _lock = self.file_lock.lock();
        self.apply_pending_session_transaction(&parent_dir, session_id)?;
        let fork_dir = session_dir(app_data_dir, &forked_meta.id)?;
        let tmp_dir = sessions_dir(app_data_dir).join(format!("{}.tmp", forked_meta.id));
        let write_result = (|| -> Result<(), String> {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            std::fs::create_dir_all(messages_dir_in_dir(&tmp_dir))
                .map_err(|e| format!("Failed to create fork messages dir: {e}"))?;
            std::fs::create_dir_all(attachments_dir_in_dir(&tmp_dir))
                .map_err(|e| format!("Failed to create fork attachments dir: {e}"))?;
            std::fs::create_dir_all(tool_outputs_dir_in_dir(&tmp_dir))
                .map_err(|e| format!("Failed to create fork tool outputs dir: {e}"))?;
            write_json_pretty_atomic(&meta_file_in_dir(&tmp_dir), &forked_meta, "session meta")?;
            write_private_context_to_dir(&tmp_dir, &forked_meta)?;
            let index = self.read_index_from_dir(&parent_dir)?;
            write_message_index_v1(&index_file_in_dir(&tmp_dir), &index)?;
            self.link_or_copy_dir_entries(
                &messages_dir_in_dir(&parent_dir),
                &messages_dir_in_dir(&tmp_dir),
            )?;
            self.link_or_copy_dir_entries(
                &attachments_dir_in_dir(&parent_dir),
                &attachments_dir_in_dir(&tmp_dir),
            )?;
            self.link_or_copy_dir_entries(
                &tool_outputs_dir_in_dir(&parent_dir),
                &tool_outputs_dir_in_dir(&tmp_dir),
            )?;
            std::fs::rename(&tmp_dir, &fork_dir)
                .map_err(|e| format!("Failed to install fork session dir: {e}"))?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(err);
        }
        let forked_id = forked_meta.id.clone();
        self.cache.write().insert(forked_id.clone(), forked_meta);
        self.invalid_sessions.write().remove(&forked_id);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn link_or_copy_dir_entries(&self, src: &Path, dst: &Path) -> Result<(), String> {
        if !src.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(dst).map_err(|e| format!("Failed to create fork dir: {e}"))?;
        for entry in
            std::fs::read_dir(src).map_err(|e| format!("Failed to read fork source dir: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Failed to read fork source entry: {e}"))?;
            let src_path = entry.path();
            let file_type = std::fs::symlink_metadata(&src_path)
                .map_err(|e| format!("Failed to inspect fork source entry: {e}"))?
                .file_type();
            if file_type.is_symlink() {
                return Err(format!(
                    "Refusing to fork symlinked session file: {}",
                    entry.file_name().to_string_lossy()
                ));
            }
            if !file_type.is_file() {
                continue;
            }
            let dst_path = dst.join(entry.file_name());
            if std::fs::hard_link(&src_path, &dst_path).is_err() {
                std::fs::copy(&src_path, &dst_path)
                    .map_err(|e| format!("Failed to copy fork session chunk: {e}"))?;
            }
        }
        Ok(())
    }
}
