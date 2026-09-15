#[path = "../src-tauri/src/domain/client_operation/policy.rs"]
mod policy;

fn main() {
    use policy::*;
    let proto = std::fs::read_to_string("proto/client.proto").unwrap();
    let commands = proto
        .split("message CommandRequest {")
        .nth(1)
        .unwrap()
        .split("message CommandResult")
        .next()
        .unwrap();
    println!("export const CLIENT_TRANSPORT = {{");
    println!("heartbeatIntervalMs: {HEARTBEAT_INTERVAL_MS}, heartbeatTimeoutMs: {HEARTBEAT_TIMEOUT_MS}, connectTimeoutMs: {CONNECT_TIMEOUT_MS}, reconnectIntervalMs: {RECONNECT_INTERVAL_MS}, sleepGapMs: {SLEEP_GAP_MS}, tickIntervalMs: {TICK_INTERVAL_MS}, maxPending: {MAX_UNACKNOWLEDGED_OPERATIONS},");
    println!("commands: {{");
    for line in commands
        .lines()
        .filter(|line| line.starts_with("    ") && line.contains(" = "))
    {
        let name = line.split_whitespace().nth(1).unwrap();
        let recovery = match recovery(name) {
            Recovery::Read => "read",
            Recovery::Idempotent => "idempotent",
            Recovery::CallerAttempt => "callerAttempt",
            Recovery::AtMostOnce => "atMostOnce",
            Recovery::Connection => "connection",
        };
        println!(
            "{name}: {{ recovery: \"{recovery}\", deadlineMs: {}, watch: {}, waitsForResult: {}, pollsResult: {}, disconnect: [\"{}\", \"{}\"] }},",
            deadline_ms(name),
            is_watch(name),
            waits_for_result(name),
            polls_result(name),
            disconnect_action(name, false),
            disconnect_action(name, true)
        );
    }
    println!("}} }} as const;");
}
