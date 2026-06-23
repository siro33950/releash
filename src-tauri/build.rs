fn main() {
    println!("cargo:rerun-if-env-changed=OTLP_ENDPOINT");
    if std::env::var("OTLP_ENDPOINT").is_err() {
        println!("cargo:rustc-env=OTLP_ENDPOINT=");
    }
    println!("cargo:rerun-if-env-changed=NEW_RELIC_LICENSE_KEY");
    if std::env::var("NEW_RELIC_LICENSE_KEY").is_err() {
        println!("cargo:rustc-env=NEW_RELIC_LICENSE_KEY=");
    }
    tauri_build::build()
}
