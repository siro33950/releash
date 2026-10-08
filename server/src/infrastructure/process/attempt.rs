use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Child;

pub(crate) type SharedChild = Arc<Mutex<Child>>;

tokio::task_local! {
    static CHILDREN: Arc<Mutex<Vec<SharedChild>>>;
}

pub(crate) fn register(child: &SharedChild) {
    let _ = CHILDREN.try_with(|children| {
        children
            .lock()
            .expect("attempt children")
            .push(child.clone())
    });
}

#[derive(Debug, thiserror::Error)]
#[error("background attempt exceeded {duration:?}; child cleanup errors: {cleanup_errors:?}")]
pub(crate) struct AttemptExpired {
    duration: Duration,
    pub(crate) cleanup_errors: Vec<std::io::Error>,
}

pub(crate) async fn timed<T>(
    duration: Duration,
    future: impl Future<Output = T>,
) -> Result<T, AttemptExpired> {
    let children = Arc::new(Mutex::new(Vec::<SharedChild>::new()));
    tokio::pin!(future);
    let result = CHILDREN
        .scope(
            children.clone(),
            tokio::time::timeout(duration, future.as_mut()),
        )
        .await;
    match result {
        Ok(value) => Ok(value),
        Err(_) => {
            let children = std::mem::take(&mut *children.lock().expect("attempt children"));
            let mut errors = Vec::new();
            // Keep the attempt (and its locks) alive until every child has been reaped.
            for child in children {
                if let Err(error) = stop(&child).await {
                    errors.push(error);
                }
            }
            Err(AttemptExpired {
                duration,
                cleanup_errors: errors,
            })
        }
    }
}

pub(crate) async fn stop(child: &SharedChild) -> std::io::Result<()> {
    child.lock().expect("background child").start_kill()?;
    std::future::poll_fn(|context| {
        let mut child = child.lock().expect("background child");
        let wait = child.wait();
        tokio::pin!(wait);
        wait.poll(context).map(|status| status.map(|_| ()))
    })
    .await
}
