use super::*;
struct Logger(parking_lot::Mutex<Vec<String>>);
static LOGGER: Logger = Logger(parking_lot::Mutex::new(Vec::new()));
impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().push(record.args().to_string());
    }
    fn flush(&self) {}
}
#[test]
fn test_復元失敗の記録_初回適用の結果と設定変更の入口の失敗を記録し成功は記録しない() {
    // Given
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Error);
    let wrapped = LogFailure(());
    // When
    wrapped.record(&Err::<(), _>("initial restore failure marker"));
    assert_eq!(
        wrapped.call(|_| Err::<(), _>("settings restore failure marker")),
        Err("settings restore failure marker")
    );
    wrapped.record(&Ok::<_, &str>("success marker"));
    // Then
    let logs = LOGGER.0.lock();
    assert_eq!(
        logs.iter()
            .filter(|value| *value == "initial restore failure marker")
            .count(),
        1
    );
    assert_eq!(
        logs.iter()
            .filter(|value| *value == "settings restore failure marker")
            .count(),
        1
    );
    assert!(!logs.iter().any(|value| value == "success marker"));
}
