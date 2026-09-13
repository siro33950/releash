use super::*;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::invalid_request;
use crate::adaptor::controller::client::value;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::client::{convert, required};

pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    _deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        router.register_domain(
            &["report_frontend_error"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::ReportFrontendError(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let payload = required(args.payload, "payload")?;
                        commands::report_frontend_error_shared(
                            required(payload.error_type, "errorType")?,
                            required(payload.message, "message")?,
                            payload.stack,
                        );
                        value(())
                    }
                    .await?;
                    Ok(wire::command_result::Command::ReportFrontendError(result))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["report_mounted_xterm_count"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::ReportMountedXtermCount(args) = command
                    else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        commands::report_mounted_xterm_count_shared(convert(required(
                            args.count, "count",
                        )?)?);
                        value(())
                    }
                    .await?;
                    Ok(wire::command_result::Command::ReportMountedXtermCount(
                        result,
                    ))
                })
            }),
        );
    }
    {
        router.register_domain(
            &["report_usage_event"],
            Box::new(move |command| {
                Box::pin(async move {
                    let wire::command_request::Command::ReportUsageEvent(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        commands::report_usage_event_shared(convert(required(args.name, "name")?)?);
                        value(())
                    }
                    .await?;
                    Ok(wire::command_result::Command::ReportUsageEvent(result))
                })
            }),
        );
    }
}
