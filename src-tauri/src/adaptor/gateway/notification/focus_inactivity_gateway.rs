use std::sync::Arc;

use crate::infrastructure::platform::focus_tracker::FocusTracker;
use crate::usecase::notification::usecase::NotificationInactivityGateway;

pub struct FocusNotificationInactivityGateway {
    focus_tracker: Arc<parking_lot::Mutex<FocusTracker>>,
}

impl FocusNotificationInactivityGateway {
    pub fn new(focus_tracker: Arc<parking_lot::Mutex<FocusTracker>>) -> Self {
        Self { focus_tracker }
    }
}

impl NotificationInactivityGateway for FocusNotificationInactivityGateway {
    fn is_inactive(&self, timeout_minutes: u32) -> bool {
        self.focus_tracker.lock().is_inactive(timeout_minutes)
    }
}
