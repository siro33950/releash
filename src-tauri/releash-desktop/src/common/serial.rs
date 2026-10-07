#[derive(Default)]
pub struct Serial(tokio::sync::Mutex<()>);
impl Serial {
    pub async fn call<T>(&self, operation: impl std::future::Future<Output = T>) -> T {
        let _guard = self.0.lock().await;
        operation.await
    }
}

#[cfg(test)]
#[path = "serial_test.rs"]
mod serial_tests;
