use crate::infrastructure::local_api::discovery::LocalApiDiscovery;

pub fn discovery(port: u16, token: &str) -> LocalApiDiscovery {
    LocalApiDiscovery {
        port: port.into(),
        token: token.to_string(),
        daemon_id: "test-instance".to_string(),
        pid: 42,
        process_started_at: 123,
        ..Default::default()
    }
}
