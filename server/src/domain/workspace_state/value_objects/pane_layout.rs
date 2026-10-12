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
}
#[derive(Clone, Debug, PartialEq)]
pub enum PaneTabKind {
    Terminal,
    Workflow,
    File,
    Session,
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
                    tabs.iter()
                        .all(|tab| !tab.id.trim().is_empty() && ids.insert(tab.id.clone()))
                        && active_tab
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
