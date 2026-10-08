use super::{
    client as wire,
    connect_wire::{rpc, to_rpc, to_wire},
};
include!(concat!(env!("OUT_DIR"), "/client_calls.rs"));
