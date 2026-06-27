pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod code;
pub(crate) mod comment;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod hooks;
pub(crate) mod notification;
pub(crate) mod notion;
pub(crate) mod pty_session;
pub(crate) mod repository;
pub(crate) mod telemetry;
pub(crate) mod workflow;
pub(crate) mod workspace_state;
pub(crate) mod workspace_tree;

type InvokeHandler = Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync>;

pub(crate) struct CommandRouter {
    fallback: InvokeHandler,
    domains: Vec<CommandDomainRoute>,
}

struct CommandDomainRoute {
    command_names: &'static [&'static str],
    handler: InvokeHandler,
}

impl CommandRouter {
    fn new(fallback: InvokeHandler) -> Self {
        Self {
            fallback,
            domains: Vec::new(),
        }
    }

    pub(crate) fn register_domain(
        &mut self,
        command_names: &'static [&'static str],
        handler: InvokeHandler,
    ) {
        self.domains.push(CommandDomainRoute {
            command_names,
            handler,
        });
    }

    fn handle(&self, invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
        let route_index = self.domain_route_index(invoke.message.command());

        if let Some(route_index) = route_index {
            (self.domains[route_index].handler)(invoke)
        } else {
            (self.fallback)(invoke)
        }
    }

    fn domain_route_index(&self, command: &str) -> Option<usize> {
        self.domains
            .iter()
            .position(|domain| domain.command_names.contains(&command))
    }
}

pub(crate) fn register_all(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let app_handler: InvokeHandler = Box::new(tauri::generate_handler![
            // ファイル監視
            crate::watcher::start_watching,
            crate::watcher::start_git_dir_watching,
            crate::watcher::stop_watching,
            // Git: diff/content（code ドメイン）
            crate::adaptor::controller::command::code::file_content::get_file_at_ref,
            crate::adaptor::controller::command::code::file_content::get_staged_content,
            crate::adaptor::controller::command::code::file_content::get_binary_staged_content,
            crate::adaptor::controller::command::code::file_content::get_file_at_branch_base,
            crate::adaptor::controller::command::code::file_content::get_binary_file_at_branch_base,
            crate::adaptor::controller::command::code::file_content::get_binary_file_at_ref,
            crate::adaptor::controller::command::code::file_content::get_review_text_diff,
            crate::adaptor::controller::command::code::file_content::get_review_image_diff,
            crate::adaptor::controller::command::code::review::get_review_snapshot,
            crate::adaptor::controller::command::code::review::get_review_file_view,
            crate::adaptor::controller::command::code::review::git_stage_review_group,
            crate::adaptor::controller::command::code::review::git_unstage_review_group,
            crate::adaptor::controller::command::code::diff::get_branch_diff_summary,
            crate::adaptor::controller::command::code::diff::build_diff_file_tree,
            crate::adaptor::controller::command::code::diff::get_head_diff_file_tree_snapshot,
            crate::adaptor::controller::command::code::diff::get_file_navigation,
            // Git: hunk/patch（code ドメイン）
            crate::adaptor::controller::command::code::hunk::compute_hidden_ranges,
            crate::adaptor::controller::command::code::hunk::compute_hidden_ranges_from_content,
            crate::adaptor::controller::command::code::hunk::compute_visible_markdown_blocks,
            crate::adaptor::controller::command::code::language::get_language_from_path,
            crate::adaptor::controller::command::code::diff::get_relative_path,
            // Git: ブランチ（repository ドメイン）
            crate::adaptor::controller::command::repository::branch::list_branches,
            crate::adaptor::controller::command::repository::branch::get_current_branch,
            crate::adaptor::controller::command::repository::branch::get_default_branch,
            crate::adaptor::controller::command::repository::branch::git_create_branch,
            crate::adaptor::controller::command::repository::branch::delete_branch,
            // Git: ステータス（repository ドメイン）
            crate::adaptor::controller::command::repository::status::get_git_status,
            crate::adaptor::controller::command::repository::status::get_git_status_snapshot,
            crate::adaptor::controller::command::repository::status::get_status_diff_stats,
            crate::adaptor::controller::command::repository::status::get_status_diff_stats_snapshot,
            crate::adaptor::controller::command::repository::log::get_git_log,
            // Git: ステージング（code ドメイン）
            crate::adaptor::controller::command::code::staging::git_stage,
            crate::adaptor::controller::command::code::staging::git_unstage,
            // Git: ワークツリー（repository ドメイン）
            crate::adaptor::controller::command::repository::worktree::get_main_repo_path,
            crate::adaptor::controller::command::repository::worktree::get_worktree_dirty_count,
            crate::adaptor::controller::command::repository::worktree::list_worktrees,
            crate::adaptor::controller::command::repository::worktree::list_branches_with_status,
            crate::adaptor::controller::command::repository::worktree::list_branches_with_status_snapshot,
            crate::adaptor::controller::command::repository::worktree::create_worktree,
            crate::adaptor::controller::command::repository::worktree::remove_worktree,
            // Git: 設定・ユーティリティ（repository ドメイン）
            crate::adaptor::controller::command::repository::util::get_cwd,
            crate::adaptor::controller::command::repository::util::get_repo_git_dir,
            crate::adaptor::controller::command::repository::git_config::get_releash_base,
            crate::adaptor::controller::command::repository::git_config::set_releash_base,
            crate::adaptor::controller::command::repository::git_config::get_branch_base,
            crate::adaptor::controller::command::repository::git_config::set_branch_base,
            // アプリ設定
            // Agent Status (Rust 中央管理)
            crate::adaptor::controller::command::agent_session::status::get_session_status,
            crate::adaptor::controller::command::agent_session::status::get_workspace_status,
            crate::adaptor::controller::command::agent_session::status::list_workspace_statuses,
            crate::adaptor::controller::command::agent_session::status::list_session_statuses,
            crate::adaptor::controller::command::agent_session::status::list_workflow_step_statuses,
            // Repo paths（repository ドメイン）
            crate::adaptor::controller::command::repository::repo_paths::get_repo_paths,
            crate::adaptor::controller::command::repository::repo_paths::add_repo_path,
            crate::adaptor::controller::command::repository::repo_paths::remove_repo_path,
            // File Mention（code ドメイン）
            crate::adaptor::controller::command::code::mention::list_mentionable_files,
            crate::adaptor::controller::command::code::mention::read_codex_mentionable_files,
            crate::adaptor::controller::command::code::mention::sync_mentions_with_text,
            // Agent Backend Registry
            crate::adaptor::controller::command::agent_session::backend::list_agent_backends,
            // Agent SDK
            crate::adaptor::controller::command::agent_session::session::start_agent_session,
            crate::adaptor::controller::command::agent_session::session::interrupt_agent_query,
            crate::adaptor::controller::command::agent_session::session::cancel_agent_queued_turn,
            crate::adaptor::controller::command::agent_session::session::build_agent_task_list_report,
            crate::adaptor::controller::command::agent_session::session::close_agent_session,
            crate::adaptor::controller::command::agent_session::model::set_agent_permission_mode,
            crate::adaptor::controller::command::agent_session::model::set_agent_plan_mode,
            crate::adaptor::controller::command::agent_session::model::set_agent_model,
            crate::adaptor::controller::command::agent_session::session::set_session_backend,
            crate::adaptor::controller::command::agent_session::command_palette::present_agent_command_palette,
            crate::adaptor::controller::command::agent_session::command_palette::is_agent_command_enabled,
            crate::adaptor::controller::command::agent_session::command_palette::get_agent_shortcut_settings,
            crate::adaptor::controller::command::agent_session::command_palette::update_agent_shortcut_settings,
            crate::adaptor::controller::command::agent_session::command_palette::reset_agent_shortcut_settings,
            crate::adaptor::controller::command::agent_session::permission::present_agent_permission_request,
            crate::adaptor::controller::command::agent_session::permission::respond_agent_permission,
            crate::adaptor::controller::command::agent_session::session::send_agent_message,
            crate::adaptor::controller::command::agent_session::session::search_agent_sessions,
            crate::adaptor::controller::command::agent_session::session::search_agent_session_messages,
            crate::adaptor::controller::command::agent_session::session::init_agent_sessions,
            crate::adaptor::controller::command::agent_session::action::scan_agent_skills,
            crate::adaptor::controller::command::agent_session::action::read_codex_skill_catalog,
            crate::adaptor::controller::command::agent_session::edit_preview::build_agent_edited_multi_edit_tool_input,
            crate::adaptor::controller::command::agent_session::edit_preview::build_agent_edited_multi_edit_tool_input_all,
            crate::adaptor::controller::command::agent_session::edit_preview::build_agent_edited_tool_input,
            crate::adaptor::controller::command::agent_session::edit_preview::build_agent_edit_preview,
            crate::adaptor::controller::command::agent_session::suggestion::build_agent_prompt_suggestion,
            crate::adaptor::controller::command::agent_session::tool_activity::present_agent_tool_activity,
            crate::adaptor::controller::command::agent_session::image::prepare_image_attachment,
            crate::adaptor::controller::command::agent_session::image::prepare_image_attachments_from_paths,
            crate::adaptor::controller::command::agent_session::paste::prepare_pasted_text_block,
            crate::adaptor::controller::command::agent_session::paste::expand_pasted_text_blocks,
            // Session
            crate::adaptor::controller::command::agent_session::stored_session::list_sessions,
            crate::adaptor::controller::command::agent_session::session::get_session,
            crate::adaptor::controller::command::agent_session::session::get_session_page,
            crate::adaptor::controller::command::agent_session::session::resync_streaming_message,
            crate::adaptor::controller::command::agent_session::session::plan_agent_chat_eviction,
            crate::adaptor::controller::command::agent_session::session::get_session_attachment,
            crate::adaptor::controller::command::agent_session::session::get_session_tool_output,
            crate::adaptor::controller::command::agent_session::stored_session::create_session,
            crate::adaptor::controller::command::agent_session::stored_session::close_session,
            crate::adaptor::controller::command::agent_session::stored_session::restore_session,
            crate::adaptor::controller::command::agent_session::stored_session::list_closed_sessions,
            crate::adaptor::controller::command::agent_session::stored_session::archive_session,
            crate::adaptor::controller::command::agent_session::stored_session::archive_open_session,
            crate::adaptor::controller::command::agent_session::stored_session::fork_session,
            crate::adaptor::controller::command::agent_session::stored_session::set_session_title,
            crate::adaptor::controller::command::agent_session::stored_session::add_message,
            crate::adaptor::controller::command::agent_session::stored_session::update_session_state,
            crate::adaptor::controller::command::agent_session::stored_session::update_session_agent_info,
            // Workspace tree
            crate::adaptor::controller::command::workspace_tree::list_workspace_worktree_nodes,
            crate::adaptor::controller::command::workspace_tree::list_workspace_workflow_history,
            crate::adaptor::controller::command::workspace_tree::get_workspace_workflow_step_detail,
            crate::adaptor::controller::command::workspace_tree::archive_workspace_workflow_run,
            crate::adaptor::controller::command::workspace_tree::restore_workspace_workflow_run,
            // Menu
            crate::menu::set_menu_items_enabled,

        ]);
    let mut router = CommandRouter::new(app_handler);
    app_config::register(&mut router);
    comment::register(&mut router);
    external_editor::register(&mut router);
    git_host::register(&mut router);
    hooks::register(&mut router);
    notification::register(&mut router);
    notion::register(&mut router);
    pty_session::register(&mut router);
    telemetry::register(&mut router);
    workspace_state::register(&mut router);
    workflow::register(&mut router);
    builder.invoke_handler(move |invoke: tauri::ipc::Invoke<tauri::Wry>| router.handle(invoke))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_handler() -> InvokeHandler {
        Box::new(|_invoke| true)
    }

    #[test]
    fn workflow_register_routes_workflow_commands_before_fallback() {
        let mut router = CommandRouter::new(dummy_handler());

        workflow::register(&mut router);

        assert_eq!(router.domain_route_index("start_workflow"), Some(0));
        assert_eq!(router.domain_route_index("workflow_submit_output"), Some(0));
        assert_eq!(router.domain_route_index("get_git_status"), None);
    }

    #[test]
    fn git_host_register_routes_git_host_commands_before_fallback() {
        let mut router = CommandRouter::new(dummy_handler());

        git_host::register(&mut router);

        assert_eq!(
            router.domain_route_index("check_pr_provider_status"),
            Some(0)
        );
        assert_eq!(router.domain_route_index("get_cached_issues"), Some(0));
        assert_eq!(router.domain_route_index("get_git_status"), None);
    }

    #[test]
    fn notion_register_routes_notion_commands_before_fallback() {
        let mut router = CommandRouter::new(dummy_handler());

        notion::register(&mut router);

        assert_eq!(router.domain_route_index("query_notion_tasks"), Some(0));
        assert_eq!(
            router.domain_route_index("fetch_notion_label_options"),
            Some(0)
        );
        assert_eq!(router.domain_route_index("save_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("get_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("delete_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("validate_notion_config"), Some(0));
        assert_eq!(router.domain_route_index("get_git_status"), None);
    }
}
