use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Daemon(Option<PathBuf>),
    BackgroundWorker,
}

fn parse_arguments(arguments: &[OsString]) -> Option<Mode> {
    match arguments {
        [] => Some(Mode::Daemon(None)),
        [mode] if mode == "--internal-background-worker" => Some(Mode::BackgroundWorker),
        [mode] if mode == "--internal-daemon" => Some(Mode::Daemon(None)),
        [mode, directory] if mode == "--internal-daemon" || mode == "--data-dir" => {
            Some(Mode::Daemon(Some(directory.into())))
        }
        _ => None,
    }
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let code = match parse_arguments(&arguments) {
        Some(Mode::Daemon(directory)) => releashd::run_daemon(directory),
        Some(Mode::BackgroundWorker) => releashd::run_background_worker(),
        None => {
            eprintln!("usage: releashd [--data-dir <DIR>]");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
#[path = "backend_test.rs"]
mod backend_tests;
