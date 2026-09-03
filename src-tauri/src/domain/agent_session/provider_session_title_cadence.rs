use std::time::Duration;

pub(crate) const PROVIDER_SESSION_TITLE_TICK_INTERVAL: Duration = Duration::from_secs(20);
pub(crate) const PROVIDER_SESSION_TITLE_REFRESH_TICKS: u64 = 15;

pub(crate) fn should_read_provider_session_title(tick: u64, has_title: bool) -> bool {
    !has_title || tick.is_multiple_of(PROVIDER_SESSION_TITLE_REFRESH_TICKS)
}

#[cfg(test)]
mod tests {
    use super::{
        should_read_provider_session_title, PROVIDER_SESSION_TITLE_REFRESH_TICKS,
        PROVIDER_SESSION_TITLE_TICK_INTERVAL,
    };

    #[test]
    fn test_provider_session_title_cadenceの基準tickは20秒で再読周期は15tick() {
        assert_eq!(PROVIDER_SESSION_TITLE_TICK_INTERVAL.as_secs(), 20);
        assert_eq!(PROVIDER_SESSION_TITLE_REFRESH_TICKS, 15);
    }

    #[test]
    fn test_provider_session_title_cadence_タイトル未取得なら毎tick読む() {
        for tick in 0..=30 {
            assert!(should_read_provider_session_title(tick, false));
        }
    }

    #[test]
    fn test_provider_session_title_cadence_タイトル取得済みなら15tickごとに読む() {
        for tick in 0..=30 {
            assert_eq!(
                should_read_provider_session_title(tick, true),
                matches!(tick, 0 | 15 | 30)
            );
        }
    }
}
