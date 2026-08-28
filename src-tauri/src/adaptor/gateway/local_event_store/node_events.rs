//! Row-level access to the unified-node fact log (`node_events`).
//!
//! State is derived by the domain fold on the read side, so this module knows
//! nothing about the fact vocabulary beyond the column shapes.

use rusqlite::Connection;

#[cfg(test)]
#[path = "node_events_test.rs"]
mod node_events_test;

/// A fact about to be appended. `seq` and `timestamp` are assigned by the
/// store at append time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewNodeEventRow {
    pub tree_id: String,
    pub node_execution_id: String,
    pub parent_id: Option<String>,
    pub node_name: String,
    pub kind: String,
    pub attempt: i64,
    pub event_type: String,
    pub session_id: Option<String>,
    pub detail: String,
}

/// A stored fact row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeEventRow {
    pub tree_id: String,
    pub seq: i64,
    pub node_execution_id: String,
    pub parent_id: Option<String>,
    pub node_name: String,
    pub kind: String,
    pub attempt: i64,
    pub event_type: String,
    pub session_id: Option<String>,
    pub detail: String,
    pub timestamp_ms: i64,
}

/// Append one fact row. `seq` is `MAX(seq) + 1` within the tree, computed
/// inside the single INSERT statement so the append is atomic on its own.
pub(crate) fn append_node_event(
    connection: &Connection,
    row: &NewNodeEventRow,
    timestamp_ms: i64,
) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "INSERT INTO node_events (
             tree_id, seq, node_execution_id, parent_id, node_name,
             kind, attempt, event_type, session_id, detail, timestamp
         )
         SELECT ?1, COALESCE(MAX(seq), 0) + 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
         FROM node_events WHERE tree_id = ?1
         RETURNING seq",
        rusqlite::params![
            row.tree_id,
            row.node_execution_id,
            row.parent_id,
            row.node_name,
            row.kind,
            row.attempt,
            row.event_type,
            row.session_id,
            row.detail,
            timestamp_ms,
        ],
        |row| row.get(0),
    )
}

/// Physically delete every row of one tree. This is the delete operation's
/// meaning (removing the data), not a fact append.
pub(crate) fn delete_tree(
    connection: &Connection,
    tree_id: &str,
) -> Result<usize, rusqlite::Error> {
    connection.execute("DELETE FROM node_events WHERE tree_id = ?1", [tree_id])
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> Result<NodeEventRow, rusqlite::Error> {
    Ok(NodeEventRow {
        tree_id: row.get(0)?,
        seq: row.get(1)?,
        node_execution_id: row.get(2)?,
        parent_id: row.get(3)?,
        node_name: row.get(4)?,
        kind: row.get(5)?,
        attempt: row.get(6)?,
        event_type: row.get(7)?,
        session_id: row.get(8)?,
        detail: row.get(9)?,
        timestamp_ms: row.get(10)?,
    })
}

const ROW_COLUMNS: &str = "tree_id, seq, node_execution_id, parent_id, node_name, \
     kind, attempt, event_type, session_id, detail, timestamp";

/// Read one tree's facts in append order.
pub(crate) fn read_tree(
    connection: &Connection,
    tree_id: &str,
) -> Result<Vec<NodeEventRow>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events WHERE tree_id = ?1 ORDER BY seq"
    ))?;
    let rows = statement.query_map([tree_id], row_from_sql)?;
    rows.collect()
}

pub(crate) fn read_tree_page(
    connection: &Connection,
    tree_id: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<NodeEventRow>, rusqlite::Error> {
    let offset = i64::try_from(offset).unwrap_or(i64::MAX);
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events WHERE tree_id = ?1 ORDER BY seq LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = statement.query_map(rusqlite::params![tree_id, limit, offset], row_from_sql)?;
    rows.collect()
}

/// Latest fact row of one node execution (used to resolve a node's identity
/// columns for facts that only carry the node execution id).
pub(crate) fn latest_row_for_node(
    connection: &Connection,
    node_execution_id: &str,
) -> Result<Option<NodeEventRow>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events
         WHERE node_execution_id = ?1 ORDER BY seq DESC LIMIT 1"
    ))?;
    let mut rows = statement.query_map([node_execution_id], row_from_sql)?;
    rows.next().transpose()
}

pub(crate) fn latest_row_for_node_with_event_types(
    connection: &Connection,
    node_execution_id: &str,
    event_types: &[&str],
) -> Result<Option<NodeEventRow>, rusqlite::Error> {
    if event_types.is_empty() {
        return Ok(None);
    }
    let placeholders = (2..event_types.len() + 2)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events
         WHERE node_execution_id = ?1 AND event_type IN ({placeholders})
         ORDER BY seq DESC LIMIT 1",
    ))?;
    let parameters = std::iter::once(node_execution_id).chain(event_types.iter().copied());
    let mut rows = statement.query_map(rusqlite::params_from_iter(parameters), row_from_sql)?;
    rows.next().transpose()
}

/// First row of one tree (the root started fact).
pub(crate) fn first_row_of_tree(
    connection: &Connection,
    tree_id: &str,
) -> Result<Option<NodeEventRow>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events
         WHERE tree_id = ?1 ORDER BY seq ASC LIMIT 1"
    ))?;
    let mut rows = statement.query_map([tree_id], row_from_sql)?;
    rows.next().transpose()
}

/// List every root fact of the given event type (`parent_id IS NULL`),
/// oldest tree first. The event type is a caller-provided narrowing value so
/// the fact vocabulary stays owned by the domain.
pub(crate) fn list_tree_roots(
    connection: &Connection,
    root_event_type: &str,
) -> Result<Vec<NodeEventRow>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events
         WHERE parent_id IS NULL AND event_type = ?1
         ORDER BY timestamp, tree_id, seq"
    ))?;
    let rows = statement.query_map([root_event_type], row_from_sql)?;
    rows.collect()
}

pub(crate) fn latest_session_attachment(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<NodeEventRow>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ROW_COLUMNS} FROM node_events
         WHERE session_id = ?1 ORDER BY tree_id DESC, seq DESC LIMIT 1"
    ))?;
    let mut rows = statement.query_map([session_id], row_from_sql)?;
    rows.next().transpose()
}
