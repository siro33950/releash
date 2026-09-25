#[test]
fn worker_entry() {
    if std::env::var_os("RELEASH_TEST_BACKGROUND_WORKER").is_some() {
        std::process::exit(super::run());
    }
}
