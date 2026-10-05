use super::*;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, required};
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
            &["request_application_quit"],
            Box::new(move |command| {
                let process_port = process_port.clone();
                let daemon = daemon.clone();
                Box::pin(async move {
                    let wire::command_request::Command::RequestApplicationQuit(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        outcome(
                            request_application_quit_shared(
                                &daemon,
                                &process_port,
                                convert(required(args.request, "request")?)?,
                            )
                            .await,
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::RequestApplicationQuit(
                        result,
                    ))
                })
            }),
        );
    }
}
