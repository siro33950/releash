use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Port(Mutex<Vec<String>>);
impl TelemetryPort for Port {
    fn report_frontend_error(&self, kind: &str, message: &str, stack: Option<&str>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("error:{kind}:{message}:{stack:?}"));
    }
    fn set_mounted_xterm_count(&self, count: u64) {
        self.0.lock().unwrap().push(format!("mounted:{count}"));
    }
    fn record_usage_event(&self, name: &str) {
        self.0.lock().unwrap().push(format!("usage:{name}"));
    }
}

#[test]
fn test_telemetry操作_引数とquery結果をportへ委譲する() {
    // Given
    let port = Port::default();
    let usecase = TelemetryUsecase::new(&port);
    // When
    usecase.report_frontend_error("type", "message", Some("stack"));
    usecase.set_mounted_xterm_count(3);
    usecase.record_usage_event("event");
    // Then
    assert_eq!(
        *port.0.lock().unwrap(),
        [
            "error:type:message:Some(\"stack\")",
            "mounted:3",
            "usage:event"
        ]
    );
}
