//! 層をまたぐ汎用ユーティリティ（特定ドメインに属さない純粋関数）。

/// 現在時刻を Unix epoch からの秒数として返す。
///
/// システム時刻が epoch より前などで変換できない場合は、既存呼び出し元の挙動に合わせて
/// `0.0` を返す。
pub fn unix_timestamp_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
