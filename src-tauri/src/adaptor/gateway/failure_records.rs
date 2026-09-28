use crate::usecase::failure::{
    requires_attention, FailureKey, FailureObservation, FailurePage, FailureQueryService,
    FailureRecord, WorkFailure,
};
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
    pub(crate) fn observe(&self, key: &FailureKey, failure: WorkFailure) -> bool {
        self.observe_at(key, failure, now_ms())
    }

    pub(crate) fn observe_at(&self, key: &FailureKey, failure: WorkFailure, now_ms: u64) -> bool {
        let mut records = self.records.lock().expect("failure records");
        let attention = records
            .iter()
            .find(|record| {
                matches_key(record, key) && record.active && requires_attention(record.kind)
            })
            .map(|record| (record.kind, record.message.clone()));
        let changed = attention
            != requires_attention(failure.kind).then(|| (failure.kind, failure.message.clone()));
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
                count: 1,
                first_observed_ms: now_ms,
                last_observed_ms: now_ms,
            }
        };
        records.push_back(record);
        changed
    }

    pub(crate) fn resolve(&self, key: &FailureKey) -> bool {
        let mut records = self.records.lock().expect("failure records");
        let mut changed = false;
        for record in records.iter_mut().filter(|record| matches_key(record, key)) {
            changed |= record.active && requires_attention(record.kind);
            record.active = false;
        }
        changed
    }

    pub(crate) fn attention_messages(&self, target: &str) -> Vec<String> {
        self.records
            .lock()
            .expect("failure records")
            .iter()
            .filter(|record| {
                record.target == target && record.active && requires_attention(record.kind)
            })
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

fn matches_key(record: &FailureRecord, key: &FailureKey) -> bool {
    record.operation == key.operation && record.target == key.target
}

fn observation(record: FailureRecord) -> FailureObservation {
    let requires_attention = record.active && requires_attention(record.kind);
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
            matching().any(|record| record.active && requires_attention(record.kind));
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
