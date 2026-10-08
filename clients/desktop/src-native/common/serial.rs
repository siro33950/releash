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

pub async fn run<R: tauri::Runtime, T, U>(
    app: &tauri::AppHandle<R>,
    operation: impl std::future::Future<Output = T>,
    completed: impl FnOnce(T) -> U,
) -> U {
    use tauri::Manager;
    app.state::<Serial>()
        .call(async { completed(operation.await) })
        .await
}
