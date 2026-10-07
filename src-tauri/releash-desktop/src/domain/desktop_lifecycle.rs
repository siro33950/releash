#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopWindow {
    Normal,
    ConnectionFailure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectedWindow {
    Visible,
    Hidden,
}
pub fn show_after_connection(
    hidden: bool,
    start_minimized: bool,
    failure_window: bool,
) -> ConnectedWindow {
    if !hidden || !start_minimized || failure_window {
        ConnectedWindow::Visible
    } else {
        ConnectedWindow::Hidden
    }
}
pub fn window(connected: bool) -> DesktopWindow {
    if connected {
        DesktopWindow::Normal
    } else {
        DesktopWindow::ConnectionFailure
    }
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
