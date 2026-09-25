use super::failure::FailureKind;
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureRecord {
    pub operation: String,
    pub target: String,
    pub kind: FailureKind,
    pub message: String,
    pub active: bool,
    pub count: u64,
    pub first_observed_ms: u64,
    pub last_observed_ms: u64,
}

pub struct FailureRecords {
    records: VecDeque<FailureRecord>,
    capacity: usize,
}

impl FailureRecords {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            records: VecDeque::new(),
            capacity,
        }
    }

    pub fn observe(
        &mut self,
        operation: &str,
        target: &str,
        kind: FailureKind,
        message: String,
        now_ms: u64,
    ) -> bool {
        let was_attention = self.resolve(operation, target);
        let existing = self.records.iter().position(|record| {
            record.operation == operation && record.target == target && record.kind == kind
        });
        let record = if let Some(index) = existing {
            let mut record = self.records.remove(index).expect("existing record");
            record.count = record.count.saturating_add(1);
            record.last_observed_ms = now_ms.max(record.last_observed_ms);
            record.message = message;
            record.active = true;
            record
        } else {
            if self.records.len() == self.capacity {
                self.records.pop_front();
            }
            FailureRecord {
                operation: operation.into(),
                target: target.into(),
                kind,
                message,
                active: true,
                count: 1,
                first_observed_ms: now_ms,
                last_observed_ms: now_ms,
            }
        };
        self.records.push_back(record);
        was_attention != kind.requires_attention()
    }

    pub fn resolve(&mut self, operation: &str, target: &str) -> bool {
        let mut changed = false;
        for record in &mut self.records {
            if record.operation == operation && record.target == target {
                changed |= record.active && record.kind.requires_attention();
                record.active = false;
            }
        }
        changed
    }

    pub fn records(&self) -> impl Iterator<Item = &FailureRecord> {
        self.records.iter()
    }
}

#[cfg(test)]
#[path = "failure_records_test.rs"]
mod failure_records_tests;
