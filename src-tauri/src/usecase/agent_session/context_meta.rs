use serde::{Deserialize, Serialize};

use crate::domain::agent_session::{
    ContextEpoch, ContextEpochId, ContextRevision, ContextSourceKind,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextSourceRevisionMeta {
    pub kind: String,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fingerprint: Option<String>,
    #[serde(skip, default)]
    pub payload: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextSourcePayloadCache {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fingerprint: Option<String>,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextEpochMeta {
    pub epoch_id: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_id: Option<String>,
    pub worktree_path: String,
    #[serde(default)]
    pub source_revisions: Vec<ContextSourceRevisionMeta>,
}

impl ContextEpochMeta {
    pub fn epoch(&self) -> ContextEpoch {
        ContextEpoch {
            id: ContextEpochId(self.epoch_id),
            backend_id: self.backend_id.clone(),
            model_id: self.model_id.clone(),
            worktree_path: self.worktree_path.clone(),
        }
    }

    pub fn revision_for(&self, kind: ContextSourceKind) -> Option<ContextRevision> {
        self.source_revisions
            .iter()
            .find(|source| context_source_kind_from_key(&source.kind) == Some(kind))
            .map(|source| ContextRevision(source.revision))
    }

    pub fn fingerprint_for(&self, kind: ContextSourceKind) -> Option<&str> {
        self.source_revisions
            .iter()
            .find(|source| context_source_kind_from_key(&source.kind) == Some(kind))
            .and_then(|source| source.fingerprint.as_deref())
    }

    pub fn payload_for(&self, kind: ContextSourceKind) -> Option<&str> {
        self.source_revisions
            .iter()
            .find(|source| context_source_kind_from_key(&source.kind) == Some(kind))
            .and_then(|source| source.payload.as_deref())
    }

    pub fn payload_cache_entries(&self) -> Vec<ContextSourcePayloadCache> {
        self.source_revisions
            .iter()
            .filter_map(|source| {
                let payload = source
                    .payload
                    .as_ref()
                    .filter(|payload| !payload.trim().is_empty())?;
                Some(ContextSourcePayloadCache {
                    kind: source.kind.clone(),
                    fingerprint: source.fingerprint.clone(),
                    payload: payload.clone(),
                })
            })
            .collect()
    }

    pub fn hydrate_payload_cache(&mut self, payloads: &[ContextSourcePayloadCache]) {
        for source in &mut self.source_revisions {
            if source.payload.is_some() {
                continue;
            }
            let Some(cached) = payloads.iter().find(|payload| {
                payload.kind == source.kind
                    && source.fingerprint.as_deref().is_none_or(|fingerprint| {
                        payload.fingerprint.as_deref() == Some(fingerprint)
                    })
            }) else {
                continue;
            };
            source.payload = Some(cached.payload.clone());
        }
    }
}

pub(crate) fn context_source_kind_key(kind: ContextSourceKind) -> &'static str {
    match kind {
        ContextSourceKind::RepoSummary => "repo_summary",
        ContextSourceKind::DiffReviewSnapshot => "diff_review_snapshot",
        ContextSourceKind::OpenEditorSelection => "open_editor_selection",
        ContextSourceKind::Mentions => "mentions",
        ContextSourceKind::TerminalLogSummary => "terminal_log_summary",
        ContextSourceKind::WorkflowState => "workflow_state",
        ContextSourceKind::ProjectInstructions => "project_instructions",
        ContextSourceKind::BackendModelIdentity => "backend_system_prompt",
    }
}

fn context_source_kind_from_key(key: &str) -> Option<ContextSourceKind> {
    match key {
        "repo_summary" => Some(ContextSourceKind::RepoSummary),
        "diff_review_snapshot" => Some(ContextSourceKind::DiffReviewSnapshot),
        "open_editor_selection" => Some(ContextSourceKind::OpenEditorSelection),
        "mentions" => Some(ContextSourceKind::Mentions),
        "terminal_log_summary" => Some(ContextSourceKind::TerminalLogSummary),
        "workflow_state" => Some(ContextSourceKind::WorkflowState),
        "project_instructions" => Some(ContextSourceKind::ProjectInstructions),
        "backend_system_prompt" => Some(ContextSourceKind::BackendModelIdentity),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_source(
        kind: &str,
        fingerprint: Option<&str>,
        payload: Option<&str>,
    ) -> ContextSourceRevisionMeta {
        ContextSourceRevisionMeta {
            kind: kind.to_string(),
            revision: 2,
            fingerprint: fingerprint.map(str::to_string),
            payload: payload.map(str::to_string),
        }
    }

    #[test]
    fn hydrate_payload_cache_skips_fingerprint_mismatch() {
        let mut meta = ContextEpochMeta {
            epoch_id: 1,
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: "/repo".to_string(),
            source_revisions: vec![meta_source("repo_summary", Some("fresh"), None)],
        };
        let payloads = vec![ContextSourcePayloadCache {
            kind: "repo_summary".to_string(),
            fingerprint: Some("stale".to_string()),
            payload: "stale payload".to_string(),
        }];

        meta.hydrate_payload_cache(&payloads);

        assert_eq!(
            meta.payload_for(ContextSourceKind::RepoSummary),
            None,
            "stale cache payload must not be restored"
        );
    }

    #[test]
    fn hydrate_payload_cache_restores_when_source_fingerprint_is_absent() {
        let mut meta = ContextEpochMeta {
            epoch_id: 1,
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: "/repo".to_string(),
            source_revisions: vec![meta_source("repo_summary", None, None)],
        };
        let payloads = vec![ContextSourcePayloadCache {
            kind: "repo_summary".to_string(),
            fingerprint: Some("cache".to_string()),
            payload: "cached payload".to_string(),
        }];

        meta.hydrate_payload_cache(&payloads);

        assert_eq!(
            meta.payload_for(ContextSourceKind::RepoSummary),
            Some("cached payload")
        );
    }

    #[test]
    fn hydrate_payload_cache_does_not_overwrite_existing_payload() {
        let mut meta = ContextEpochMeta {
            epoch_id: 1,
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: "/repo".to_string(),
            source_revisions: vec![meta_source(
                "repo_summary",
                Some("fresh"),
                Some("existing payload"),
            )],
        };
        let payloads = vec![ContextSourcePayloadCache {
            kind: "repo_summary".to_string(),
            fingerprint: Some("fresh".to_string()),
            payload: "cached payload".to_string(),
        }];

        meta.hydrate_payload_cache(&payloads);

        assert_eq!(
            meta.payload_for(ContextSourceKind::RepoSummary),
            Some("existing payload")
        );
    }

    #[test]
    fn backend_model_identity_uses_legacy_backend_system_prompt_key() {
        assert_eq!(
            context_source_kind_key(ContextSourceKind::BackendModelIdentity),
            "backend_system_prompt"
        );
        assert_eq!(
            context_source_kind_from_key("backend_system_prompt"),
            Some(ContextSourceKind::BackendModelIdentity)
        );
    }
}
