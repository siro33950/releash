use crate::domain::local_event::LocalEventQueryError;
use connectrpc::ErrorCode;
use rusqlite::{ffi, Connection};

#[derive(Clone)]
pub(crate) enum ReadFailure {
    Query(LocalEventQueryError),
    Sqlite(i32),
}

impl ReadFailure {
    pub(crate) fn cases() -> [(Self, ErrorCode); 7] {
        [
            (
                Self::Query(LocalEventQueryError::QueryBusy),
                ErrorCode::Unavailable,
            ),
            (
                Self::Query(LocalEventQueryError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                        message: "deadline exceeded".into(),
                    },
                )),
                ErrorCode::DeadlineExceeded,
            ),
            (Self::Sqlite(ffi::SQLITE_BUSY), ErrorCode::Unavailable),
            (Self::Sqlite(ffi::SQLITE_LOCKED), ErrorCode::Unavailable),
            (
                Self::Sqlite(ffi::SQLITE_IOERR),
                ErrorCode::FailedPrecondition,
            ),
            (Self::Sqlite(ffi::SQLITE_CORRUPT), ErrorCode::DataLoss),
            (Self::Sqlite(ffi::SQLITE_NOTADB), ErrorCode::DataLoss),
        ]
    }

    pub(super) fn run<T>(
        self,
        connection: &Connection,
        run: impl FnOnce(&Connection) -> Result<T, LocalEventQueryError>,
    ) -> Result<T, LocalEventQueryError> {
        let code = match self {
            Self::Query(error) => return Err(error),
            Self::Sqlite(code) => code,
        };
        unsafe extern "C" fn fail(
            context: *mut ffi::sqlite3_context,
            _: i32,
            _: *mut *mut ffi::sqlite3_value,
        ) {
            // SAFETY: SQLite supplies the callback context and the integer-valued user data.
            unsafe {
                let code = ffi::sqlite3_user_data(context) as isize as i32;
                ffi::sqlite3_result_error_code(context, code);
            }
        }
        // SAFETY: the connection stays on its reader thread; user data is an integer, not dereferenced.
        let result = unsafe {
            ffi::sqlite3_create_function_v2(
                connection.handle(),
                c"injected_read_failure".as_ptr(),
                0,
                ffi::SQLITE_UTF8,
                code as isize as *mut std::ffi::c_void,
                Some(fail),
                None,
                None,
                None,
            )
        };
        assert_eq!(result, ffi::SQLITE_OK);
        connection
            .execute_batch(
                "CREATE TEMP VIEW node_events AS
             SELECT events.* FROM
               (SELECT injected_read_failure() AS failure LIMIT -1 OFFSET 0) AS fault
             CROSS JOIN main.node_events AS events WHERE fault.failure;",
            )
            .unwrap();
        let result = run(connection);
        connection
            .execute_batch("DROP VIEW temp.node_events")
            .unwrap();
        result
    }
}
