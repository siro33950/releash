fn main() {
    generate_client_protocol();
    println!("cargo:rerun-if-env-changed=OTLP_ENDPOINT");
    if std::env::var("OTLP_ENDPOINT").is_err() {
        println!("cargo:rustc-env=OTLP_ENDPOINT=");
    }
    println!("cargo:rerun-if-env-changed=NEW_RELIC_LICENSE_KEY");
    if std::env::var("NEW_RELIC_LICENSE_KEY").is_err() {
        println!("cargo:rustc-env=NEW_RELIC_LICENSE_KEY=");
    }
    #[cfg(feature = "desktop")]
    tauri_build::build()
}

fn generate_client_protocol() {
    println!("cargo:rerun-if-changed=../proto/client.proto");
    println!("cargo:rerun-if-changed=../proto/client_options.proto");
    let directory = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut config = prost_build::Config::new();
    for message in ["CommandRequest", "CommandResult"] {
        config.message_attribute(
            format!(".releash.client.v1.{message}"),
            "#[cfg(any(test, all(debug_assertions, feature = \"desktop\")))]",
        );
    }
    for field in [
        "Push.event.workflow_execution_changed",
        "CommandError.variant.application",
    ] {
        config.boxed(format!(".releash.client.v1.{field}"));
    }
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path().expect("bundled protoc"));
    config.file_descriptor_set_path(directory.join("client_descriptor.bin"));
    let descriptors = config
        .load_fds(&["../proto/client.proto"], &["../proto"])
        .expect("client protocol descriptor");
    let messages: Vec<_> = descriptors
        .file
        .iter()
        .filter(|file| file.package() == "releash.client.v1")
        .flat_map(|file| &file.message_type)
        .collect();
    let mut code = String::new();
    for (message, module) in [
        ("CommandRequest", "command_request"),
        ("CommandResult", "command_result"),
        ("Push", "push"),
    ] {
        let descriptor = messages.iter().find(|item| item.name() == message).unwrap();
        let oneof = if message == "Push" {
            "event"
        } else {
            "command"
        };
        let enum_name = if message == "Push" {
            "Event"
        } else {
            "Command"
        };
        let harness_only = "#[cfg(any(test, all(debug_assertions, feature = \"desktop\")))] ";
        let decode_test = if message == "CommandRequest" {
            "#[cfg(test)] "
        } else {
            harness_only
        };
        let encode_test = if message == "CommandRequest" {
            harness_only
        } else {
            "#[cfg(test)] "
        };
        let mut decode = format!("{harness_only}impl {message} {{ {decode_test}pub(crate) fn into_value(self) -> Result<(&'static str, serde_json::Value), String> {{ match self.{oneof}.ok_or(\"Missing {oneof}\")? {{\n");
        let mut encode = format!("{harness_only}impl {message} {{ {encode_test}pub(crate) fn from_value(name: &str, value: serde_json::Value) -> Result<Self, String> {{ Ok(Self {{ {oneof}: Some(match name {{\n");
        let mut command_names = format!("impl {module}::{enum_name} {{ pub(crate) fn name(&self) -> &'static str {{ match self {{\n");
        let mut names = String::from("#[cfg(test)] pub const COMMAND_NAMES: &[&str] = &[\n");
        for field in descriptor
            .field
            .iter()
            .filter(|field| field.oneof_index.is_some())
        {
            let name = field.name();
            let variant = name
                .split('_')
                .map(|word| {
                    let mut chars = word.chars();
                    chars.next().unwrap().to_uppercase().to_string() + chars.as_str()
                })
                .collect::<String>();
            let type_name = field.type_name().trim_start_matches('.');
            let public_name = if message == "Push" {
                name.replace('_', "-")
            } else {
                name.to_string()
            };
            names.push_str(&format!("{name:?},\n"));
            command_names.push_str(&format!("Self::{variant}(..) => {public_name:?},\n"));
            decode.push_str(&format!("{module}::{enum_name}::{variant}(value) => Ok(({public_name:?}, from_message({type_name:?}, &value)?)),\n"));
            encode.push_str(&format!("{public_name:?} => {module}::{enum_name}::{variant}(to_message({type_name:?}, value)?),\n"));
        }
        decode.push_str("} } }\n");
        encode.push_str("_ => return Err(format!(\"Unknown protocol name: {name}\")), })");
        encode.push_str("}) } }\n");
        command_names.push_str("} } }\n");
        if message == "CommandRequest" {
            code.push_str(&command_names);
        }
        code.push_str(&decode);
        code.push_str(&encode);
        if message == "CommandRequest" {
            names.push_str("];\n");
            code.push_str(&names);
        }
    }
    let service = descriptors
        .file
        .iter()
        .flat_map(|file| &file.service)
        .find(|service| service.name() == "ClientService")
        .expect("ClientService");
    let mut handlers = String::from("impl rpc::ClientService for ClientApiDeps {\n");
    let mut calls = String::from("pub(crate) fn call(client: &rpc::ClientServiceClient<connectrpc::client::HttpClient>, command: wire::command_request::Command) -> futures_util::future::BoxFuture<'_ , Result<wire::command_result::Command, connectrpc::ConnectError>> { match command {\n");
    let commands = messages
        .iter()
        .find(|message| message.name() == "CommandRequest")
        .unwrap();
    for method in &service.method {
        let Some(field) = commands
            .field
            .iter()
            .find(|field| field.type_name() == method.input_type() && field.oneof_index.is_some())
        else {
            continue;
        };
        if method.server_streaming() {
            continue;
        }
        let name = field.name();
        let variant = method.name();
        let input = method.input_type().rsplit('.').next().unwrap();
        let output = method.output_type().rsplit('.').next().unwrap();
        handlers.push_str(&format!("async fn {name}<'a>(&'a self, _ctx: connectrpc::RequestContext, request: connectrpc::ServiceRequest<'_, rpc::{input}>) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::{output}> + Send + use<'a>> {{ let args = to_wire(&request.to_owned_message())?; let result = self.execute(_ctx.deadline(), wire::command_request::Command::{variant}(args)).await?; let headers = response_headers(&result); match result {{ wire::command_result::Command::{variant}(result) => {{ let mut response = connectrpc::Response::new(to_rpc::<rpc::{output}>(&result)?); response.headers = headers; Ok(response) }}, _ => Err(crate::adaptor::presenter::connect::classified_error(crate::adaptor::presenter::error::AppError::new(\"Mismatched command result\"))) }} }}\n"));
        calls.push_str(&format!("wire::command_request::Command::{variant}(args) => Box::pin(async move {{ let result = client.{name}(to_rpc(&args)?).await?; Ok(wire::command_result::Command::{variant}(to_wire(&result.into_owned())?)) }}),\n"));
    }
    handlers.push_str(include_str!("src/adaptor/controller/api/client_service.rs"));
    handlers.push_str("}\n");
    calls.push_str("} }\n");
    std::fs::write(directory.join("client_service.rs"), handlers).expect("write service handlers");
    std::fs::write(directory.join("client_calls.rs"), calls).expect("write native calls");
    println!("cargo:rerun-if-changed=src/adaptor/controller/api/client_service.rs");
    config
        .compile_fds(descriptors)
        .expect("generate client protocol");
    connectrpc_build::Config::new()
        .files(&["client.proto"])
        .descriptor_set(directory.join("client_descriptor.bin"))
        .out_dir(directory.join("connect"))
        .include_file("mod.rs")
        .compile()
        .expect("generate Connect services");
    std::fs::write(directory.join("client_commands.rs"), code)
        .expect("write client command codecs");
}
