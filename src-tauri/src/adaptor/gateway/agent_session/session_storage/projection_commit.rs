use std::path::{Path, PathBuf};

use super::layout::{
    event_log_file_in_dir, index_file_in_dir, meta_file_in_dir, private_context_file_in_dir,
    session_dir, write_binary_atomic, write_json_pretty_atomic,
};
use super::private_context::write_private_context_to_dir;
use super::FileSessionStorage;
use crate::domain::agent_session::{AgentSessionProjectedMessage, AgentSessionProjectionPreparer};
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::{ChatMessage, MessagePart, SessionMeta};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionCommitStage {
    Events,
    Message,
    Meta,
}

struct FileSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
}

impl FileSnapshot {
    fn capture(path: PathBuf) -> Result<Self, String> {
        let contents = match std::fs::read(&path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "Failed to checkpoint projected session file {}: {error}",
                    path.display()
                ));
            }
        };
        Ok(Self { path, contents })
    }

    fn restore(&self) -> Result<(), String> {
        match &self.contents {
            Some(contents) => {
                write_binary_atomic(&self.path, contents, "session projection rollback")
            }
            None => match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!(
                    "Failed to remove projected session file {}: {error}",
                    self.path.display()
                )),
            },
        }
    }
}

impl FileSessionStorage {
    pub fn commit_session_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        prepare: &mut dyn AgentSessionProjectionPreparer<
            AgentSessionEvent,
            SessionMeta,
            ChatMessage,
            MessagePart,
        >,
    ) -> Result<Vec<MessagePart>, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(error) = self.invalid_sessions.read().get(session_id) {
            return Err(error.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }

        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        let current_meta = self.read_meta_from_dir(&dir, session_id)?;
        let mut projected_events = self.read_session_events_from_dir(&dir)?;
        projected_events.extend_from_slice(events);
        let prepared = prepare.prepare(&projected_events, &current_meta)?;
        let mut index = self.read_consistent_index_from_dir_with_lock_held(&dir, session_id)?;
        let message_path = match &prepared.message {
            AgentSessionProjectedMessage::Append(message) => {
                if index.iter().any(|entry| entry.id == message.id) {
                    return Err(format!(
                        "Message already exists: {session_id}/{}",
                        message.id
                    ));
                }
                Self::message_path_for_append(&dir, &index)
            }
            AgentSessionProjectedMessage::PersistParts { message_id, .. } => {
                Self::message_path_for_persist(&dir, &index, session_id, message_id)?
            }
        };
        let snapshots = [
            event_log_file_in_dir(&dir),
            message_path.clone(),
            index_file_in_dir(&dir),
            meta_file_in_dir(&dir),
            private_context_file_in_dir(&dir),
        ]
        .into_iter()
        .map(FileSnapshot::capture)
        .collect::<Result<Vec<_>, _>>()?;

        let mut recovered_event_log = false;
        let commit = (|| {
            for event in events {
                recovered_event_log |= self.append_session_event_to_dir(&dir, event)?.recovered;
            }
            self.run_projection_commit_hook(ProjectionCommitStage::Events)?;

            let (meta, persisted_parts) = match prepared.message {
                AgentSessionProjectedMessage::Append(message) => {
                    self.append_message_with_lock_held(&dir, &mut index, prepared.meta, &message)?
                }
                AgentSessionProjectedMessage::PersistParts {
                    message_id,
                    parts,
                    streaming_final_seq,
                    completed_at,
                } => self.persist_message_parts_with_lock_held(
                    &dir,
                    session_id,
                    &mut index,
                    prepared.meta,
                    &message_id,
                    &parts,
                    streaming_final_seq,
                    Some(completed_at),
                )?,
            };
            self.run_projection_commit_hook(ProjectionCommitStage::Message)?;
            write_private_context_to_dir(&dir, &meta)?;
            write_json_pretty_atomic(&index_file_in_dir(&dir), &index, "session index")?;
            write_json_pretty_atomic(&meta_file_in_dir(&dir), &meta, "session meta")?;
            self.run_projection_commit_hook(ProjectionCommitStage::Meta)?;
            self.cache.write().insert(session_id.to_string(), meta);
            self.invalid_sessions.write().remove(session_id);
            Ok(persisted_parts)
        })();

        match commit {
            Ok(parts) => {
                if recovered_event_log {
                    self.record_event_log_recovery(session_id);
                }
                Ok(parts)
            }
            Err(error) => {
                let restore_errors = snapshots
                    .iter()
                    .rev()
                    .filter_map(|snapshot| snapshot.restore().err())
                    .collect::<Vec<_>>();
                if restore_errors.is_empty() {
                    Err(error)
                } else {
                    Err(format!(
                        "{error}; failed to restore projected session files: {}",
                        restore_errors.join("; ")
                    ))
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_projection_commit_hook_for_test(&self, hook: super::ProjectionCommitHook) {
        *self.projection_commit_hook.write() = Some(hook);
    }

    #[cfg(test)]
    fn run_projection_commit_hook(&self, stage: ProjectionCommitStage) -> Result<(), String> {
        if let Some(hook) = self.projection_commit_hook.read().clone() {
            hook(stage)?;
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn run_projection_commit_hook(&self, _stage: ProjectionCommitStage) -> Result<(), String> {
        Ok(())
    }
}
