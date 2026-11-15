use super::*;
#[test]
fn test_check表示_分類順と経過時間とログ対象を決める() {
    // Given
    let checks = Checks::new(
        [
            CheckBucket::Pass,
            CheckBucket::Pending,
            CheckBucket::Fail,
            CheckBucket::Skipping,
            CheckBucket::Cancel,
        ]
        .into_iter()
        .map(|bucket| Check {
            name: format!("{bucket:?}"),
            bucket,
            started_at: Some(10),
            completed_at: Some(250),
            job: Some(ActionsJobId(1)),
        })
        .collect(),
    );
    // When
    let listed = checks.listed();
    // Then
    assert_eq!(
        listed.iter().map(|c| c.bucket).collect::<Vec<_>>(),
        vec![CheckBucket::Fail, CheckBucket::Cancel, CheckBucket::Pending]
    );
    assert_eq!(listed[0].failure_seconds(), Some(240));
    assert_eq!(listed[0].log_job(), Some(ActionsJobId(1)));
    assert_eq!(listed[1].failure_seconds(), None);
    assert_eq!(listed[1].log_job(), None);
    assert_eq!(listed[2].running_since(), Some(10));
    assert_eq!(listed[0].running_since(), None);
    assert_eq!(checks.passed_names(), vec!["Pass"]);
    assert_eq!(checks.skipped_names(), vec!["Skipping"]);
    assert_eq!(checks.count(CheckBucket::Fail), 1);
}
#[test]
fn test_check表示_時刻とjobが無いときは表示しない() {
    // Given
    let check = Check {
        name: "external".into(),
        bucket: CheckBucket::Fail,
        started_at: None,
        completed_at: None,
        job: None,
    };
    // When / Then
    assert_eq!(check.failure_seconds(), None);
    assert_eq!(check.log_job(), None);
    assert_eq!(check.running_since(), None);
}
