use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub enum PaneLayout {
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
}
#[derive(Clone, Debug, PartialEq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

impl PaneLayout {
    pub fn validate(&self) -> Result<(), crate::domain::workspace_state::WorkspaceStateError> {
        fn visit(layout: &PaneLayout, ids: &mut HashSet<String>, depth: usize) -> bool {
            if depth > 32 {
                return false;
            }
            let id = match layout {
                PaneLayout::Pane { id, .. } | PaneLayout::Split { id, .. } => id,
            };
            if id.trim().is_empty() || !ids.insert(id.clone()) {
                return false;
            }
            match layout {
                PaneLayout::Pane {
                    tabs, active_tab, ..
                } => {
                    tabs.iter()
                        .all(|tab| !tab.id.trim().is_empty() && ids.insert(tab.id.clone()))
                        && active_tab
                            .as_ref()
                            .is_none_or(|active| tabs.iter().any(|tab| &tab.id == active))
                }
                PaneLayout::Split {
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
        if visit(self, &mut HashSet::new(), 0) {
            Ok(())
        } else {
            Err(
                crate::domain::workspace_state::WorkspaceStateError::Message(
                    "Invalid pane layout".into(),
                ),
            )
        }
    }
}

#[cfg(test)]
#[path = "pane_layout_test.rs"]
mod pane_layout_tests;
