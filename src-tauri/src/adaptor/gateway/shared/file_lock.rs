use std::fs::File;
use std::io;
use std::time::{Duration, Instant};

use crate::domain::operation_context::{OperationContext, OperationStopped};

pub enum LockError {
    Io(io::Error),
    Stopped(OperationStopped),
}

fn try_exclusive(file: &File, context: &OperationContext) -> Result<bool, LockError> {
    context.check(Instant::now()).map_err(LockError::Stopped)?;
    match fs2::FileExt::try_lock_exclusive(file) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(LockError::Io(error)),
    }
}

pub fn exclusive(file: &File, context: &OperationContext) -> Result<(), LockError> {
    while !try_exclusive(file, context)? {
        crate::other::operation_context::sleep(context, Duration::from_millis(10))
            .map_err(LockError::Stopped)?;
    }
    Ok(())
}

pub async fn exclusive_async(file: &File, context: &OperationContext) -> Result<(), LockError> {
    while !try_exclusive(file, context)? {
        crate::other::operation_context::wait(
            context,
            tokio::time::sleep(Duration::from_millis(10)),
        )
        .await
        .map_err(LockError::Stopped)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "file_lock_test.rs"]
mod file_lock_tests;
