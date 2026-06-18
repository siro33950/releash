#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLayoutState {
    pub center_tab: String,
    pub active_view: String,
    pub left_nav_collapsed: bool,
    pub right_collapsed: bool,
    pub right_bottom_collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_bottom_active_tab: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_diff_file: Option<String>,
}
