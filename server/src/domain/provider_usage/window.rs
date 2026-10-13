use super::UsageError;
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    used_percent: f64,
    window_minutes: u64,
    resets_at: u64,
}
impl UsageWindow {
    pub fn five_hour(used_percent: f64, resets_at: u64) -> Result<Self, UsageError> {
        Self::new(used_percent, 300, resets_at)
    }
    pub fn seven_day(used_percent: f64, resets_at: u64) -> Result<Self, UsageError> {
        Self::new(used_percent, 10080, resets_at)
    }

    pub fn new(used_percent: f64, window_minutes: u64, resets_at: u64) -> Result<Self, UsageError> {
        if !used_percent.is_finite()
            || !(0.0..=100.0).contains(&used_percent)
            || window_minutes == 0
            || resets_at == 0
            || resets_at > i64::MAX as u64
        {
            return Err(UsageError::Invalid);
        }
        Ok(Self {
            used_percent,
            window_minutes,
            resets_at,
        })
    }
    pub fn used_percent(&self) -> f64 {
        self.used_percent
    }
    pub fn window_minutes(&self) -> u64 {
        self.window_minutes
    }
    pub fn resets_at(&self) -> u64 {
        self.resets_at
    }
    pub fn warning(&self) -> bool {
        self.used_percent > 80.0
    }
}

#[cfg(test)]
#[path = "window_test.rs"]
mod window_tests;
