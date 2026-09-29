use crate::domain::failure::{FailureKey, FailureRecord, FailureRecordRepository, WorkFailure};
use crate::usecase::failure::{FailureObservation, FailurePage, FailureQueryService};
use std::collections::VecDeque;
use std::sync::Mutex;

const CAPACITY: usize = 4096;
const PAGE_SIZE: usize = 100;

pub struct FailureRecordStore {
    records: Mutex<VecDeque<FailureRecord>>,
}

impl Default for FailureRecordStore {
    fn default() -> Self {
        Self {
            records: Mutex::new(VecDeque::new()),
        }
    }
}

impl FailureRecordStore {
    #[cfg(test)]
    pub(crate) fn observe(&self, key: &FailureKey, failure: WorkFailure) -> bool {
        self.observe_at(key, failure, now_ms())
    }

    #[cfg(test)]
    pub(crate) fn observe_at(&self, key: &FailureKey, failure: WorkFailure, now_ms: u64) -> bool {
        let attention = crate::usecase::failure::requires_attention(failure.kind);
        self.observe_at_with_attention(key, failure, attention, now_ms)
    }

    fn observe_at_with_attention(
        &self,
        key: &FailureKey,
        failure: WorkFailure,
        requires_attention: bool,
        now_ms: u64,
    ) -> bool {
        let mut records = self.records.lock().expect("failure records");
        let attention = records
            .iter()
            .find(|record| matches_key(record, key) && record.active && record.requires_attention)
            .map(|record| (record.kind, record.message.clone()));
        let changed =
            attention != requires_attention.then(|| (failure.kind, failure.message.clone()));
        for record in records.iter_mut().filter(|record| matches_key(record, key)) {
            record.active = false;
        }
        let existing = records
            .iter()
            .position(|record| matches_key(record, key) && record.kind == failure.kind);
        let record = if let Some(index) = existing {
            let mut record = records.remove(index).expect("existing record");
            record.count = record.count.saturating_add(1);
            record.last_observed_ms = now_ms.max(record.last_observed_ms);
            record.message = failure.message;
            record.active = true;
            record.requires_attention = requires_attention;
            record
        } else {
            if records.len() == CAPACITY {
                records.pop_front();
            }
            FailureRecord {
                operation: key.operation.clone(),
                target: key.target.clone(),
                kind: failure.kind,
                message: failure.message,
                active: true,
                requires_attention,
                count: 1,
                first_observed_ms: now_ms,
                last_observed_ms: now_ms,
            }
        };
        records.push_back(record);
        changed
    }

    #[cfg(test)]
    pub(crate) fn resolve(&self, key: &FailureKey) -> bool {
        self.record_resolved(key)
    }

    fn resolve_record(&self, key: &FailureKey) -> bool {
        let mut records = self.records.lock().expect("failure records");
        let mut changed = false;
        for record in records.iter_mut().filter(|record| matches_key(record, key)) {
            changed |= record.active && record.requires_attention;
            record.active = false;
        }
        changed
    }

    pub(crate) fn attention_messages(&self, target: &str) -> Vec<String> {
        self.records
            .lock()
            .expect("failure records")
            .iter()
            .filter(|record| record.target == target && record.active && record.requires_attention)
            .map(|record| record.message.clone())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn records(&self, target: &str) -> Vec<FailureObservation> {
        self.records
            .lock()
            .expect("failure records")
            .iter()
            .filter(|record| target == "*" || record.target == target)
            .cloned()
            .map(observation)
            .collect()
    }
}

impl FailureRecordRepository for FailureRecordStore {
    fn record_observed(
        &self,
        key: &FailureKey,
        failure: WorkFailure,
        requires_attention: bool,
    ) -> bool {
        self.observe_at_with_attention(key, failure, requires_attention, now_ms())
    }

    fn record_resolved(&self, key: &FailureKey) -> bool {
        self.resolve_record(key)
    }
}

fn matches_key(record: &FailureRecord, key: &FailureKey) -> bool {
    record.operation == key.operation && record.target == key.target
}

fn observation(record: FailureRecord) -> FailureObservation {
    let requires_attention = record.active && record.requires_attention;
    FailureObservation {
        record,
        requires_attention,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait::async_trait]
impl FailureQueryService for FailureRecordStore {
    async fn page(&self, targets: &[String], offset: usize) -> FailurePage {
        let records = self.records.lock().expect("failure records");
        let matching = || {
            records.iter().filter(|record| {
                targets
                    .iter()
                    .any(|target| target == "*" || record.target == *target)
            })
        };
        let requires_attention =
            matching().any(|record| record.active && record.requires_attention);
        let total = matching().count();
        let items = matching()
            .skip(offset)
            .take(PAGE_SIZE)
            .cloned()
            .map(observation)
            .collect();
        FailurePage {
            items,
            next_offset: (offset.saturating_add(PAGE_SIZE) < total)
                .then_some(offset.saturating_add(PAGE_SIZE)),
            requires_attention,
        }
    }
}

#[cfg(test)]
#[path = "failure_records_test.rs"]
mod failure_records_tests;
