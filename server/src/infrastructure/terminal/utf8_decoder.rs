pub(crate) fn decode_utf8_chunk(raw_chunk: &[u8], pending: &mut Vec<u8>) -> Option<String> {
    pending.extend_from_slice(raw_chunk);
    let mut decoded = String::new();
    let mut consumed = 0;

    loop {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(valid) => {
                decoded.push_str(valid);
                pending.clear();
                break;
            }
            Err(error) => {
                let valid_end = consumed + error.valid_up_to();
                decoded.push_str(
                    std::str::from_utf8(&pending[consumed..valid_end])
                        .expect("UTF-8 validator reported a valid prefix"),
                );
                let Some(error_len) = error.error_len() else {
                    pending.drain(..valid_end);
                    break;
                };
                decoded.push('�');
                consumed = valid_end + error_len;
            }
        }
    }

    (!decoded.is_empty()).then_some(decoded)
}

#[cfg(test)]
#[path = "utf8_decoder_test.rs"]
mod utf8_decoder_tests;
