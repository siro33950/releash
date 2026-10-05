async fn get_server_info<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    _request: connectrpc::ServiceRequest<'_, rpc::Unit>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::ServerInfo> + Send + use<'a>> {
    let info = crate::adaptor::presenter::daemon::server_info(self.daemon.info().await,
        std::env::var("RELEASH_DAEMON_LAUNCH_ID").unwrap_or_default());
    connectrpc::Response::ok(to_rpc::<rpc::ServerInfo>(&info)?)
}

async fn open_state_stream(
    &self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::OpenStateStreamRequest>,
) -> connectrpc::ServiceResult<
    connectrpc::ServiceStream<
        impl connectrpc::Encodable<rpc::StateSubscriptionEvent> + Send + use<>,
    >,
> {
    let request: wire::OpenStateStreamRequest = to_wire(&request.to_owned_message())?;
    if !crate::adaptor::controller::api::client_stream::valid_subscription_id(&request.client_id) {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::usecase::state_subscription::SubscriptionError::InvalidId,
        ));
    }
    let stream = self
        .subscriptions()?
        .stream_wire(request.client_id)
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::stream_ok(Box::pin(stream))
}

async fn start_state_subscription<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::StartStateSubscriptionRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let request: wire::StartStateSubscriptionRequest = to_wire(&request.to_owned_message())?;
    if !crate::adaptor::controller::api::client_stream::valid_subscription_id(&request.subscription_id) {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::usecase::state_subscription::SubscriptionError::InvalidId,
        ));
    }
    let target = crate::usecase::state_subscription::SubscriptionTarget::from_parts(
        &request.target,
        &request.args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(crate::adaptor::presenter::connect::classified_error)?;
    let version = request.version.map(|v| (v.epoch, v.sequence));
    let cursor = version
        .as_ref()
        .map(|(epoch, sequence)| (epoch.as_str(), *sequence));
    self.subscriptions()?
        .start_subscription(&request.client_id, &target, &request.subscription_id, cursor)
        .await
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::ok(rpc::Unit::default())
}

async fn stop_state_subscription<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::StopStateSubscriptionRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let request: wire::StopStateSubscriptionRequest = to_wire(&request.to_owned_message())?;
    self.subscriptions()?
        .stop_subscription(&request.subscription_id)
        .await
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::ok(rpc::Unit::default())
}

async fn report_terminal_processed<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::ReportTerminalProcessedRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let request: wire::ReportTerminalProcessedRequest = to_wire(&request.to_owned_message())?;
    if request.units as usize != crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::report_units() {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::invalid_request(
                "Invalid terminal processed units",
            ),
        ));
    }
    self.subscriptions()?
        .terminal_processed(&request.subscription_id, request.units as usize)
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::ok(rpc::Unit::default())
}
