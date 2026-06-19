#[derive(Clone, Debug)]
pub struct WorkspaceLayoutState {
    pub center_tab: String,
    pub active_view: String,
    pub left_nav_collapsed: bool,
    pub right_collapsed: bool,
    pub right_bottom_collapsed: bool,
    pub right_bottom_active_tab: Option<String>,
    pub selected_diff_file: Option<String>,
}
