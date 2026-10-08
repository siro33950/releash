use crate::domain::failure::{FailureKey, FailureRecord, FailureRecordRepository, WorkFailure};
use std::collections::VecDeque;
use std::sync::Mutex;

const CAPACITY: usize = 4096;

pub struct FailureRecordStore {
    records: Mutex<VecDeque<FailureRecord>>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
pub struct FailureRecordObservation {
    pub record: FailureRecord,
    pub requires_attention: bool,
}

impl Default for FailureRecordStore {
    fn default() -> Self {
        Self {
            records: Mutex::new(VecDeque::new()),
        }
    }
}

impl FailureRecordStore {
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&self, key: &FailureKey, failure: WorkFailure) -> bool {
        self.observe_at(key, failure, now_ms())
    }

    #[cfg(any(test, feature = "test-support"))]
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
            .find(|record| matches_key(record, key) && is_attention(record))
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn resolve(&self, key: &FailureKey) -> bool {
        self.record_resolved(key)
    }

    fn resolve_record(&self, key: &FailureKey) -> bool {
        let mut records = self.records.lock().expect("failure records");
        let mut changed = false;
        for record in records.iter_mut().filter(|record| matches_key(record, key)) {
            changed |= is_attention(record);
            record.active = false;
        }
        changed
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn records(&self, target: &str) -> Vec<FailureRecordObservation> {
        self.records
            .lock()
            .expect("failure records")
            .iter()
            .filter(|record| target == "*" || record.target == target)
            .cloned()
            .map(|record| FailureRecordObservation {
                requires_attention: is_attention(&record),
                record,
            })
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

    fn attention_messages(&self, target: &str) -> Vec<String> {
        self.records
            .lock()
            .expect("failure records")
            .iter()
            .filter(|record| record.target == target && is_attention(record))
            .map(|record| record.message.clone())
            .collect()
    }
}

fn matches_key(record: &FailureRecord, key: &FailureKey) -> bool {
    record.operation == key.operation && record.target == key.target
}

fn is_attention(record: &FailureRecord) -> bool {
    record.active && record.requires_attention
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
#[path = "failure_records_test.rs"]
mod failure_records_tests;
