mod shared;
pub(crate) use shared::register_shared;

pub(crate) mod issue;

use crate::adaptor::presenter::error::AppError;

pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    crate::common::operation_context::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))
}
