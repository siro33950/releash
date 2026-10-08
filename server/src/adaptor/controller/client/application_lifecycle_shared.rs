use super::*;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{invalid_request, outcome};
use crate::adaptor::presenter::client as wire;

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let process_port = deps.process_port.clone();
        let daemon = deps.daemon.clone();
        router.register_domain(
            &["stop_daemon"],
            Box::new(move |command| {
                let process_port = process_port.clone();
                let daemon = daemon.clone();
                Box::pin(async move {
                    let wire::command_request::Command::StopDaemon(_) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result =
                        async move { outcome(stop_daemon_shared(&daemon, &process_port).await) }
                            .await?;
                    Ok(wire::command_result::Command::StopDaemon(result))
                })
            }),
        );
    }
}
