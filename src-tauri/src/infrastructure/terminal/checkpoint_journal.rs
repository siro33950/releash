use crate::infrastructure::terminal::terminal_emulator::{
    NativeTerminalCheckpoint, NativeTerminalCheckpointRecord,
};

pub(crate) struct PendingCheckpointFlush {
    pub(crate) base: Option<NativeTerminalCheckpoint>,
    pub(crate) records: Vec<NativeTerminalCheckpointRecord>,
}

pub(crate) struct IncrementalCheckpointJournal {
    base: NativeTerminalCheckpoint,
    base_persisted: bool,
    pending: Vec<NativeTerminalCheckpointRecord>,
    latest_sequence: u64,
}

impl IncrementalCheckpointJournal {
    pub(crate) fn new(base: NativeTerminalCheckpoint, base_persisted: bool) -> Self {
        let latest_sequence = base.sequence;
        Self {
            base,
            base_persisted,
            pending: Vec::new(),
            latest_sequence,
        }
    }

    pub(crate) fn record(&mut self, record: NativeTerminalCheckpointRecord) -> Result<(), String> {
        let sequence = record.sequence();
        if sequence != self.latest_sequence + 1 {
            return Err(format!(
                "Terminal Surface checkpoint sequence {} does not follow {}",
                sequence, self.latest_sequence
            ));
        }
        self.latest_sequence = sequence;
        self.pending.push(record);
        Ok(())
    }

    pub(crate) fn take_pending(&mut self) -> PendingCheckpointFlush {
        let base = (!self.base_persisted).then(|| self.base.clone());
        self.base_persisted = true;
        PendingCheckpointFlush {
            base,
            records: std::mem::take(&mut self.pending),
        }
    }

    pub(crate) fn restore_failed(&mut self, mut failed: PendingCheckpointFlush) {
        if failed.base.is_some() {
            self.base_persisted = false;
        }
        failed.records.append(&mut self.pending);
        self.pending = failed.records;
    }

    pub(crate) fn compacted(&mut self, base: NativeTerminalCheckpoint) {
        self.pending
            .retain(|record| record.sequence() > base.sequence);
        self.base = base;
        self.base_persisted = true;
    }
}

#[cfg(test)]
#[path = "checkpoint_journal_test.rs"]
mod checkpoint_journal_tests;
