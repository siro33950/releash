#![allow(dead_code)]

pub(crate) mod error;
pub(crate) mod gateway;
pub(crate) mod services;
pub(crate) mod value_objects;

pub use error::RemoteAccessError;
pub use gateway::{CertificateGateway, NetworkInterfaceGateway, QrRenderGateway};
pub use value_objects::{DetectedInterface, QrCodeResult, VpnInterface};
