pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn samples_current_process() {
        let observer = ProcessResourceObserver::default();
        let sample = observer.sample().unwrap();
        assert!(sample.rss_bytes > 0);
        assert!(sample.cpu_percent >= 0.0);
    }
}
