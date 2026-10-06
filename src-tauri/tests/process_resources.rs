use releash_lib::test_support::integration::platform::ProcessResourceObserver;
use releash_lib::test_support::integration::process::signal_process_group;

#[test]
fn test_プロセス資源_現在のプロセスを取得する() {
    let sample = ProcessResourceObserver::default().sample().unwrap();
    assert!(sample.rss_bytes > 0);
    assert!(sample.cpu_percent >= 0.0);
}

#[cfg(unix)]
#[test]
fn test_process_group_signal_存在しない安全なpgidは成功する() {
    signal_process_group(i32::MAX, 0).unwrap();
}
