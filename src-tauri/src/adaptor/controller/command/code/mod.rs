pub(crate) mod review_blob;
pub(crate) const COMMAND_NAMES: &[&str] = &[
    "get_file_at_ref",
    "get_staged_content",
    "get_binary_staged_content",
    "get_file_at_branch_base",
    "get_binary_file_at_branch_base",
    "get_binary_file_at_ref",
    "get_review_snapshot",
    "get_review_file_view",
    "git_stage_review_group",
    "git_unstage_review_group",
    "get_branch_diff_summary",
    "build_diff_file_tree",
    "get_head_diff_file_tree_snapshot",
    "get_file_navigation",
    "compute_hidden_ranges",
    "compute_hidden_ranges_from_content",
    "compute_visible_markdown_blocks",
    "compute_markdown_diff_ranges",
    "compute_markdown_split_rows",
    "compute_markdown_inline_chunks",
    "get_language_from_path",
    "get_relative_path",
    "git_stage",
    "git_unstage",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
