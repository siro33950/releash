fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let code = match arguments.as_slice() {
        [mode] if mode == "--internal-background-worker" => releash_lib::run_background_worker(),
        [mode] if mode == "--internal-daemon" => releash_lib::run_daemon(None),
        [mode, directory] if mode == "--internal-daemon" => {
            releash_lib::run_daemon(Some(directory.into()))
        }
        _ => {
            eprintln!("usage: releash-backend --internal-daemon [DATA_DIR]");
            2
        }
    };
    std::process::exit(code);
}
