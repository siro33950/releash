use crate::adaptor::controller::client::dispatch::ClientCommandDispatch;

pub fn dispatch() -> ClientCommandDispatch {
    ClientCommandDispatch::new(crate::usecase::daemon::DaemonUsecase::test_with_repository(
        crate::adaptor::gateway::daemon::serving(),
    ))
}
