pub(crate) mod settings_gateway_impl;
pub(crate) mod webhook_sender_impl;

pub use settings_gateway_impl::{
    config_notify_to_domain, domain_notify_to_config, NotificationSettingsConfigGateway,
};
pub use webhook_sender_impl::ReqwestWebhookSenderGateway;
