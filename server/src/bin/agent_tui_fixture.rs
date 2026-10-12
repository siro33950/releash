#[path = "agent_tui_fixture/runtime.rs"]
mod runtime;

fn main() {
    if std::env::args().any(|argument| argument == "--version") {
        println!("releash-agent-tui-fixture {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    runtime::run_process();
}
