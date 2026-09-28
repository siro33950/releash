async fn get_server_info<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    _request: connectrpc::ServiceRequest<'_, rpc::Unit>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::ServerInfo> + Send + use<'a>> {
    let _permit = self.request_permit()?;
    connectrpc::Response::ok(to_rpc::<rpc::ServerInfo>(&wire::ServerInfo {
        launch_id: std::env::var("RELEASH_DAEMON_LAUNCH_ID").unwrap_or_default(),
        release: env!("CARGO_PKG_VERSION").into(),
        desktop_settings: self
            .desktop_settings()
            .map_err(crate::adaptor::presenter::connect::classified_error)?,
    })?)
}

async fn subscribe_push(
    &self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::SubscribePushRequest>,
) -> connectrpc::ServiceResult<
    connectrpc::ServiceStream<impl connectrpc::Encodable<rpc::Push> + Send + use<>>,
> {
    let request: wire::SubscribePushRequest = to_wire(&request.to_owned_message())?;
    let id = request.subscription_id;
    if !crate::adaptor::controller::api::client_stream::valid_subscription_id(&id, true) {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::invalid_request(
                format!("Identifier exceeds {} bytes", crate::adaptor::controller::api::client_stream::SUBSCRIPTION_ID_MAX_BYTES),
            ),
        ));
    }
    let id = if id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        id
    };
    let subscription = self.watcher.subscribe(id).map_err(watch_error)?;
    let stream = futures_util::stream::unfold(
        (self.push.subscribe(), subscription, true),
        |(mut push, permit, initial)| async move {
            let event = if initial {
                crate::adaptor::presenter::push::resync()
            } else {
                match push.recv().await {
                    Ok(bytes) => bytes,
                    Err(ClientPushError::Lagged(_)) => {
                        push.resubscribe();
                        crate::adaptor::presenter::push::resync()
                    }
                    Err(ClientPushError::Closed) => return None,
                }
            };
            Some((Ok(EncodedPush(event)), (push, permit, false)))
        },
    );
    connectrpc::Response::stream_ok(Box::pin(stream))
}

async fn watch_files<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::WatchFilesRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::ResultUint64> + Send + use<'a>> {
    let request: wire::WatchFilesRequest = to_wire(&request.to_owned_message())?;
    let args = request.request.ok_or_else(|| {
        crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::invalid_request("Missing watch request"),
        )
    })?;
    connectrpc::Response::ok(
        self.watch(
            _ctx.deadline(),
            request.subscription_id,
            crate::adaptor::controller::client::required(args.path, "path")
                .map_err(command_error)?,
            false,
        )
        .await?,
    )
}

async fn watch_git_directory<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::WatchGitDirectoryRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::ResultUint64> + Send + use<'a>> {
    let request: wire::WatchGitDirectoryRequest = to_wire(&request.to_owned_message())?;
    let args = request.request.ok_or_else(|| {
        crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::invalid_request("Missing watch request"),
        )
    })?;
    connectrpc::Response::ok(
        self.watch(
            _ctx.deadline(),
            request.subscription_id,
            crate::adaptor::controller::client::required(args.repo_path, "repoPath")
                .map_err(command_error)?,
            true,
        )
        .await?,
    )
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
    if !crate::adaptor::controller::api::client_stream::valid_subscription_id(&request.client_id, false) {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::usecase::state_subscription::SubscriptionError::InvalidId,
        ));
    }
    let stream = self
        .state_presenter()?
        .stream_wire(self.state_subscriptions()?.clone(), request.client_id)
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::stream_ok(Box::pin(stream))
}

async fn start_state_subscription<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::StartStateSubscriptionRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let _permit = self.request_permit()?;
    let request: wire::StartStateSubscriptionRequest = to_wire(&request.to_owned_message())?;
    let target = crate::usecase::state_subscription::SubscriptionTarget::from_parts(
        &request.target,
        &request.args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(crate::adaptor::presenter::connect::classified_error)?;
    let version = request.version.map(|v| (v.epoch, v.sequence));
    let cursor = version
        .as_ref()
        .map(|(epoch, sequence)| (epoch.as_str(), *sequence));
    let subscriptions = self.state_subscriptions()?;
    subscriptions
        .start_subscription(&request.client_id, &target, request.terminal_input_id.as_deref(), cursor)
        .await
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::ok(rpc::Unit::default())
}

async fn stop_state_subscription<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::StopStateSubscriptionRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let _permit = self.request_permit()?;
    let request: wire::StopStateSubscriptionRequest = to_wire(&request.to_owned_message())?;
    let target = crate::usecase::state_subscription::SubscriptionTarget::from_parts(
        &request.target,
        &request.args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(crate::adaptor::presenter::connect::classified_error)?;
    let subscriptions = self.state_subscriptions()?;
    subscriptions
        .stop_subscription(&request.client_id, &target)
        .await
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::ok(rpc::Unit::default())
}

async fn report_terminal_processed<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::ReportTerminalProcessedRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let _permit = self.request_permit()?;
    let request: wire::ReportTerminalProcessedRequest = to_wire(&request.to_owned_message())?;
    let target = crate::usecase::state_subscription::SubscriptionTarget::from_parts(
        "terminal",
        &request.args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(crate::adaptor::presenter::connect::classified_error)?;
    if request.units as usize != self.state_presenter()?.terminal_report_units() {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::invalid_request(
                "Invalid terminal processed units",
            ),
        ));
    }
    self.state_subscriptions()?
        .terminal_processed(
            &request.client_id,
            &target,
            request.units as usize,
        )
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    connectrpc::Response::ok(rpc::Unit::default())
}
