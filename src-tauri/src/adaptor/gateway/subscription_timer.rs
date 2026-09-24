use crate::usecase::state_subscription::SubscriptionTimer;
use futures_util::Stream;
use std::{pin::Pin, time::Duration};
pub(crate) struct TokioSubscriptionTimer;
impl SubscriptionTimer for TokioSubscriptionTimer {
    fn interval(&self, duration: Duration) -> Pin<Box<dyn Stream<Item = ()> + Send>> {
        let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + duration, duration);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Box::pin(futures_util::stream::unfold(
            timer,
            |mut timer| async move {
                timer.tick().await;
                Some(((), timer))
            },
        ))
    }
}

#[cfg(test)]
#[path = "subscription_timer_test.rs"]
mod subscription_timer_tests;
