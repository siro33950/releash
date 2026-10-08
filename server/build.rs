#[cfg(not(test))]
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
}

#[cfg(not(test))]
fn generate_client_protocol() {
    println!("cargo:rerun-if-changed=../proto/client.proto");
    println!("cargo:rerun-if-changed=../proto/client_options.proto");
    let directory = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut config = prost_build::Config::new();
    config.boxed(".releash.client.v1.Push.event.workflow_execution_changed");
    for message in ["CommandRequest", "CommandResult"] {
        config.message_attribute(
            format!(".releash.client.v1.{message}"),
            "#[cfg(any(test, feature = \"test-support\"))]",
        );
    }

    config.protoc_executable(protoc_bin_vendored::protoc_bin_path().expect("bundled protoc"));
    config.file_descriptor_set_path(directory.join("client_descriptor.bin"));
    let descriptors = config
        .load_fds(&["../proto/client.proto"], &["../proto"])
        .expect("client protocol descriptor");
    config
        .compile_fds(descriptors.clone())
        .expect("client protocol");
    connectrpc_build::Config::new()
        .files(&["client.proto"])
        .descriptor_set(directory.join("client_descriptor.bin"))
        .out_dir(directory.join("connect"))
        .include_file("mod.rs")
        .compile()
        .expect("Connect protocol");
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
    ] {
        let descriptor = messages.iter().find(|item| item.name() == message).unwrap();
        let harness_only = "#[cfg(any(test, feature = \"test-support\"))] ";
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
        let mut decode = format!("{decode_test}pub(crate) trait {message}Decode {{ fn into_value(self) -> Result<(&'static str, serde_json::Value), String>; }} {decode_test}impl {message}Decode for {message} {{ fn into_value(self) -> Result<(&'static str, serde_json::Value), String> {{ match self.command.ok_or(\"Missing command\")? {{\n");
        let mut encode = format!("{encode_test}pub(crate) trait {message}Encode {{ fn from_value(name: &str, value: serde_json::Value) -> Result<Self, String> where Self: Sized; }} {encode_test}impl {message}Encode for {message} {{ fn from_value(name: &str, value: serde_json::Value) -> Result<Self, String> {{ Ok(Self {{ command: Some(match name {{\n");
        let mut command_names = format!("pub(crate) trait CommandName {{ fn name(&self) -> &'static str; }} impl CommandName for {module}::Command {{ fn name(&self) -> &'static str {{ match self {{\n");
        let mut names = String::from(
            "#[cfg(any(test, feature = \"test-support\"))] pub const COMMAND_NAMES: &[&str] = &[\n",
        );
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
            names.push_str(&format!("{name:?},\n"));
            command_names.push_str(&format!("Self::{variant}(..) => {name:?},\n"));
            decode.push_str(&format!("{module}::Command::{variant}(value) => Ok(({name:?}, from_message({type_name:?}, &value)?)),\n"));
            encode.push_str(&format!(
                "{name:?} => {module}::Command::{variant}(to_message({type_name:?}, value)?),\n"
            ));
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
    let pool = prost_reflect::DescriptorPool::decode(
        std::fs::read(directory.join("client_descriptor.bin"))
            .unwrap()
            .as_slice(),
    )
    .expect("client descriptors with options");
    let scope_extension = pool
        .get_extension_by_name("releash.client.v1.scope")
        .expect("scope option");
    let reflected_service = pool
        .get_service_by_name("releash.client.v1.ClientService")
        .unwrap();
    let scope_enum = pool
        .get_enum_by_name("releash.client.v1.Scope")
        .expect("Scope enum");
    let unspecified = scope_enum
        .get_value_by_name("SCOPE_UNSPECIFIED")
        .expect("unspecified scope")
        .number();
    let mut scopes = String::from(
        "pub(crate) fn method_scopes(method: &str) -> Option<&'static [crate::adaptor::presenter::client::Scope]> { match method {\n",
    );
    for method in reflected_service.methods() {
        let value = method
            .options()
            .get_extension(&scope_extension)
            .into_owned();
        let prost_reflect::Value::List(values) = value else {
            panic!("scope must be repeated");
        };
        let values: Vec<i32> = values
            .into_iter()
            .map(|value| match value {
                prost_reflect::Value::EnumNumber(value) => value,
                _ => panic!("invalid scope"),
            })
            .collect();
        assert!(
            valid_method_scopes(&values, unspecified, |value| scope_enum
                .get_value(value)
                .is_some()),
            "{} must have nonempty, specified, unique scopes",
            method.name()
        );
        let variants = values
            .iter()
            .map(|value| {
                let name = scope_enum
                    .get_value(*value)
                    .unwrap()
                    .name()
                    .strip_prefix("SCOPE_")
                    .unwrap()
                    .to_ascii_lowercase();
                let variant = name
                    .split('_')
                    .map(|part| {
                        let mut chars = part.chars();
                        chars.next().unwrap().to_uppercase().to_string() + chars.as_str()
                    })
                    .collect::<String>();
                format!("crate::adaptor::presenter::client::Scope::{variant}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        scopes.push_str(&format!("{:?} => Some(&[{variants}]),\n", method.name()));
    }
    scopes.push_str("_ => None, } }\n");
    std::fs::write(directory.join("client_scopes.rs"), scopes).expect("write method scopes");
    let mut handlers = String::from("impl rpc::ClientService for ClientApiDeps {\n");
    let mut calls = String::from("pub fn call(client: &rpc::ClientServiceClient<connectrpc::client::HttpClient>, command: wire::command_request::Command) -> futures_util::future::BoxFuture<'_ , Result<wire::command_result::Command, connectrpc::ConnectError>> { match command {\n");
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
        handlers.push_str(&format!("async fn {name}<'a>(&'a self, _ctx: connectrpc::RequestContext, request: connectrpc::ServiceRequest<'_, rpc::{input}>) -> connectrpc::ServiceResult<impl connectrpc::Encodable<rpc::{output}> + Send + use<'a>> {{ let args = to_wire(&request.to_owned_message())?; let result = self.execute(_ctx.deadline(), wire::command_request::Command::{variant}(args)).await?; match result {{ wire::command_result::Command::{variant}(result) => Ok(connectrpc::Response::new(to_rpc::<rpc::{output}>(&result)?)), _ => Err(crate::adaptor::presenter::connect::classified_error(crate::adaptor::presenter::error::AppError::new(\"Mismatched command result\"))) }} }}\n"));
        calls.push_str(&format!("wire::command_request::Command::{variant}(args) => Box::pin(async move {{ let result = client.{name}(to_rpc(&args)?).await?; Ok(wire::command_result::Command::{variant}(to_wire(&result.into_owned())?)) }}),\n"));
    }
    handlers.push_str(include_str!("src/adaptor/controller/api/client_service.rs"));
    handlers.push_str("}\n");
    calls.push_str("} }\n");
    std::fs::write(directory.join("client_service.rs"), handlers).expect("write service handlers");
    std::fs::write(directory.join("client_calls.rs"), calls).expect("write native calls");
    println!("cargo:rerun-if-changed=src/adaptor/controller/api/client_service.rs");

    std::fs::write(directory.join("client_commands.rs"), code)
        .expect("write client command codecs");
}

pub(crate) fn valid_method_scopes(
    values: &[i32],
    unspecified: i32,
    known_scope: impl Fn(i32) -> bool,
) -> bool {
    !values.is_empty()
        && values
            .iter()
            .all(|value| *value != unspecified && known_scope(*value))
        && values
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == values.len()
}
