pub(crate) const SUBSCRIPTION_ID_MAX_BYTES: usize = 128;

pub(crate) fn valid_subscription_id(id: &str) -> bool {
    id.len() <= SUBSCRIPTION_ID_MAX_BYTES && !id.is_empty()
}

#[cfg(test)]
#[path = "client_stream_test.rs"]
mod client_stream_tests;
