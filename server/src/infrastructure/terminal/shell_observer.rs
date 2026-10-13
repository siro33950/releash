use super::native_pty::{NativePtyError, NativePtyRuntime, NativeShellObservation};

pub(crate) fn spawn(
    native: NativePtyRuntime,
    active: impl FnMut() -> bool + Send + 'static,
    observed: impl FnMut(NativeShellObservation) + Send + 'static,
) {
    std::thread::spawn(move || run(|| native.observe_shell(), active, observed));
}

fn run(
    mut observe: impl FnMut() -> Result<Option<NativeShellObservation>, NativePtyError>,
    mut active: impl FnMut() -> bool,
    mut observed: impl FnMut(NativeShellObservation),
) {
    let mut failure_reported = false;
    while active() {
        match observe() {
            Ok(Some(observation)) => {
                observed(observation);
                failure_reported = false;
            }
            Ok(None) => {}
            Err(error) => {
                if !failure_reported {
                    log::warn!("Terminal shell observation failed: {}", error.message);
                }
                failure_reported = true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
#[path = "shell_observer_test.rs"]
mod shell_observer_tests;
