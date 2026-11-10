#[allow(clippy::enum_variant_names)]
pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
}
#[allow(clippy::enum_variant_names)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/connect/mod.rs"));
}
pub use generated::releash::client::v1 as rpc;
pub mod descriptor;
