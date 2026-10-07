fn main() {
    let proto = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("../proto");
    println!("cargo:rerun-if-changed={}", proto.display());
    let directory = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path().expect("bundled protoc"));
    config.boxed(".releash.client.v1.Push.event.workflow_execution_changed");
    config.file_descriptor_set_path(directory.join("client_descriptor.bin"));
    config
        .compile_protos(&[proto.join("client.proto")], &[proto])
        .expect("client protocol");
    connectrpc_build::Config::new()
        .files(&["client.proto"])
        .descriptor_set(directory.join("client_descriptor.bin"))
        .out_dir(directory.join("connect"))
        .include_file("mod.rs")
        .compile()
        .expect("Connect client");
}
