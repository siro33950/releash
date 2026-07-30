const STREAM_EMIT_FAILURE_FALLBACK_LIMIT: u32 = 5;
const STREAM_EMIT_FAILURE_STOP_LIMIT: u32 = STREAM_EMIT_FAILURE_FALLBACK_LIMIT * 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEmitFailureDecision {
    RetryDelta {
        failures: u32,
        install_snapshot_retry: bool,
        schedule_retry: bool,
    },
    FallbackToSnapshot {
        failures: u32,
        fallback_started: bool,
        schedule_retry: bool,
    },
    Stop {
        failures: u32,
    },
}

impl StreamEmitFailureDecision {
    pub fn failures(self) -> u32 {
        match self {
            Self::RetryDelta { failures, .. }
            | Self::FallbackToSnapshot { failures, .. }
            | Self::Stop { failures } => failures,
        }
    }
}

#[derive(Debug, Default)]
pub struct RuntimeStreamingDelivery {
    regular_failures: u32,
    regular_suppressed: bool,
    regular_flush_scheduled: bool,
    authoritative_failures: u32,
    authoritative_flush_scheduled: bool,
}

impl RuntimeStreamingDelivery {
    pub fn reset_regular(&mut self) {
        self.regular_failures = 0;
        self.regular_suppressed = false;
        self.regular_flush_scheduled = false;
    }

    pub fn finish_regular_turn(&mut self) {
        self.regular_failures = 0;
        self.regular_suppressed = false;
    }

    pub fn regular_is_suppressed(&self) -> bool {
        self.regular_suppressed
    }

    #[cfg(test)]
    pub fn regular_failure_count(&self) -> u32 {
        self.regular_failures
    }

    #[cfg(test)]
    pub fn regular_flush_is_scheduled(&self) -> bool {
        self.regular_flush_scheduled
    }

    pub fn schedule_regular_flush(&mut self) -> bool {
        if self.regular_flush_scheduled {
            false
        } else {
            self.regular_flush_scheduled = true;
            true
        }
    }

    pub fn clear_regular_flush_schedule(&mut self) {
        self.regular_flush_scheduled = false;
    }

    pub fn record_regular_success(&mut self) {
        self.regular_failures = 0;
    }

    pub fn record_regular_failure(&mut self, has_retry: bool) -> StreamEmitFailureDecision {
        self.regular_failures = self.regular_failures.saturating_add(1);
        let failures = self.regular_failures;
        if failures >= STREAM_EMIT_FAILURE_STOP_LIMIT {
            self.regular_suppressed = true;
            self.regular_flush_scheduled = false;
            return StreamEmitFailureDecision::Stop { failures };
        }
        let schedule_retry = self.schedule_regular_flush();
        if failures >= STREAM_EMIT_FAILURE_FALLBACK_LIMIT {
            StreamEmitFailureDecision::FallbackToSnapshot {
                failures,
                fallback_started: failures == STREAM_EMIT_FAILURE_FALLBACK_LIMIT,
                schedule_retry,
            }
        } else {
            StreamEmitFailureDecision::RetryDelta {
                failures,
                install_snapshot_retry: !has_retry,
                schedule_retry,
            }
        }
    }

    pub fn authoritative_flush_is_scheduled(&self) -> bool {
        self.authoritative_flush_scheduled
    }

    pub fn schedule_authoritative_flush(&mut self) -> bool {
        if self.authoritative_flush_scheduled {
            false
        } else {
            self.authoritative_flush_scheduled = true;
            true
        }
    }

    pub fn clear_authoritative_flush_schedule(&mut self) {
        self.authoritative_flush_scheduled = false;
    }

    pub fn record_authoritative_success(&mut self) {
        self.authoritative_failures = 0;
    }

    pub fn record_authoritative_failure(&mut self) -> StreamEmitFailureDecision {
        self.authoritative_failures = self.authoritative_failures.saturating_add(1);
        let failures = self.authoritative_failures;
        if failures >= STREAM_EMIT_FAILURE_STOP_LIMIT {
            self.authoritative_flush_scheduled = false;
            StreamEmitFailureDecision::Stop { failures }
        } else {
            StreamEmitFailureDecision::RetryDelta {
                failures,
                install_snapshot_retry: false,
                schedule_retry: self.schedule_authoritative_flush(),
            }
        }
    }

    #[cfg(test)]
    pub fn suppress_regular_for_test(&mut self) {
        self.regular_suppressed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_delivery_falls_back_then_stops_at_domain_owned_limits() {
        let mut delivery = RuntimeStreamingDelivery::default();
        for expected in 1..STREAM_EMIT_FAILURE_FALLBACK_LIMIT {
            assert_eq!(
                delivery.record_regular_failure(false),
                StreamEmitFailureDecision::RetryDelta {
                    failures: expected,
                    install_snapshot_retry: true,
                    schedule_retry: expected == 1,
                }
            );
        }
        assert_eq!(
            delivery.record_regular_failure(true),
            StreamEmitFailureDecision::FallbackToSnapshot {
                failures: STREAM_EMIT_FAILURE_FALLBACK_LIMIT,
                fallback_started: true,
                schedule_retry: false,
            }
        );
        for _ in STREAM_EMIT_FAILURE_FALLBACK_LIMIT + 1..STREAM_EMIT_FAILURE_STOP_LIMIT {
            delivery.record_regular_failure(false);
        }
        assert_eq!(
            delivery.record_regular_failure(false),
            StreamEmitFailureDecision::Stop {
                failures: STREAM_EMIT_FAILURE_STOP_LIMIT,
            }
        );
        assert!(delivery.regular_is_suppressed());
    }

    #[test]
    fn authoritative_delivery_has_an_independent_retry_fence() {
        let mut delivery = RuntimeStreamingDelivery::default();
        assert!(matches!(
            delivery.record_authoritative_failure(),
            StreamEmitFailureDecision::RetryDelta {
                schedule_retry: true,
                ..
            }
        ));
        delivery.record_authoritative_success();
        delivery.clear_authoritative_flush_schedule();
        assert!(delivery.schedule_authoritative_flush());
        assert!(!delivery.schedule_authoritative_flush());
    }

    #[test]
    fn finishing_a_turn_preserves_the_existing_flush_schedule() {
        let mut delivery = RuntimeStreamingDelivery::default();
        assert!(delivery.schedule_regular_flush());
        delivery.record_regular_failure(false);

        delivery.finish_regular_turn();

        assert_eq!(delivery.regular_failure_count(), 0);
        assert!(!delivery.regular_is_suppressed());
        assert!(delivery.regular_flush_is_scheduled());
    }
}
