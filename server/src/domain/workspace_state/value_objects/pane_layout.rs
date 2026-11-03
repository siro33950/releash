use crate::domain::code::value_objects::review::{FileMode, ReviewBase};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct PaneLayout(PaneLayoutNode);

#[derive(Clone, Debug, PartialEq)]
enum PaneLayoutNode {
    Pane {
        id: String,
        tabs: Vec<PaneTab>,
        active_tab: Option<String>,
    },
    Split {
        id: String,
        axis: SplitAxis,
        ratio: f64,
        first: Box<PaneLayout>,
        second: Box<PaneLayout>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneTab {
    pub id: String,
    pub kind: PaneTabKind,
    pub node_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub file_path: Option<String>,
    pub file_mode: Option<FileMode>,
    pub file_base: Option<ReviewBase>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PaneTabKind {
    Terminal,
    File,
    Session,
    Command,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

pub enum PaneLayoutView<'a> {
    Pane {
        id: &'a str,
        tabs: &'a [PaneTab],
        active_tab: Option<&'a str>,
    },
    Split {
        id: &'a str,
        axis: &'a SplitAxis,
        ratio: f64,
        first: &'a PaneLayout,
        second: &'a PaneLayout,
    },
}

impl PaneLayout {
    pub(crate) fn add_session_tab(&mut self, session_id: &str, node_id: &str) {
        if self.contains_session(session_id) || self.bind_pending_session(session_id, node_id) {
            return;
        }
        let mut id = format!("session-{session_id}");
        while self.contains_id(&id) {
            id.push('-');
        }
        self.append_tab(PaneTab {
            id,
            kind: PaneTabKind::Session,
            node_id: None,
            agent_session_id: Some(session_id.into()),
            file_path: None,
            file_mode: None,
            file_base: None,
        });
    }

    fn append_tab(&mut self, tab: PaneTab) {
        match &mut self.0 {
            PaneLayoutNode::Pane { tabs, .. } => tabs.push(tab),
            PaneLayoutNode::Split { first, .. } => first.append_tab(tab),
        }
    }

    fn contains_session(&self, session_id: &str) -> bool {
        match &self.0 {
            PaneLayoutNode::Pane { tabs, .. } => tabs
                .iter()
                .any(|tab| tab.agent_session_id.as_deref() == Some(session_id)),
            PaneLayoutNode::Split { first, second, .. } => {
                first.contains_session(session_id) || second.contains_session(session_id)
            }
        }
    }

    fn contains_id(&self, candidate: &str) -> bool {
        match &self.0 {
            PaneLayoutNode::Pane { id, tabs, .. } => {
                id == candidate || tabs.iter().any(|tab| tab.id == candidate)
            }
            PaneLayoutNode::Split {
                id, first, second, ..
            } => id == candidate || first.contains_id(candidate) || second.contains_id(candidate),
        }
    }

    fn bind_pending_session(&mut self, session_id: &str, node_id: &str) -> bool {
        match &mut self.0 {
            PaneLayoutNode::Pane { tabs, .. } => {
                if let Some(tab) = tabs.iter_mut().find(|tab| {
                    tab.kind == PaneTabKind::Session
                        && tab.node_id.as_deref() == Some(node_id)
                        && tab.agent_session_id.is_none()
                }) {
                    tab.agent_session_id = Some(session_id.into());
                    tab.node_id = None;
                    return true;
                }
                false
            }
            PaneLayoutNode::Split { first, second, .. } => {
                first.bind_pending_session(session_id, node_id)
                    || second.bind_pending_session(session_id, node_id)
            }
        }
    }

    pub(crate) fn remove_session_tab(&mut self, session_id: &str) {
        match &mut self.0 {
            PaneLayoutNode::Pane {
                tabs, active_tab, ..
            } => {
                let removed = |tab: &PaneTab| tab.agent_session_id.as_deref() == Some(session_id);
                if let Some(index) = tabs
                    .iter()
                    .position(|tab| active_tab.as_ref() == Some(&tab.id) && removed(tab))
                {
                    *active_tab = tabs[index + 1..]
                        .iter()
                        .find(|tab| !removed(tab))
                        .or_else(|| tabs[..index].iter().rev().find(|tab| !removed(tab)))
                        .map(|tab| tab.id.clone());
                }
                tabs.retain(|tab| !removed(tab));
            }
            PaneLayoutNode::Split { first, second, .. } => {
                first.remove_session_tab(session_id);
                second.remove_session_tab(session_id);
            }
        }
    }

    pub fn pane(
        id: String,
        tabs: Vec<PaneTab>,
        active_tab: Option<String>,
    ) -> Result<Self, crate::domain::workspace_state::WorkspaceStateError> {
        Self::checked(PaneLayoutNode::Pane {
            id,
            tabs,
            active_tab,
        })
    }
    pub fn split(
        id: String,
        axis: SplitAxis,
        ratio: f64,
        first: Self,
        second: Self,
    ) -> Result<Self, crate::domain::workspace_state::WorkspaceStateError> {
        Self::checked(PaneLayoutNode::Split {
            id,
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        })
    }
    pub fn view(&self) -> PaneLayoutView<'_> {
        match &self.0 {
            PaneLayoutNode::Pane {
                id,
                tabs,
                active_tab,
            } => PaneLayoutView::Pane {
                id,
                tabs,
                active_tab: active_tab.as_deref(),
            },
            PaneLayoutNode::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => PaneLayoutView::Split {
                id,
                axis,
                ratio: *ratio,
                first,
                second,
            },
        }
    }
    pub fn legacy_session_nodes(&self) -> Vec<String> {
        match &self.0 {
            PaneLayoutNode::Pane { tabs, .. } => tabs
                .iter()
                .filter(|tab| tab.kind == PaneTabKind::Session && tab.agent_session_id.is_none())
                .filter_map(|tab| tab.node_id.clone())
                .collect(),
            PaneLayoutNode::Split { first, second, .. } => {
                let mut ids = first.legacy_session_nodes();
                ids.extend(second.legacy_session_nodes());
                ids
            }
        }
    }

    pub fn resolve_legacy_sessions(
        &mut self,
        sessions: &std::collections::HashMap<String, String>,
    ) {
        match &mut self.0 {
            PaneLayoutNode::Pane { tabs, .. } => {
                for tab in tabs.iter_mut().filter(|tab| {
                    tab.kind == PaneTabKind::Session && tab.agent_session_id.is_none()
                }) {
                    if let Some(id) = tab.node_id.as_ref().and_then(|id| sessions.get(id)) {
                        tab.agent_session_id = Some(id.clone());
                        tab.node_id = None;
                    }
                }
            }
            PaneLayoutNode::Split { first, second, .. } => {
                first.resolve_legacy_sessions(sessions);
                second.resolve_legacy_sessions(sessions);
            }
        }
    }

    fn checked(
        node: PaneLayoutNode,
    ) -> Result<Self, crate::domain::workspace_state::WorkspaceStateError> {
        let layout = Self(node);
        fn visit(layout: &PaneLayout, ids: &mut HashSet<String>, depth: usize) -> bool {
            if depth > 32 {
                return false;
            }
            let id = match &layout.0 {
                PaneLayoutNode::Pane { id, .. } | PaneLayoutNode::Split { id, .. } => id,
            };
            if id.trim().is_empty() || !ids.insert(id.clone()) {
                return false;
            }
            match &layout.0 {
                PaneLayoutNode::Pane {
                    tabs, active_tab, ..
                } => {
                    tabs.iter().all(|tab| {
                        !tab.id.trim().is_empty()
                            && ids.insert(tab.id.clone())
                            && tab
                                .node_id
                                .as_deref()
                                .is_none_or(|id| !id.trim().is_empty() && !id.contains('\0'))
                            && (matches!(tab.kind, PaneTabKind::Session | PaneTabKind::Command)
                                || tab.node_id.is_none())
                            && tab
                                .agent_session_id
                                .as_deref()
                                .is_none_or(|id| !id.trim().is_empty() && !id.contains('\0'))
                            && (tab.agent_session_id.is_none()
                                || (tab.kind == PaneTabKind::Session && tab.node_id.is_none()))
                            && tab
                                .file_path
                                .as_deref()
                                .is_none_or(|path| !path.trim().is_empty() && !path.contains('\0'))
                            && (tab.kind == PaneTabKind::File
                                || (tab.file_path.is_none()
                                    && tab.file_mode.is_none()
                                    && tab.file_base.is_none()))
                    }) && active_tab
                        .as_ref()
                        .is_none_or(|active| tabs.iter().any(|tab| &tab.id == active))
                }
                PaneLayoutNode::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    ratio.is_finite()
                        && *ratio > 0.0
                        && *ratio < 1.0
                        && visit(first, ids, depth + 1)
                        && visit(second, ids, depth + 1)
                }
            }
        }
        if visit(&layout, &mut HashSet::new(), 0) {
            Ok(layout)
        } else {
            Err(crate::domain::workspace_state::WorkspaceStateError::InvalidLayout)
        }
    }
}

#[cfg(test)]
#[path = "pane_layout_test.rs"]
mod pane_layout_tests;
