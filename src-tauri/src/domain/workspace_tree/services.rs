use std::collections::HashSet;

use super::entities::WorkspaceTree;
use super::value_objects::WorkspaceNodeKind;

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
