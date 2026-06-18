pub(crate) mod agent_session;
pub(crate) mod code;
pub(crate) mod repository;
pub(crate) mod workflow;

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

            // PTY
            crate::pty::spawn_pty,
            crate::pty::write_pty,
            crate::pty::resize_pty,
            crate::pty::kill_pty,
            crate::pty::list_pty_sessions,
            crate::pty::get_or_spawn_pty,
            crate::pty::kill_ptys_by_worktree,
            crate::pty::gc_ptys_for_worktree,
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
            crate::adaptor::controller::command::code::diff::get_branch_diff_summary,
            crate::adaptor::controller::command::code::diff::build_diff_file_tree,
            crate::adaptor::controller::command::code::diff::get_file_navigation,
            // Git: hunk/patch（code ドメイン）
            crate::adaptor::controller::command::code::hunk::compute_diff_hunks,
            crate::adaptor::controller::command::code::hunk::compute_hidden_ranges,
            crate::adaptor::controller::command::code::hunk::compute_hidden_ranges_from_content,
            crate::adaptor::controller::command::code::hunk::compute_visible_markdown_blocks,
            crate::adaptor::controller::command::code::hunk::generate_group_patch,
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
            crate::adaptor::controller::command::repository::status::get_status_diff_stats,
            crate::adaptor::controller::command::repository::log::get_git_log,
            // Git: ステージング（code ドメイン）
            crate::adaptor::controller::command::code::staging::git_stage,
            crate::adaptor::controller::command::code::staging::git_unstage,
            crate::adaptor::controller::command::code::staging::git_stage_hunk,
            crate::adaptor::controller::command::code::staging::git_unstage_hunk,
            // Git: ワークツリー（repository ドメイン）
            crate::adaptor::controller::command::repository::worktree::get_main_repo_path,
            crate::adaptor::controller::command::repository::worktree::get_worktree_dirty_count,
            crate::adaptor::controller::command::repository::worktree::list_worktrees,
            crate::adaptor::controller::command::repository::worktree::list_branches_with_status,
            crate::adaptor::controller::command::repository::worktree::create_worktree,
            crate::adaptor::controller::command::repository::worktree::remove_worktree,
            // Git: 設定・ユーティリティ（repository ドメイン）
            crate::adaptor::controller::command::repository::util::get_cwd,
            crate::adaptor::controller::command::repository::util::get_repo_git_dir,
            crate::adaptor::controller::command::repository::git_config::get_releash_base,
            crate::adaptor::controller::command::repository::git_config::set_releash_base,
            crate::adaptor::controller::command::repository::git_config::get_branch_base,
            crate::adaptor::controller::command::repository::git_config::set_branch_base,
            // Git Host
            crate::git_host::check_pr_provider_status,
            crate::git_host::fetch_pr_status,
            crate::git_host::get_cached_pr_status,
            crate::git_host::fetch_issues,
            crate::git_host::get_cached_issues,
            // Notion
            crate::notion::query_notion_tasks,
            crate::notion::fetch_notion_label_options,
            crate::notion::save_notion_config,
            crate::notion::get_notion_config,
            crate::notion::delete_notion_config,
            crate::notion::validate_notion_config,
            // アプリ設定
            crate::config::get_server_config,
            crate::config::update_server_port,
            crate::config::regenerate_token,
            crate::config::generate_hooks_config,
            crate::config::apply_hooks_config,
            crate::config::get_hooks_status,
            crate::config::update_telemetry_enabled,
            crate::config::get_notify_config,
            crate::config::update_notify_config,
            crate::config::get_remote_config,
            crate::config::update_remote_config,
            crate::config::get_workflow_config,
            crate::config::update_workflow_config,
            crate::config::get_app_settings,
            crate::config::update_app_settings,
            crate::config::update_last_server_context,
            crate::config::get_crash_reporting_enabled,
            crate::config::update_crash_reporting,
            crate::config::update_webhook_url,
            crate::config::get_external_editor,
            crate::config::update_external_editor,
            crate::config::get_mcp_config,
            crate::config::update_mcp_config,
            crate::config::regenerate_mcp_token,
            // External Editor
            crate::external_editor::detect_editors,
            crate::external_editor::open_in_editor,
            crate::external_editor::open_folder_in_editor,
            // Agent Status (Rust 中央管理)
            crate::adaptor::controller::command::agent_session::status::get_session_status,
            crate::adaptor::controller::command::agent_session::status::get_workspace_status,
            crate::adaptor::controller::command::agent_session::status::list_workspace_statuses,
            crate::adaptor::controller::command::agent_session::status::list_session_statuses,
            // ネットワーク
            crate::vpn_detect::detect_vpn_tunnel,
            crate::vpn_detect::get_network_info,
            crate::qr_code::get_connection_qr,
            // WebSocket サーバー
            crate::ws_server::commands::start_server,
            crate::ws_server::commands::stop_server,
            crate::ws_server::commands::get_server_status,
            crate::ws_server::commands::get_server_info,
            crate::ws_server::commands::update_terminal_startup_command,
            // Repo paths（repository ドメイン）
            crate::adaptor::controller::command::repository::repo_paths::get_repo_paths,
            crate::adaptor::controller::command::repository::repo_paths::add_repo_path,
            crate::adaptor::controller::command::repository::repo_paths::remove_repo_path,
            // MCP Server
            crate::mcp::start_mcp_server,
            crate::mcp::stop_mcp_server,
            crate::mcp::get_mcp_server_status,
            crate::mcp::get_mcp_connection_info,
            crate::mcp::mcp_json::get_configured_agents,
            crate::mcp::mcp_json::remove_agent_mcp_config,
            crate::mcp::mcp_json::save_and_generate_mcp_configs,
            crate::mcp::mcp_json::save_mcp_agent_selection,
            crate::mcp::mcp_json::generate_agent_mcp_config,
            crate::mcp::mcp_json::preview_agent_mcp_config,
            // Workspace state
            crate::workspace_state_store::load_workspace_state,
            crate::workspace_state_store::save_workspace_state,
            // Review comments
            crate::review_comments::list_review_threads,
            crate::review_comments::get_review_thread,
            crate::review_comments::create_review_thread,
            crate::review_comments::append_review_comment,
            crate::review_comments::resolve_review_thread,
            crate::review_comments::delete_review_thread,
            crate::review_comments::get_review_thread_history,
            crate::review_comments::build_review_thread_handoff,
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
            crate::adaptor::controller::command::agent_session::session::search_agent_thread_messages,
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
            // Menu
            crate::menu::set_menu_items_enabled,

        ]);
    let mut router = CommandRouter::new(app_handler);
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
}
