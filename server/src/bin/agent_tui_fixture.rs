fn main() {
    if std::env::args().any(|argument| argument == "--version") {
        println!("releash-agent-tui-fixture {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    releashd::test_support::agent_tui_fixture::run_process();
}
