use super::work_queue::{FailureObservation, FailurePage};
use crate::domain::failure_records::{FailureRecord, FailureRecords};
use tokio::sync::Mutex;

pub(crate) struct FailureQueryService {
    pub(super) records: Mutex<FailureRecords>,
}
impl FailureQueryService {
    pub(crate) fn new() -> Self {
        Self {
            records: Mutex::new(FailureRecords::new(4096)),
        }
    }
    pub async fn records(&self, target: &str) -> Vec<FailureObservation> {
        let records: Vec<_> = self
            .records
            .lock()
            .await
            .records()
            .filter(|record| target == "*" || record.target == target)
            .cloned()
            .collect();
        Self::observations(records)
    }

    pub async fn records_page(&self, target: &str, offset: usize) -> FailurePage {
        self.records_page_for_targets(&[target.to_string()], offset)
            .await
    }

    pub(crate) async fn records_page_for_targets(
        &self,
        targets: &[String],
        offset: usize,
    ) -> FailurePage {
        let (records, total, requires_attention) = {
            let records = self.records.lock().await;
            let matching = || {
                records.records().filter(|record| {
                    targets
                        .iter()
                        .any(|target| target == "*" || record.target == *target)
                })
            };
            let mut requires_attention = false;
            for record in matching() {
                if record.active && self::requires_attention(record.kind) {
                    requires_attention = true;
                    break;
                }
            }
            (
                matching().skip(offset).take(100).cloned().collect(),
                matching().count(),
                requires_attention,
            )
        };
        FailurePage {
            items: Self::observations(records),
            next_offset: (offset.saturating_add(100) < total).then_some(offset.saturating_add(100)),
            requires_attention,
        }
    }

    fn observations(records: Vec<FailureRecord>) -> Vec<FailureObservation> {
        let mut result = Vec::new();
        for record in records {
            let requires_attention = record.active && self::requires_attention(record.kind);
            result.push(FailureObservation {
                record,
                requires_attention,
            });
        }
        result
    }

    pub async fn apply_workflow_failures(
        &self,
        tree: &mut crate::domain::workspace_tree::WorkspaceTree,
    ) {
        let targets: std::collections::HashSet<_> = tree
            .nodes()
            .iter()
            .flat_map(|node| {
                [
                    Some(node.id.clone()),
                    node.node_execution_id.clone(),
                    node.execution_id.clone(),
                ]
            })
            .flatten()
            .collect();
        for target in targets {
            for observation in self.records(&target).await {
                if observation.requires_attention {
                    tree.observe_background_failure(&target, &observation.record.message);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "failure_query_service_test.rs"]
mod failure_query_service_tests;

pub(super) fn requires_attention(kind: crate::domain::failure::Failure) -> bool {
    use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};
    matches!(
        kind,
        Failure::Business(BusinessFailure::Other)
            | Failure::Technical(TechnicalFailureNature::TimedOut | TechnicalFailureNature::Other)
    )
}
