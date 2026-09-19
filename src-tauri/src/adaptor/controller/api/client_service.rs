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
            .map_err(connectrpc::ConnectError::internal)?,
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
    super::client_stream::validate_identifier(&id)?;
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
                prost::Message::encode_to_vec(&wire::Push {
                    event: Some(wire::push::Event::Resync(wire::Unit {})),
                })
                .into()
            } else {
                match push.recv().await {
                    Ok(bytes) => bytes,
                    Err(ClientPushError::Lagged(_)) => {
                        push.resubscribe();
                        prost::Message::encode_to_vec(&wire::Push {
                            event: Some(wire::push::Event::Resync(wire::Unit {})),
                        })
                        .into()
                    }
                    Err(ClientPushError::Closed) => return None,
                }
            };
            Some((Ok(EncodedPush(event)), (push, permit, false)))
        },
    );
    connectrpc::Response::stream_ok(Box::pin(stream))
}

async fn subscribe_terminal_surfaces(
    &self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::SubscribeTerminalSurfacesRequest>,
) -> connectrpc::ServiceResult<
    connectrpc::ServiceStream<
        impl connectrpc::Encodable<rpc::TerminalSubscriptionEvent> + Send + use<>,
    >,
> {
    let terminal = self
        .terminal
        .as_ref()
        .ok_or_else(|| connectrpc::ConnectError::unavailable("Terminal unavailable"))?;
    let request: wire::SubscribeTerminalSurfacesRequest = to_wire(&request.to_owned_message())?;
    connectrpc::Response::stream_ok(terminal.subscribe(request.subscription_id)?)
}

async fn attach_terminal_surface<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::AttachSubscribedTerminalSurfaceRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::Unit> + Send + use<'a>> {
    let _permit = self.request_permit()?;
    self.dispatch
        .admit("attach_terminal_surface")
        .map_err(command_error)?;
    let terminal = self
        .terminal
        .as_ref()
        .ok_or_else(|| connectrpc::ConnectError::unavailable("Terminal unavailable"))?;
    let request: wire::AttachSubscribedTerminalSurfaceRequest =
        to_wire(&request.to_owned_message())?;
    terminal.attach(
        &request.subscription_id,
        request.stream_id,
        request.request.ok_or_else(|| {
            connectrpc::ConnectError::invalid_argument("Missing terminal request")
        })?,
    )?;
    connectrpc::Response::ok(rpc::Unit::default())
}

async fn watch_files<'a>(
    &'a self,
    _ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<'_, rpc::WatchFilesRequest>,
) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::ResultUint64> + Send + use<'a>> {
    let request: wire::WatchFilesRequest = to_wire(&request.to_owned_message())?;
    let args = request
        .request
        .ok_or_else(|| connectrpc::ConnectError::invalid_argument("Missing watch request"))?;
    connectrpc::Response::ok(
        self.watch(
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
    let args = request
        .request
        .ok_or_else(|| connectrpc::ConnectError::invalid_argument("Missing watch request"))?;
    connectrpc::Response::ok(
        self.watch(
            request.subscription_id,
            crate::adaptor::controller::client::required(args.repo_path, "repoPath")
                .map_err(command_error)?,
            true,
        )
        .await?,
    )
}
