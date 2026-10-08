use std::sync::Arc;

pub(crate) fn run() -> i32 {
    let data_dir = match crate::infrastructure::platform::data_dir::resolve_data_dir(None) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("background worker data directory: {error}");
            return 1;
        }
    };
    let repository = Arc::new(super::wiring::build_repository_usecase_with_worktree_terminals(
        Arc::new(crate::adaptor::gateway::repository::worktree_terminal::NoopWorktreeTerminalGateway),
        Arc::new(crate::usecase::worktree_operation::WorktreeOperations::new(Arc::new(
            crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks::new(&data_dir),
        ))),
    ));
    let scanner = crate::adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
        repository,
        Arc::new(super::wiring::build_code_usecase()),
    );
    match crate::adaptor::gateway::shared::background_worker::serve(&scanner) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("background worker failed: {error}");
            1
        }
    }
}
