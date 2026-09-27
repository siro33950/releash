use crate::adaptor::presenter::connect::{invalid_request, invalid_response};

include!(concat!(env!("OUT_DIR"), "/connect/mod.rs"));
pub use releash::client::v1 as rpc;

pub(crate) fn to_wire<T: prost::Message + Default>(
    value: &impl buffa::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode(buffa::Message::encode_to_vec(value).as_slice())
        .map_err(|error| invalid_request(error.to_string()))
}

pub(crate) fn to_rpc<T: buffa::Message>(
    value: &impl prost::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode_from_slice(&prost::Message::encode_to_vec(value))
        .map_err(|error| invalid_response(error.to_string()))
}
