pub struct Deadline(pub std::time::Duration);
impl Deadline {
    pub async fn call<T>(
        &self,
        operation: impl std::future::Future<Output = T>,
    ) -> Result<T, tokio::time::error::Elapsed> {
        tokio::time::timeout(self.0, operation).await
    }
}
