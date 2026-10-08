use std::sync::Mutex;

use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessSample {
    pub rss_bytes: u64,
    pub cpu_percent: f64,
}

pub struct ProcessResourceObserver {
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
    pub fn sample(&self) -> Option<ProcessSample> {
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
