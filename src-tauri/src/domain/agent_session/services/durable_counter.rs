pub const MAX_DURABLE_COUNTER: u64 = i64::MAX as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableCounterExhausted;

pub fn advance_durable_counter(value: u64) -> Result<u64, DurableCounterExhausted> {
    let next = value.checked_add(1).ok_or(DurableCounterExhausted)?;
    if next > MAX_DURABLE_COUNTER {
        return Err(DurableCounterExhausted);
    }
    Ok(next)
}

pub fn add_durable_count(value: usize, delta: usize) -> Result<usize, DurableCounterExhausted> {
    let next = value.checked_add(delta).ok_or(DurableCounterExhausted)?;
    if u64::try_from(next).map_or(true, |next| next > MAX_DURABLE_COUNTER) {
        return Err(DurableCounterExhausted);
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_counters_close_at_the_authority_capacity() {
        assert_eq!(advance_durable_counter(7), Ok(8));
        assert_eq!(
            advance_durable_counter(MAX_DURABLE_COUNTER),
            Err(DurableCounterExhausted)
        );
        assert_eq!(add_durable_count(7, 2), Ok(9));
    }
}
