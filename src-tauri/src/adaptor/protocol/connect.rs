include!(concat!(env!("OUT_DIR"), "/connect/mod.rs"));
pub use releash::client::v1 as rpc;

pub(crate) fn to_wire<T: prost::Message + Default>(
    value: &impl buffa::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode(buffa::Message::encode_to_vec(value).as_slice())
        .map_err(|error| connectrpc::ConnectError::invalid_argument(error.to_string()))
}

pub(crate) fn to_rpc<T: buffa::Message>(
    value: &impl prost::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode_from_slice(&prost::Message::encode_to_vec(value))
        .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))
}
