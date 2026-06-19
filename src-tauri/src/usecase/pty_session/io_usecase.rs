use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionGateway;

pub fn write(
    manager: &impl PtySessionGateway,
    pty_id: u64,
    data: &str,
) -> Result<(), UsecaseError> {
    manager.write(pty_id, data)
}

pub fn resize(
    manager: &impl PtySessionGateway,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), UsecaseError> {
    manager.resize(pty_id, rows, cols)
}
