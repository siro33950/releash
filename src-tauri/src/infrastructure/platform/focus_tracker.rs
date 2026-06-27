use std::sync::Arc;
use std::time::Instant;

use tauri::Manager;

pub struct FocusTracker {
    last_blur_at: Option<Instant>,
    is_focused: bool,
}

impl FocusTracker {
    pub fn new() -> Self {
        Self {
            last_blur_at: None,
            is_focused: true,
        }
    }

    pub fn on_focus(&mut self) {
        self.is_focused = true;
        self.last_blur_at = None;
    }

    pub fn on_blur(&mut self) {
        self.is_focused = false;
        self.last_blur_at = Some(Instant::now());
    }

    pub fn on_focus_changed(&mut self, focused: bool) {
        if focused {
            self.on_focus();
        } else {
            self.on_blur();
        }
    }

    pub fn is_inactive(&self, timeout_minutes: u32) -> bool {
        if self.is_focused {
            return false;
        }
        match self.last_blur_at {
            Some(blur_at) => {
                blur_at.elapsed() >= std::time::Duration::from_secs(u64::from(timeout_minutes) * 60)
            }
            None => false,
        }
    }
}

pub(crate) fn install<R: tauri::Runtime>(
    app: &tauri::App<R>,
    focus_tracker: Arc<parking_lot::Mutex<FocusTracker>>,
) {
    let window = app.get_webview_window("main");
    if window.is_none() {
        log::warn!("Main window not found; focus tracking will be disabled");
    }
    if let Some(window) = window {
        crate::other::telemetry::record_startup_from_origin(
            crate::other::telemetry::Startup::FirstWindowReady,
        );
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                focus_tracker.lock().on_focus_changed(*focused);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_is_focused() {
        let tracker = FocusTracker::new();
        assert!(tracker.is_focused);
        assert!(tracker.last_blur_at.is_none());
    }

    #[test]
    fn blur_then_immediate_check_not_inactive() {
        let mut tracker = FocusTracker::new();
        tracker.on_blur();
        assert!(!tracker.is_focused);
        // timeout=1 minute なので、blur直後はinactiveにならない
        assert!(!tracker.is_inactive(1));
    }

    #[test]
    fn focus_resets_inactive() {
        let mut tracker = FocusTracker::new();
        tracker.on_blur();
        assert!(!tracker.is_focused);
        tracker.on_focus();
        assert!(tracker.is_focused);
        assert!(tracker.last_blur_at.is_none());
        assert!(!tracker.is_inactive(0));
    }

    #[test]
    fn zero_timeout_inactive_immediately_after_blur() {
        let mut tracker = FocusTracker::new();
        tracker.on_blur();
        // timeout=0 なので、blur後すぐにinactive
        assert!(tracker.is_inactive(0));
    }

    #[test]
    fn focus_change_dispatches_to_matching_tracker_state() {
        let mut tracker = FocusTracker::new();

        tracker.on_focus_changed(false);
        assert!(!tracker.is_focused);
        assert!(tracker.last_blur_at.is_some());

        tracker.on_focus_changed(true);
        assert!(tracker.is_focused);
        assert!(tracker.last_blur_at.is_none());
    }
}
