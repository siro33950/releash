use std::collections::HashSet;

use super::entities::WorkspaceTree;
use super::value_objects::{
    WorkspaceNodeKind, WorkspaceSessionFact, WorkspaceSessionState, WorkspaceStructureFact,
};

pub struct WorkspaceTreeVisibilityPolicy;

impl WorkspaceTreeVisibilityPolicy {
    pub fn hidden_branch_ids<'a>(
        tree: &'a WorkspaceTree,
        active_archive_execution_ids: impl IntoIterator<Item = &'a str>,
    ) -> HashSet<String> {
        let archived = active_archive_execution_ids
            .into_iter()
            .collect::<HashSet<_>>();
        tree.nodes()
            .iter()
            .filter(|node| {
                node.kind == WorkspaceNodeKind::Workflow
                    && node
                        .execution_id
                        .as_deref()
                        .is_some_and(|execution_id| archived.contains(execution_id))
            })
            .map(|node| node.id.clone())
            .collect()
    }
}

pub struct WorkspaceSessionPublicationPolicy;

impl WorkspaceSessionPublicationPolicy {
    pub fn public_summary(
        value: &crate::domain::local_event::AgentSessionProjectionRecord,
    ) -> Option<(
        super::value_objects::WorkspaceSessionListKind,
        crate::domain::local_event::AgentSessionSummaryRecord,
    )> {
        use super::value_objects::WorkspaceSessionListKind as List;
        use crate::domain::agent_session::events::AgentSessionDomainEvent;
        use crate::domain::local_event::AgentRecoveryPublicationListRecord as PublicationList;

        let summary = Self::summary(value);
        let list = match summary.state {
            crate::domain::local_event::AgentSessionStateRecord::Closed => List::Closed,
            crate::domain::local_event::AgentSessionStateRecord::Archived => List::Archived,
            _ => List::Active,
        };
        let mut active_recovery_id = None;
        for event in &value.reducer_events {
            match event {
                AgentSessionDomainEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                    active_recovery_id = Some(recovery_id.as_str());
                }
                AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
                    recovery_id, ..
                } if active_recovery_id == Some(recovery_id.as_str()) => {
                    active_recovery_id = None;
                }
                AgentSessionDomainEvent::BackendSessionRecoveryFailed { .. } => {
                    active_recovery_id = None;
                }
                _ => {}
            }
        }
        let Some(recovery_id) = active_recovery_id else {
            return Some((list, summary));
        };
        let snapshot = value
            .meta
            .recovery_publication_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.recovery_id == recovery_id)?;
        let expected_list = match list {
            List::Active => PublicationList::SessionList,
            List::Closed => PublicationList::ClosedHistory,
            List::Archived => PublicationList::ArchivedHistory,
        };
        if snapshot.classification.list != expected_list
            || !Self::recovery_publication_owner_matches(snapshot)
        {
            return None;
        }
        let mut published = snapshot.summary.clone();
        published.first_message = summary.first_message;
        Some((list, published))
    }

    pub fn summary(
        value: &crate::domain::local_event::AgentSessionProjectionRecord,
    ) -> crate::domain::local_event::AgentSessionSummaryRecord {
        crate::domain::local_event::AgentSessionSummaryRecord {
            id: value.meta.id.clone(),
            worktree_path: crate::domain::repository::normalize_repo_path(
                &value.meta.worktree_path,
            ),
            state: value.meta.state,
            error_reason: value.meta.error_reason.clone(),
            created_at_bits: value.meta.created_at_bits,
            updated_at_bits: value.meta.updated_at_bits,
            first_message: value
                .title
                .clone()
                .unwrap_or_else(|| value.meta.first_message_preview.clone()),
            message_count: value.meta.message_count,
            agent_session_id: value.meta.agent_session_id.clone(),
            context_carry: value.meta.context_carry,
            permission_mode: value.meta.permission_mode.clone(),
            plan_mode: value.meta.plan_mode,
            permission_profile_id: value.meta.permission_profile_id.clone(),
            backend_id: Some(value.meta.backend_id.clone()),
            workflow_node_session: value.meta.workflow_node_session,
            workflow_node_context: value.meta.workflow_node_context.clone(),
        }
    }

    pub fn structure_fact(
        summary: &crate::domain::local_event::AgentSessionSummaryRecord,
        unresolved_recovery_reason: Option<String>,
    ) -> WorkspaceStructureFact {
        use crate::domain::local_event::AgentSessionStateRecord as S;
        let state = match summary.state {
            S::Active => WorkspaceSessionState::Active,
            S::Idle => WorkspaceSessionState::Idle,
            S::Done => WorkspaceSessionState::Done,
            S::Error => WorkspaceSessionState::Error,
            S::Closed => WorkspaceSessionState::Closed,
            S::Archived => WorkspaceSessionState::Archived,
        };
        WorkspaceStructureFact::SessionProjected(WorkspaceSessionFact {
            id: summary.id.clone(),
            worktree_path: summary.worktree_path.clone(),
            state,
            error_reason: summary.error_reason.clone(),
            updated_at_bits: summary.updated_at_bits,
            title: Some(summary.first_message.clone()),
            first_message: summary.first_message.clone(),
            workflow_node_session: summary.workflow_node_session,
            workflow_execution_id: summary
                .workflow_node_context
                .as_ref()
                .map(|context| context.execution_id.clone()),
            workflow_node_execution_id: summary
                .workflow_node_context
                .as_ref()
                .map(|context| context.node_execution_id.clone()),
            unresolved_recovery_reason,
        })
    }

    fn recovery_publication_owner_matches(
        snapshot: &crate::domain::local_event::AgentRecoveryPublicationSnapshotRecord,
    ) -> bool {
        let summary = &snapshot.summary;
        match &snapshot.classification.workflow_owner {
            None => !summary.workflow_node_session && summary.workflow_node_context.is_none(),
            Some(owner) => {
                if !summary.workflow_node_session && summary.workflow_node_context.is_none() {
                    return false;
                }
                match &summary.workflow_node_context {
                    Some(context) => {
                        owner.execution_id.as_deref() == Some(context.execution_id.as_str())
                            && owner.node_execution_id.as_deref()
                                == Some(context.node_execution_id.as_str())
                    }
                    None => owner.execution_id.is_none() && owner.node_execution_id.is_none(),
                }
            }
        }
    }
}

pub fn recovery_reason(
    obligation_id: &str,
    record: &crate::domain::local_event::ObligationRecord,
) -> Option<String> {
    record
        .unresolved_recovery_original_identity(obligation_id)
        .map(|identity| format!("Unresolved recovery {identity} must be resolved before resume."))
}

pub async fn unresolved_recovery_reason(
    repository: &dyn crate::domain::local_event::LocalEventTransactionRepository,
    owner: &str,
) -> Result<Option<String>, String> {
    use crate::domain::local_event::{LocalEventQuery, LocalEventQueryResult, QueryCursor};

    if owner.is_empty() {
        return Err("recovery owner identity is invalid".to_string());
    }
    let mut cursor = None;
    loop {
        let result = repository
            .query(LocalEventQuery::PendingRecoveryPage {
                limit: 200,
                partition: None,
                owner: Some(owner.to_string()),
                ordered_key_prefix: None,
                shutdown_plan: None,
                cursor,
            })
            .await
            .map_err(|error| format!("recovery fence read failed: {error}"))?;
        let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
            return Err("recovery fence read returned the wrong result type".to_string());
        };
        for entry in page.entries {
            if entry.owner != owner {
                return Err("recovery fence owner index is inconsistent".to_string());
            }
            if let Some(reason) = recovery_reason(&entry.obligation_id, &entry.record) {
                return Ok(Some(reason));
            }
        }
        let Some(next) = page.next_cursor else {
            return Ok(None);
        };
        cursor = Some(QueryCursor::from_opaque(next.as_str().to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::events::{
        AgentSessionDomainEvent, BackendSessionRecoveryReason,
    };
    use crate::domain::local_event::{
        AgentRecoveryPublicationClassificationRecord, AgentRecoveryPublicationListRecord,
        AgentRecoveryPublicationSnapshotRecord, AgentRecoveryPublicationWorkflowOwnerRecord,
        AgentSessionMetadataRecord, AgentSessionProjectionRecord, AgentSessionStateRecord,
    };
    use crate::domain::workflow::WorkflowNodeContext;

    fn projection() -> AgentSessionProjectionRecord {
        AgentSessionProjectionRecord {
            meta: AgentSessionMetadataRecord {
                id: "session".to_string(),
                worktree_path: "/repo".to_string(),
                state: AgentSessionStateRecord::Active,
                error_reason: None,
                state_revision: 0,
                created_at_bits: 1.0f64.to_bits(),
                updated_at_bits: 2.0f64.to_bits(),
                agent_session_id: None,
                provider_session_generation: 0,
                provider_session_observation_id: None,
                context_reinjection_generation: None,
                context_carry: None,
                pending_recovery_message: None,
                recovery_publication_snapshot: None,
                permission_mode: "ask".to_string(),
                plan_mode: false,
                selected_model: None,
                permission_profile_id: None,
                backend_id: "codex".to_string(),
                workflow_node_session: false,
                workflow_node_context: None,
                workflow_instructions: Vec::new(),
                agent_read_paths: None,
                context_epoch: None,
                last_turn_interruption: None,
                last_turn_id: None,
                first_message_preview: "current-preview".to_string(),
                message_count: 1,
                body_format_version: 1,
            },
            title: Some("current-title".to_string()),
            reducer_events: Vec::new(),
            queue_paused_at_bits: None,
            latest_token_usage: None,
            pending_send_queue: Vec::new(),
        }
    }

    fn recovery_started() -> AgentSessionDomainEvent {
        AgentSessionDomainEvent::BackendSessionRecoveryStarted {
            recovery_id: "recovery".to_string(),
            old_provider_session_generation: 0,
            reason: BackendSessionRecoveryReason::BackendSessionLost,
            at: 3.0,
        }
    }

    fn snapshot(
        projection: &AgentSessionProjectionRecord,
        list: AgentRecoveryPublicationListRecord,
        workflow_owner: Option<AgentRecoveryPublicationWorkflowOwnerRecord>,
    ) -> AgentRecoveryPublicationSnapshotRecord {
        let mut summary = WorkspaceSessionPublicationPolicy::summary(projection);
        summary.first_message = "snapshot-title".to_string();
        AgentRecoveryPublicationSnapshotRecord {
            recovery_id: "recovery".to_string(),
            summary,
            classification: AgentRecoveryPublicationClassificationRecord {
                list,
                workflow_owner,
            },
        }
    }

    #[test]
    fn public_summary_publishes_when_recovery_never_started() {
        let projection = projection();
        let (_, summary) = WorkspaceSessionPublicationPolicy::public_summary(&projection).unwrap();
        assert_eq!(summary.first_message, "current-title");
    }

    #[test]
    fn public_summary_publishes_current_state_after_recovery_completion() {
        let mut projection = projection();
        projection.reducer_events = vec![
            recovery_started(),
            AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
                recovery_id: "recovery".to_string(),
                provider_session_generation: 1,
                at: 4.0,
            },
        ];
        assert!(WorkspaceSessionPublicationPolicy::public_summary(&projection).is_some());
    }

    #[test]
    fn public_summary_suppresses_active_recovery_without_snapshot() {
        let mut projection = projection();
        projection.reducer_events.push(recovery_started());
        assert_eq!(
            WorkspaceSessionPublicationPolicy::public_summary(&projection),
            None
        );
    }

    #[test]
    fn public_summary_suppresses_snapshot_with_wrong_list() {
        let mut projection = projection();
        projection.reducer_events.push(recovery_started());
        projection.meta.recovery_publication_snapshot = Some(snapshot(
            &projection,
            AgentRecoveryPublicationListRecord::ClosedHistory,
            None,
        ));
        assert_eq!(
            WorkspaceSessionPublicationPolicy::public_summary(&projection),
            None
        );
    }

    #[test]
    fn public_summary_suppresses_snapshot_with_wrong_workflow_owner() {
        let mut projection = projection();
        projection.meta.workflow_node_session = true;
        projection.meta.workflow_node_context = Some(WorkflowNodeContext {
            execution_id: "execution".to_string(),
            node_execution_id: "node".to_string(),
            workflow_name: "workflow".to_string(),
            node_name: "review".to_string(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        });
        projection.reducer_events.push(recovery_started());
        projection.meta.recovery_publication_snapshot = Some(snapshot(
            &projection,
            AgentRecoveryPublicationListRecord::SessionList,
            Some(AgentRecoveryPublicationWorkflowOwnerRecord {
                execution_id: Some("other".to_string()),
                node_execution_id: Some("node".to_string()),
            }),
        ));
        assert_eq!(
            WorkspaceSessionPublicationPolicy::public_summary(&projection),
            None
        );
    }

    #[test]
    fn public_summary_keeps_current_first_message_over_snapshot_value() {
        let mut projection = projection();
        projection.reducer_events.push(recovery_started());
        projection.meta.recovery_publication_snapshot = Some(snapshot(
            &projection,
            AgentRecoveryPublicationListRecord::SessionList,
            None,
        ));
        let (_, summary) = WorkspaceSessionPublicationPolicy::public_summary(&projection).unwrap();
        assert_eq!(summary.first_message, "current-title");
    }
}
