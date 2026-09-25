use std::sync::Mutex;

use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProcessSample {
    pub(crate) rss_bytes: u64,
    pub(crate) cpu_percent: f64,
}

pub(crate) struct ProcessResourceObserver {
    system: Mutex<System>,
}

impl Default for ProcessResourceObserver {
    fn default() -> Self {
        Self {
            system: Mutex::new(System::new()),
        }
    }
}

impl ProcessResourceObserver {
    pub(crate) fn sample(&self) -> Option<ProcessSample> {
        let pid = get_current_pid().ok()?;
        let mut system = self.system.lock().ok()?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );
        let process = system.process(pid)?;
        Some(ProcessSample {
            rss_bytes: process.memory(),
            cpu_percent: f64::from(process.cpu_usage()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_current_process() {
        let observer = ProcessResourceObserver::default();
        let sample = observer.sample().unwrap();
        assert!(sample.rss_bytes > 0);
        assert!(sample.cpu_percent >= 0.0);
    }
}
