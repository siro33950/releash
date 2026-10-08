//! Indexed columns derived from projection rows at commit time.
//!
//! 統一 Node 事実ログ移行後、session_projection に残るのは provider 系の
//! record のみで、公開一覧用のインデックス列は使われない（常に NULL）。

use crate::domain::local_event::SessionProjectionRecord;

pub(crate) struct IndexedSessionPublicColumns {
    pub workspace_identity: Option<String>,
    pub list_kind: Option<&'static str>,
    pub sort_key_bits: Option<i64>,
    pub summary: Option<String>,
}

pub(crate) fn indexed_session_public_columns(
    projection: &SessionProjectionRecord,
) -> Result<IndexedSessionPublicColumns, String> {
    match projection {
        SessionProjectionRecord::ProviderSessionOwnership(_)
        | SessionProjectionRecord::ProviderHookHealth(_) => Ok(IndexedSessionPublicColumns {
            workspace_identity: None,
            list_kind: None,
            sort_key_bits: None,
            summary: None,
        }),
    }
}
