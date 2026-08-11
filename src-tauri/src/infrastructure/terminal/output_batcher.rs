use std::time::{Duration, Instant};

pub(crate) const OUTPUT_BATCH_WINDOW: Duration = Duration::from_millis(2);
pub(crate) const OUTPUT_BATCH_MAX_CODE_UNITS: usize = 16 * 1024;

#[derive(Default)]
pub(crate) struct TerminalOutputBatcher {
    pending: String,
    pending_code_units: usize,
    started_at: Option<Instant>,
}

impl TerminalOutputBatcher {
    pub(crate) fn push(&mut self, now: Instant, data: String) -> Vec<String> {
        let mut ready = self.flush_due(now).into_iter().collect::<Vec<_>>();
        let mut remaining = data;
        loop {
            if remaining.is_empty() {
                break;
            }
            if self.started_at.is_none() {
                self.started_at = Some(now);
            }
            let available = OUTPUT_BATCH_MAX_CODE_UNITS - self.pending_code_units;
            let (head, tail, head_code_units) = take_code_units(remaining, available);
            if head.is_empty() && self.pending_code_units > 0 {
                ready.push(
                    self.flush()
                        .expect("partial output batch must contain data"),
                );
                remaining = tail;
                continue;
            }
            self.pending.push_str(&head);
            self.pending_code_units += head_code_units;
            remaining = tail;
            if self.pending_code_units == OUTPUT_BATCH_MAX_CODE_UNITS {
                ready.push(self.flush().expect("full output batch must contain data"));
            }
        }
        ready
    }

    pub(crate) fn flush_due(&mut self, now: Instant) -> Option<String> {
        self.started_at
            .is_some_and(|started_at| {
                now.saturating_duration_since(started_at) >= OUTPUT_BATCH_WINDOW
            })
            .then(|| self.flush())
            .flatten()
    }

    pub(crate) fn remaining_window(&self, now: Instant) -> Option<Duration> {
        self.started_at.map(|started_at| {
            OUTPUT_BATCH_WINDOW.saturating_sub(now.saturating_duration_since(started_at))
        })
    }

    pub(crate) fn flush(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        self.pending_code_units = 0;
        self.started_at = None;
        Some(std::mem::take(&mut self.pending))
    }
}

fn take_code_units(data: String, maximum: usize) -> (String, String, usize) {
    let mut code_units = 0;
    let mut split_at = data.len();
    for (index, character) in data.char_indices() {
        let next = code_units + character.len_utf16();
        if next > maximum {
            split_at = index;
            break;
        }
        code_units = next;
    }
    let tail = data[split_at..].to_string();
    let head = data[..split_at].to_string();
    (head, tail, code_units)
}

#[cfg(test)]
#[path = "output_batcher_test.rs"]
mod output_batcher_tests;
