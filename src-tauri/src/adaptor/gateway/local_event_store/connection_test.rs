use super::*;
use crate::domain::operation_context::{Deadline, OperationContext};
use std::time::Instant;

#[test]
fn test_busy待ち_呼出期限より早い資源側の2秒で終了する() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("busy.db");
    let blocker = Connection::open(&path).unwrap();
    blocker
        .execute_batch("CREATE TABLE value(n); INSERT INTO value VALUES(1);")
        .unwrap();
    let reader = open_reader(&path).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let context = OperationContext::default()
        .with_deadline(Deadline::new(Instant::now() + Duration::from_secs(10)));
    let started = Instant::now();
    let error = crate::other::operation_context::sync_scope(context.clone(), || {
        reader.query_row("SELECT n FROM value", [], |row| row.get::<_, i64>(0))
    })
    .unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(context.check(Instant::now()).is_ok());
    blocker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT n FROM value", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
