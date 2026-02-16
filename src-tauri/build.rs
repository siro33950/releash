fn main() {
    if std::env::var("SENTRY_DSN").is_err() {
        println!("cargo:rustc-env=SENTRY_DSN=");
    }
    tauri_build::build()
}
