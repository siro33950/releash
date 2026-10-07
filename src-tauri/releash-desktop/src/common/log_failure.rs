pub struct LogFailure<T>(pub T);
impl<T> LogFailure<T> {
    pub fn call<V, E: std::fmt::Display>(
        &self,
        operation: impl FnOnce(&T) -> Result<V, E>,
    ) -> Result<V, E> {
        let result = operation(&self.0);
        self.record(&result);
        result
    }
    pub fn record<V, E: std::fmt::Display>(&self, result: &Result<V, E>) {
        if let Err(error) = result {
            log::error!("{error}");
        }
    }
}

#[cfg(test)]
#[path = "log_failure_test.rs"]
mod log_failure_tests;
