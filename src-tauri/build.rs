fn main() {
    #[allow(unused_mut)]
    let mut attributes = tauri_build::Attributes::new();
    #[cfg(windows)]
    {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        add_manifest();
    }
    tauri_build::try_build(attributes).unwrap();
}

#[cfg(windows)]
fn add_manifest() {
    static MANIFEST_FILE: &str = "windows-app-manifest.xml";
    let manifest = std::env::current_dir().unwrap().join(MANIFEST_FILE);
    println!("cargo:rerun-if-changed={}", manifest.display());
    // cargo:rustc-link-arg（-binsなし）で全バイナリ（テスト含む）にマニフェストを埋め込む
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest.to_str().unwrap()
    );
    println!("cargo:rustc-link-arg=/WX");
}
