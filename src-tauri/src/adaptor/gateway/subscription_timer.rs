use crate::usecase::state_subscription::SubscriptionTimer;
use futures_util::Stream;
use std::{pin::Pin, time::Duration};
pub(crate) struct TokioSubscriptionTimer;
impl SubscriptionTimer for TokioSubscriptionTimer {
    fn interval(&self, duration: Duration) -> Pin<Box<dyn Stream<Item = ()> + Send>> {
        Box::pin(crate::infrastructure::timer::ticks_after(
            duration, duration,
        ))
    }
}

#[cfg(test)]
#[path = "subscription_timer_test.rs"]
mod subscription_timer_tests;
