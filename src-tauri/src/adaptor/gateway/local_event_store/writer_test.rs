pub(crate) mod tests {
    use super::super::*;
    fn request(critical: bool, bytes: usize) -> WriteJob {
        WriteJob {
            run: Box::new(|_| {}),
            critical,
            bytes,
        }
    }

    #[test]
    fn critical_lane_is_popped_first() {
        let queue = WriteQueue::new();
        assert!(queue.admit(request(false, 1)).is_ok());
        assert!(queue.admit(request(true, 1)).is_ok());
        assert!(queue.pop_blocking().unwrap().critical);
        assert!(!queue.pop_blocking().unwrap().critical);
    }

    #[test]
    fn lane_request_bounds_are_enforced() {
        let queue = WriteQueue::new();
        for _ in 0..CRITICAL_LANE_MAX_REQUESTS {
            assert!(queue.admit(request(true, 1)).is_ok());
        }
        assert!(queue.admit(request(true, 1)).is_err());
        // The normal lane still admits.
        assert!(queue.admit(request(false, 1)).is_ok());
    }

    #[test]
    fn lane_byte_bounds_are_enforced() {
        let queue = WriteQueue::new();
        assert!(queue
            .admit(request(false, NORMAL_LANE_MAX_BYTES - 1))
            .is_ok());
        assert!(queue.admit(request(false, 2)).is_err());
        assert!(queue.admit(request(false, 1)).is_ok());
    }

    #[test]
    fn close_after_drain_preserves_admitted_requests() {
        let queue = WriteQueue::new();
        assert!(queue.admit(request(false, 1)).is_ok());
        assert!(queue.admit(request(true, 1)).is_ok());

        queue.close_after_drain();

        assert!(queue.pop_blocking().unwrap().critical);
        assert!(!queue.pop_blocking().unwrap().critical);
        assert!(queue.pop_blocking().is_none());
        assert!(matches!(
            queue.admit(request(false, 1)),
            Err(AdmitRejection::Closed)
        ));
    }
}
