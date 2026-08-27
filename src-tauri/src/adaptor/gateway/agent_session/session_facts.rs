use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::workflow::{NodeFactMeta, NodeKindName};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionLocation {
    pub(crate) tree_id: String,
    pub(crate) node_execution_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) node_name: String,
    pub(crate) attempt: u32,
}

impl SessionLocation {
    pub(crate) fn from_meta(meta: &NodeFactMeta) -> Self {
        Self {
            tree_id: meta.tree_id.clone(),
            node_execution_id: meta.node_execution_id.clone(),
            parent_id: meta.parent_id.clone(),
            node_name: meta.node_name.clone(),
            attempt: meta.attempt,
        }
    }

    pub(crate) fn meta(&self) -> NodeFactMeta {
        NodeFactMeta {
            tree_id: self.tree_id.clone(),
            node_execution_id: self.node_execution_id.clone(),
            parent_id: self.parent_id.clone(),
            node_name: self.node_name.clone(),
            kind: NodeKindName::Session,
            attempt: self.attempt,
        }
    }
}

pub(crate) fn locate_session(
    backend: &FactLogReadBackend,
    session_id: &str,
) -> Result<Option<SessionLocation>, String> {
    let Some((tree_id, node_execution_id)) =
        fact_log::find_session_attachment(backend, session_id)?
    else {
        return Ok(None);
    };
    let records = fact_log::read_tree_records_from(backend, &tree_id)?;
    let Some(row) = records
        .iter()
        .find(|record| record.meta.node_execution_id == node_execution_id)
    else {
        return Ok(None);
    };
    Ok(Some(SessionLocation::from_meta(&row.meta)))
}
