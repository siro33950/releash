pub struct LogFailure<T>(pub T);
impl<T> LogFailure<T> {
    pub fn call<V, E: std::fmt::Display>(
        &self,
        operation: impl FnOnce(&T) -> Result<V, E>,
    ) -> Result<V, E> {
        let result = operation(&self.0);
        if let Err(error) = &result {
            log::error!("{error}");
        }
        result
    }
}
