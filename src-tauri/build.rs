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
    for field in [
        "Push.event.workflow_execution_changed",
        "CommandError.variant.application",
        "CurrentShutdownResultDtoV1.variant.current",
        "ApplicationQuitOutcomeDtoV1.variant.previous_shutdown_reconciliation_required",
        "CommandResponse.outcome.result",
        "Envelope.body.request",
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
            ""
        } else {
            harness_only
        };
        let encode_test = if message == "CommandRequest" {
            harness_only
        } else {
            "#[cfg(test)] "
        };
        let mut decode = format!("impl {message} {{ {decode_test}pub(crate) fn into_value(self) -> Result<(&'static str, serde_json::Value), String> {{ match self.{oneof}.ok_or(\"Missing {oneof}\")? {{\n");
        let mut encode = format!("impl {message} {{ {encode_test}pub(crate) fn from_value(name: &str, value: serde_json::Value) -> Result<Self, String> {{ Ok(Self {{ {oneof}: Some(match name {{\n");
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
        if message == "CommandRequest" {
            encode.push_str(
                ", request_id: String::new(), instance_id: String::new(), recover: false, predecessors: Vec::new(), deadline_unix_ms: 0, user_retry: false, successors: Vec::new()",
            );
        }
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
    config
        .compile_fds(descriptors)
        .expect("generate client protocol");
    std::fs::write(directory.join("client_commands.rs"), code)
        .expect("write client command codecs");
}
