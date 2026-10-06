thread_local! {
    static GIT_PROGRAM: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn git_program() -> std::path::PathBuf {
    GIT_PROGRAM.with_borrow(|program| program.clone().unwrap_or_else(|| "git".into()))
}

pub fn set_git_program(program: Option<std::path::PathBuf>) {
    GIT_PROGRAM.set(program);
}
