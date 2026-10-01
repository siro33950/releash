//! 持ち主が取得した値の状態。
//!
//! 取得に失敗しても最後に取れた値を残し、初回の取得中・初回の失敗・項目なしを区別する。

/// 最後に取れた値と、直近の取得の失敗。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fetched<T, E = crate::domain::failure::WorkFailure> {
    pub value: Option<T>,
    pub error: Option<E>,
}

impl<T, E> Default for Fetched<T, E> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchState {
    Loading,
    InitialFailed,
    Empty,
    Ready,
    RefreshFailed,
}

impl<T, E> Fetched<T, E> {
    pub fn ready(value: T) -> Self {
        Self {
            value: Some(value),
            error: None,
        }
    }

    /// 取得の結果を記録する。失敗のときは最後に取れた値を残す。
    pub fn record(&mut self, result: Result<T, E>) {
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn loaded(&self) -> bool {
        self.value.is_some()
    }

    pub fn state(&self, empty: bool) -> FetchState {
        match (self.loaded(), self.error.is_some(), empty) {
            (false, false, _) => FetchState::Loading,
            (false, true, _) => FetchState::InitialFailed,
            (true, true, _) => FetchState::RefreshFailed,
            (true, false, true) => FetchState::Empty,
            (true, false, false) => FetchState::Ready,
        }
    }
}

#[cfg(test)]
#[path = "fetched_test.rs"]
mod fetched_tests;
