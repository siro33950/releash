fn main() {
    println!("cargo:rerun-if-env-changed=SENTRY_DSN");
    if std::env::var("SENTRY_DSN").is_err() {
        println!("cargo:rustc-env=SENTRY_DSN=");
    }
    tauri_build::build()
}
