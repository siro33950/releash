pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
    mod values;
}
mod generated {
    include!(concat!(env!("OUT_DIR"), "/connect/mod.rs"));
}
pub use generated::releash::client::v1 as rpc;
pub mod compatibility;
pub mod descriptor;
pub mod discovery;
