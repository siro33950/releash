pub(crate) mod launcher_impl;
pub(crate) mod scanner_impl;
pub(crate) mod settings_gateway_impl;

pub use launcher_impl::NativeEditorLauncherGateway;
pub use scanner_impl::MacInstalledEditorGateway;
pub use settings_gateway_impl::EditorSettingsConfigGateway;
