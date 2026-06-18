pub(crate) mod certificate_impl;
pub(crate) mod network_impl;
pub(crate) mod qr_impl;

pub use network_impl::SystemNetworkInterfaceGateway;
pub use qr_impl::QrCodeRenderGateway;
