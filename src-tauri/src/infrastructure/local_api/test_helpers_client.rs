use super::*;
pub fn discovery(port: u16, token: &str) -> LocalApiDiscovery {
    LocalApiDiscovery {
        port,
        token: token.to_string(),
        instance_id: "test-instance".to_string(),
        pid: 42,
        process_started_at: 123,
    }
}
