use crate::usecase::pty_session::ports::PtySessionGateway;

pub fn write(manager: &impl PtySessionGateway, pty_id: u64, data: &str) -> Result<(), String> {
    manager.write(pty_id, data)
}

pub fn resize(
    manager: &impl PtySessionGateway,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    manager.resize(pty_id, rows, cols)
}
