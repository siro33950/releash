pub fn record<V, E: std::fmt::Display>(result: &Result<V, E>) {
    if let Err(error) = result {
        log::error!("{error}");
    }
}

#[cfg(test)]
#[path = "log_failure_test.rs"]
mod log_failure_tests;
