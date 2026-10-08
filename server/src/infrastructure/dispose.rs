//! ブロッキングし得る資源破棄を呼び出し元スレッドから隔離するユーティリティ。

/// `value` の drop を使い捨てスレッドで実行する。
///
/// FSEvents watcher の drop は run loop の停止待ちで長時間（最悪無期限に）ブロック
/// し得るため、メインスレッドや async worker 上では実行しない（#1641）。
/// スレッド生成に失敗した場合のみ、その場で drop する。
pub fn dispose_in_background<T: Send + 'static>(thread_name: &str, value: T) {
    let _ = std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || drop(value));
}

#[cfg(test)]
#[path = "dispose_test.rs"]
mod dispose_tests;
