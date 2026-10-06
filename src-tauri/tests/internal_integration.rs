#[path = "support/agent_tui_fixture.rs"]
pub(crate) mod agent_tui_fixture;
pub(crate) mod acceptance_test_support {
    pub use releash_lib::test_support::integration::acceptance_test_support::*;
}
pub(crate) mod adaptor {
    pub(crate) mod controller {
        pub(crate) mod agent_session_launch_retention {
            pub use releash_lib::test_support::integration::adaptor::controller::agent_session_launch_retention::*;
            pub(crate) mod agent_session_launch_retention_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/controller/agent_session_launch_retention_test.rs"));
            }
        }
        pub(crate) mod agent_session_wiring {
            pub use releash_lib::test_support::integration::adaptor::controller::agent_session_wiring::*;
        }
        pub(crate) mod api {

            pub(crate) use axum::middleware;
            pub(crate) use axum::Router;

            pub use releash_lib::test_support::integration::adaptor::controller::api::*;
            pub(crate) mod auth {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::controller::api::auth::*;
                pub(crate) mod client_auth_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/controller/api/client_auth_test.rs"
                    ));
                }
            }
            pub(crate) mod client {
                pub(crate) use crate::adaptor::presenter::client as wire;
                pub(crate) use axum::Router;
                pub(crate) use std::sync::Arc;

                pub(crate) use releash_lib::desktop_api::rpc;
                pub use releash_lib::test_support::integration::adaptor::controller::api::client::*;
                pub(crate) mod client_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/controller/api/client_test.rs"
                    ));
                }
            }
            pub(crate) mod mod_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/api/mod_test.rs"
                ));
            }
            pub(crate) use mod_integration_tests::test_support;
            pub(crate) mod protocol {
                pub use releash_lib::test_support::integration::adaptor::controller::api::protocol::*;
            }
            pub(crate) mod provider_lifecycle {
                pub(crate) mod provider_lifecycle_controller_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/controller/api/provider_lifecycle_controller_test.rs"));
                }
            }
            pub(crate) mod workflow {
                pub(crate) mod workflow_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/controller/api/workflow_test.rs"
                    ));
                }
            }
        }
        pub(crate) mod app_data_composition {
            pub(crate) use std::sync::Arc;

            pub use releash_lib::test_support::integration::adaptor::controller::app_data_composition::*;
            pub(crate) mod app_data_composition_integration_tests_extra {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/app_data_composition_test_extra.rs"
                ));
            }
            pub(crate) mod app_data_composition_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/app_data_composition_test.rs"
                ));
            }
        }
        pub(crate) mod client {
            pub(crate) mod dependencies {}

            pub use releash_lib::test_support::integration::adaptor::controller::client::*;
            pub(crate) mod app_config {
                pub(crate) mod commands {
                    pub(crate) use std::sync::Arc;

                    pub use releash_lib::test_support::integration::adaptor::controller::client::app_config::commands::*;
                    pub(crate) mod commands_tests {
                        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/controller/client/app_config/commands_test.rs"));
                    }
                }
                pub(crate) mod shared {

                    pub(crate) use crate::adaptor::presenter::client as wire;

                    pub use releash_lib::test_support::integration::adaptor::controller::client::app_config::shared::*;
                    pub(crate) mod shared_tests {
                        include!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/tests/support/adaptor/controller/client/app_config/shared_test.rs"
                        ));
                    }
                }
            }
            pub(crate) mod code {
                pub use releash_lib::test_support::integration::adaptor::controller::client::code::*;
            }
            pub(crate) mod dispatch {
                pub(crate) use crate::adaptor::presenter::client as wire;

                pub use releash_lib::test_support::integration::adaptor::controller::client::dispatch::*;
                pub(crate) mod tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/adaptor/controller/client/dispatch_test.rs"
                    ));
                }
            }
            pub(crate) mod dispatch_parity_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/client/dispatch_parity_test.rs"
                ));
            }
            pub(crate) mod repository {
                pub use releash_lib::test_support::integration::adaptor::controller::client::repository::*;
            }
            pub(crate) mod workflow {
                pub(crate) use std::sync::Arc;

                pub(crate) use crate::adaptor::gateway::workflow::builtin;
                pub(crate) use crate::adaptor::gateway::workflow::storage;

                pub use releash_lib::test_support::integration::adaptor::controller::client::workflow::*;
                pub(crate) mod mod_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/controller/client/workflow/mod_test.rs"
                    ));
                }
                pub(crate) use mod_integration_tests::tests;
            }
            pub(crate) mod workspace_tree {
                pub use releash_lib::test_support::integration::adaptor::controller::client::workspace_tree::*;
                pub(crate) mod shared {
                    pub(crate) use super::*;

                    pub(crate) use crate::adaptor::presenter::client as wire;

                    pub use releash_lib::test_support::integration::adaptor::controller::client::workspace_tree::shared::*;
                    pub(crate) mod workspace_tree_shared_tests {
                        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/controller/client/workspace_tree_shared_test.rs"));
                    }
                }
            }
            pub(crate) mod worktree_mutation {
                pub(crate) use crate::adaptor::presenter::client as wire;

                pub use releash_lib::test_support::integration::adaptor::controller::client::worktree_mutation::*;
                pub(crate) mod worktree_mutation_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/controller/client/worktree_mutation_test.rs"));
                }
            }
        }
        pub(crate) mod daemon {

            pub(crate) use std::sync::Arc;

            pub(crate) use crate::adaptor;
            pub(crate) use crate::domain;
            pub(crate) use crate::infrastructure;

            pub(crate) use crate::usecase;

            pub use releash_lib::test_support::integration::adaptor::controller::daemon::*;
            pub(crate) mod daemon_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/daemon_test.rs"
                ));
            }
        }
        pub(crate) mod repository_scan {
            pub use releash_lib::test_support::integration::adaptor::controller::repository_scan::*;
        }
        pub(crate) mod terminal_surface_runtime {
            pub(crate) use std::sync::Arc;

            pub use releash_lib::test_support::integration::adaptor::controller::terminal_surface_runtime::*;
            pub(crate) mod terminal_surface_runtime_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/terminal_surface_runtime_test.rs"
                ));
            }
        }
        pub(crate) mod wiring {
            pub(crate) use std::sync::Arc;

            pub use releash_lib::test_support::integration::adaptor::controller::wiring::*;
            pub(crate) mod wiring_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/controller/wiring_test.rs"
                ));
            }
        }
        pub(crate) mod workflow_startup {
            pub use releash_lib::test_support::integration::adaptor::controller::workflow_startup::*;
        }
    }
    pub(crate) mod gateway {
        pub(crate) mod application_lifecycle {
            pub use releash_lib::test_support::integration::adaptor::gateway::application_lifecycle::*;
        }

        pub(crate) mod agent_session {
            pub(crate) mod provider_availability_gateway {}

            pub(crate) mod agent_session_query_service {}

            pub(crate) mod agent_session_history_query_service {}

            pub(crate) mod agent_session_history_gateway {}

            pub use releash_lib::test_support::integration::adaptor::gateway::agent_session::*;
            pub(crate) mod agent_session_history_gateway_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/agent_session/agent_session_history_gateway_test.rs"));
            }
            pub(crate) mod agent_session_repository {
                pub use releash_lib::test_support::integration::adaptor::gateway::agent_session::agent_session_repository::*;
            }
            pub(crate) mod agent_session_repository_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/agent_session/agent_session_repository_test.rs"));
            }
            pub(crate) mod provider_agent_launch_gateway {
                pub use releash_lib::test_support::integration::adaptor::gateway::agent_session::provider_agent_launch_gateway::*;
                pub(crate) mod provider_agent_launch_gateway_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/agent_session/provider_agent_launch_gateway_test.rs"));
                }
            }
            pub(crate) mod provider_availability_gateway_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/agent_session/provider_availability_gateway_test.rs"));
            }
            pub(crate) mod session_facts {
                pub use releash_lib::test_support::integration::adaptor::gateway::agent_session::session_facts::*;
                pub(crate) mod session_facts_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/agent_session/session_facts_test.rs"));
                }
            }
        }
        pub(crate) mod app_config {
            pub use releash_lib::test_support::integration::adaptor::gateway::app_config::*;
            pub(crate) mod config_models {
                pub use releash_lib::test_support::integration::adaptor::gateway::app_config::config_models::*;
            }
            pub(crate) mod repository_impl {
                pub(crate) use crate::domain::app_config::value_objects as domain_vo;
                pub(crate) use std::fs;
                pub(crate) use std::path::PathBuf;

                pub use releash_lib::test_support::integration::adaptor::gateway::app_config::repository_impl::*;
                pub(crate) mod repository_impl_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/app_config/repository_impl_test.rs"));
                }
            }
        }
        pub(crate) mod app_data_gc {
            pub(crate) use std::path::Path;
            pub(crate) use std::path::PathBuf;
            pub(crate) use std::sync::Arc;

            pub use releash_lib::test_support::integration::adaptor::gateway::app_data_gc::*;
            pub(crate) mod mod_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/gateway/app_data_gc/mod_test.rs"
                ));
            }
        }
        pub(crate) mod code {
            pub(crate) mod diff_compute {
                pub use releash_lib::test_support::integration::adaptor::gateway::code::diff_compute::*;
            }

            pub use releash_lib::test_support::integration::adaptor::gateway::code::*;
            pub(crate) mod branch_diff {
                pub(crate) use git2::Repository;

                pub use releash_lib::test_support::integration::adaptor::gateway::code::branch_diff::*;
                pub(crate) mod branch_diff_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/code/branch_diff_test.rs"
                    ));
                }
            }
            pub(crate) mod file_content {
                pub(crate) use git2::Repository;

                pub use releash_lib::test_support::integration::adaptor::gateway::code::file_content::*;
                pub(crate) mod file_content_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/code/file_content_test.rs"
                    ));
                }
            }
            pub(crate) mod mod_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/gateway/code/mod_test.rs"
                ));
            }
            pub(crate) mod staging {

                pub(crate) use git2::Repository;

                pub use releash_lib::test_support::integration::adaptor::gateway::code::staging::*;
                pub(crate) mod staging_test_helpers {
                    pub use releash_lib::test_support::integration::code_staging_helpers::*;
                }
                pub(crate) mod staging_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/code/staging_test.rs"
                    ));
                }
            }
        }
        pub(crate) mod comment {
            pub(crate) use std::time::Duration;
            pub(crate) use std::time::Instant;

            pub use releash_lib::test_support::integration::adaptor::gateway::comment::*;
            pub(crate) mod mod_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/gateway/comment/mod_test.rs"
                ));
            }
        }
        pub(crate) mod daemon {
            pub use releash_lib::test_support::integration::adaptor::gateway::daemon::*;
        }
        pub(crate) mod external_editor {
            pub(crate) mod settings_gateway_impl {}

            pub(crate) mod scanner_impl {}

            pub use releash_lib::test_support::integration::adaptor::gateway::external_editor::*;
            pub(crate) mod launcher_impl {
                pub use releash_lib::test_support::integration::adaptor::gateway::external_editor::launcher_impl::*;
                pub(crate) mod launcher_impl_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/external_editor/launcher_impl_test.rs"));
                }
            }
        }
        pub(crate) mod failure_records {
            pub use releash_lib::test_support::integration::adaptor::gateway::failure_records::*;
        }
        pub(crate) mod git_host {
            pub(crate) mod cache {}

            pub use releash_lib::test_support::integration::adaptor::gateway::git_host::*;
            pub(crate) mod discovery {

                pub use releash_lib::test_support::integration::adaptor::gateway::git_host::discovery::*;
                pub(crate) mod discovery_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/git_host/discovery_test.rs"
                    ));
                }
            }
            pub(crate) mod github {
                pub(crate) use std::collections::HashMap;
                pub(crate) use std::sync::Arc;
                pub(crate) use std::time::Duration;

                pub use releash_lib::test_support::integration::adaptor::gateway::git_host::github::*;
                pub(crate) mod github_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/git_host/github_test.rs"
                    ));
                }
            }
        }
        pub(crate) mod identity {
            pub use releash_lib::test_support::integration::adaptor::gateway::identity::*;
        }
        pub(crate) mod local_api {
            pub(crate) use std::path::Path;

            pub use releash_lib::test_support::integration::adaptor::gateway::local_api::*;
            pub(crate) mod local_api_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/gateway/local_api_test.rs"
                ));
            }
        }
        pub(crate) mod local_event_store {
            pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::*;
            pub(crate) mod canonical_cbor {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::canonical_cbor::*;
            }
            pub(crate) mod clock {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::clock::*;
            }
            pub(crate) mod envelope {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::envelope::*;
            }
            pub(crate) mod fault {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::fault::*;
            }
            pub(crate) mod layout {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::layout::*;
            }
            pub(crate) mod maintenance {
                pub(crate) use rusqlite::Connection;

                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::maintenance::*;
                pub(crate) mod maintenance_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/local_event_store/maintenance_test.rs"));
                }
            }
            pub(crate) mod node_events {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::node_events::*;
                pub(crate) mod node_events_test {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/local_event_store/node_events_test.rs"));
                }
            }
            pub(crate) mod provider_lifecycle_codec {
                pub(crate) mod provider_lifecycle_codec_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/local_event_store/provider_lifecycle_codec_test.rs"));
                }
            }
            pub(crate) mod read_only {

                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::read_only::*;
                pub(crate) mod read_only_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/local_event_store/read_only_test.rs"));
                }
            }
            pub(crate) mod reader {
                pub(crate) use crate::adaptor::gateway::workflow::fact_codec;
                pub(crate) use rusqlite::Connection;
                pub(crate) use std::sync::Arc;

                pub(crate) use rusqlite::params;

                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::reader::*;
                pub(crate) mod reader_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/local_event_store/reader_test.rs"
                    ));
                }
            }
            pub(crate) mod schema {
                pub(crate) use rusqlite::Connection;

                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::schema::*;
                pub(crate) mod schema_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/local_event_store/schema_test.rs"
                    ));
                }
                pub(crate) use schema_tests as schema_test;
            }
            pub(crate) mod store {

                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::store::*;
                pub(crate) mod store_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/local_event_store/store_test.rs"
                    ));
                }
            }
            pub(crate) mod test_helpers {
                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::test_helpers::*;
            }
            pub(crate) mod writer {

                pub use releash_lib::test_support::integration::adaptor::gateway::local_event_store::writer::*;
            }
        }
        pub(crate) mod notion {
            pub use releash_lib::test_support::integration::adaptor::gateway::notion::*;
            pub(crate) mod service_impl {

                pub use releash_lib::test_support::integration::adaptor::gateway::notion::service_impl::*;
                pub(crate) mod service_impl_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/notion/service_impl_test.rs"
                    ));
                }
            }
        }
        pub(crate) mod provider_lifecycle {
            pub(crate) mod launch_spec {}

            pub(crate) mod hook_health_repository_impl {}

            pub(crate) mod hook_health_failure_query_impl {}

            pub(crate) mod event_repository_impl {}

            pub(crate) mod credential_gateway_impl {}

            pub use releash_lib::test_support::integration::adaptor::gateway::provider_lifecycle::*;
            pub(crate) mod provider_hook_health_failure_query_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/provider_lifecycle/provider_hook_health_failure_query_test.rs"));
            }
            pub(crate) mod provider_hook_health_repository_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/provider_lifecycle/provider_hook_health_repository_test.rs"));
            }
            pub(crate) mod provider_lifecycle_gateway_tests {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/provider_lifecycle/provider_lifecycle_gateway_test.rs"));
            }
        }
        pub(crate) mod repository {
            pub(crate) mod branch {
                pub(crate) use git2::build::CheckoutBuilder;
                pub(crate) use git2::BranchType;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::branch::*;
                pub(crate) mod branch_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/branch_test.rs"
                    ));
                }
            }
            pub(crate) mod file_watcher {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::file_watcher::*;
                pub(crate) mod file_watcher_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/file_watcher_test.rs"
                    ));
                }
            }
            pub(crate) mod git_config {
                pub use releash_lib::test_support::integration::adaptor::gateway::repository::git_config::*;
                pub(crate) mod git_config_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/git_config_test.rs"
                    ));
                }
            }
            pub(crate) mod repo_paths {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::repo_paths::*;
                pub(crate) mod repo_paths_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/repo_paths_test.rs"
                    ));
                }
            }
            pub(crate) mod scanner {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::scanner::*;
                pub(crate) mod scanner_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/scanner_test.rs"
                    ));
                }
            }
            pub(crate) mod state {
                pub(crate) use notify_debouncer_mini::DebouncedEvent;
                pub(crate) use std::path::Path;
                pub(crate) use std::path::PathBuf;
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::state::*;
                pub(crate) mod state_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/state_test.rs"
                    ));
                }
            }
            pub(crate) mod status {
                pub use releash_lib::test_support::integration::adaptor::gateway::repository::status::*;
                pub(crate) mod status_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/status_test.rs"
                    ));
                }
            }
            pub(crate) mod test_helpers {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/adaptor/gateway/repository/test_helpers.rs"
                ));
            }
            pub(crate) mod util {
                pub use releash_lib::test_support::integration::adaptor::gateway::repository::util::*;
                pub(crate) mod util_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/util_test.rs"
                    ));
                }
            }
            pub(crate) mod watch {

                pub(crate) use std::path::PathBuf;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::watch::*;
                pub(crate) mod watch_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/watch_test.rs"
                    ));
                }
            }
            pub(crate) mod worktree {
                pub(crate) use crate::adaptor::gateway::shared::git_operation;
                pub(crate) use git2::BranchType;
                pub(crate) use git2::Repository;
                pub(crate) use git2::WorktreeAddOptions;
                pub(crate) use git2::WorktreePruneOptions;
                pub(crate) use std::path::Path;
                pub(crate) use std::path::PathBuf;

                pub use releash_lib::test_support::integration::adaptor::gateway::repository::worktree::*;
                pub(crate) mod worktree_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/repository/worktree_test.rs"
                    ));
                }
            }
            pub(crate) mod worktree_operation {
                pub use releash_lib::test_support::integration::adaptor::gateway::repository::worktree_operation::*;
                pub(crate) mod worktree_operation_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/repository/worktree_operation_test.rs"));
                }
            }
        }
        pub(crate) mod shared {
            pub(crate) mod background_io {
                pub use releash_lib::test_support::integration::adaptor::gateway::shared::background_io::*;
            }
            pub(crate) mod background_worker {
                pub(crate) use std::path::PathBuf;
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::shared::background_worker::*;
                pub(crate) mod background_worker_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/shared/background_worker_test.rs"
                    ));
                }
            }
            pub(crate) mod git_operation {
                pub use releash_lib::test_support::integration::adaptor::gateway::shared::git_operation::*;
                pub(crate) mod git_operation_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/shared/git_operation_test.rs"
                    ));
                }
            }
        }
        pub(crate) mod state_subscription_reads {
            pub use releash_lib::test_support::integration::adaptor::gateway::state_subscription_reads::*;
            pub(crate) mod state_subscription_reads_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/adaptor/gateway/state_subscription_reads_test.rs"
                ));
            }
        }
        pub(crate) mod telemetry {
            pub use releash_lib::test_support::integration::adaptor::gateway::telemetry::*;
        }
        pub(crate) mod terminal_surface {
            pub(crate) mod event_source {
                pub use releash_lib::test_support::integration::adaptor::gateway::terminal_surface::event_source::*;
            }
            pub(crate) mod runtime_gateway_impl {
                pub(crate) use std::sync::Arc;

                pub(crate) use parking_lot::Mutex;

                pub use releash_lib::test_support::integration::adaptor::gateway::terminal_surface::runtime_gateway_impl::*;
                pub(crate) mod runtime_gateway_impl_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/terminal_surface/runtime_gateway_impl_test.rs"));
                }
                pub(crate) mod test_support {

                    pub use releash_lib::test_support::integration::adaptor::gateway::terminal_surface::runtime_gateway_impl::test_support::*;
                }
            }
        }
        pub(crate) mod workflow {
            pub(crate) mod span_map {}

            pub(crate) mod secret_source {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::secret_source::*;
            }

            pub(crate) mod builtin {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::builtin::*;
            }

            pub use releash_lib::test_support::integration::adaptor::gateway::workflow::*;
            pub(crate) mod definition_repository {
                pub(crate) use crate::adaptor::gateway::workflow::builtin;
                pub(crate) use std::fs;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::definition_repository::*;
                pub(crate) mod definition_repository_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/definition_repository_test.rs"));
                }
            }
            pub(crate) mod diagnostics {
                pub(crate) use crate::adaptor::gateway::workflow::builtin;
                pub(crate) use std::collections::HashMap;
                pub(crate) use std::collections::HashSet;
                pub(crate) use std::path::Path;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::diagnostics::*;
                pub(crate) mod delegate_diagnostics_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/delegate_diagnostics_test.rs"));
                }
                pub(crate) mod diagnostics_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/diagnostics_test.rs"
                    ));
                }
            }
            pub(crate) mod diagnostics_gateway {
                pub(crate) use crate::adaptor::gateway::workflow::diagnostics;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::diagnostics_gateway::*;
                pub(crate) mod diagnostics_gateway_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/diagnostics_gateway_test.rs"));
                }
            }
            pub(crate) mod editor_gateway {
                pub(crate) use crate::adaptor::gateway::workflow::builtin;
                pub(crate) use crate::adaptor::gateway::workflow::facet;
                pub(crate) use crate::adaptor::gateway::workflow::storage;
                pub(crate) use std::path::PathBuf;
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::editor_gateway::*;
                pub(crate) mod editor_gateway_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/editor_gateway_test.rs"
                    ));
                }
            }
            pub(crate) mod event {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::event::*;
            }
            pub(crate) mod event_repository {

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::event_repository::*;
                pub(crate) mod event_repository_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/event_repository_test.rs"
                    ));
                }
            }
            pub(crate) mod execution_archive_repository {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::execution_archive_repository::*;
                pub(crate) mod execution_archive_repository_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/execution_archive_repository_test.rs"));
                }
            }
            pub(crate) mod execution_projection_repository {
                pub(crate) use crate::domain::workflow::services::fact_replay;
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::execution_projection_repository::*;
                pub(crate) mod execution_projection_repository_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/execution_projection_repository_test.rs"));
                }
            }
            pub(crate) mod facet {
                pub(crate) use crate::adaptor::gateway::workflow::builtin;
                pub(crate) use std::fs;
                pub(crate) use std::path::Path;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::facet::*;
                pub(crate) mod facet_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/facet_test.rs"
                    ));
                }
            }
            pub(crate) mod facet_repository {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::facet_repository::*;
                pub(crate) mod facet_repository_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/facet_repository_test.rs"
                    ));
                }
            }
            pub(crate) mod fact_codec {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::fact_codec::*;
            }
            pub(crate) mod fact_log {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::fact_log::*;
                pub(crate) mod fact_log_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/fact_log_test.rs"
                    ));
                }
            }
            pub(crate) mod lua {
                pub(crate) use crate::domain::workflow::services::reference;
                pub(crate) use std::collections::HashMap;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::lua::*;
                pub(crate) mod field_span {
                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::lua::field_span::*;
                    pub(crate) mod field_span_integration_tests {
                        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/lua/field_span_test.rs"));
                    }
                }
                pub(crate) mod mod_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/lua/mod_test.rs"
                    ));
                }
                pub(crate) mod stubs {
                    pub(crate) use crate::adaptor::gateway::workflow::builtin;
                    pub(crate) use std::fs;
                    pub(crate) use std::path::Path;

                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::lua::stubs::*;
                    pub(crate) mod stubs_integration_tests {
                        include!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/tests/support/internal/adaptor/gateway/workflow/lua/stubs_test.rs"
                        ));
                    }
                }
            }
            pub(crate) mod mapper {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::mapper::*;
            }
            pub(crate) mod node_process {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::node_process::*;
            }
            pub(crate) mod node_session_boundary {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::node_session_boundary::*;
            }
            pub(crate) mod runtime_command_gateway {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::runtime_command_gateway::*;
                pub(crate) mod runtime_command_gateway_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/runtime_command_gateway_test.rs"));
                }
            }
            pub(crate) mod runtime_resolver {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::runtime_resolver::*;
                pub(crate) mod runtime_resolver_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/runtime_resolver_test.rs"
                    ));
                }
            }
            pub(crate) mod schema {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::schema::*;
            }
            pub(crate) mod secret_source_gateway {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::secret_source_gateway::*;
                pub(crate) mod secret_source_gateway_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/secret_source_gateway_test.rs"));
                }
            }
            pub(crate) mod startup_repository {

                pub(crate) use crate::adaptor::gateway::workflow::fact_log;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::startup_repository::*;
                pub(crate) mod startup_repository_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/startup_repository_test.rs"));
                }
            }
            pub(crate) mod storage {
                pub(crate) use crate::adaptor::gateway::workflow::builtin;
                pub(crate) use std::path::Path;

                pub(crate) use std::fs;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::storage::*;
                pub(crate) mod storage_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/storage_test.rs"
                    ));
                }
            }
            pub(crate) mod stored_definition {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::stored_definition::*;
            }
            pub(crate) mod test_support {
                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::test_support::*;
            }
            pub(crate) mod workflow_host {
                pub(crate) mod runtime_session {
                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::runtime_session::*;
                }

                pub(crate) mod output_limit {
                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::output_limit::*;
                }

                pub(crate) mod activation {}

                pub(crate) use std::collections::BTreeMap;
                pub(crate) use std::sync::Arc;

                pub(crate) use crate::adaptor::gateway::workflow::fact_codec;
                pub(crate) use crate::adaptor::gateway::workflow::fact_log as workflow_fact_log;
                pub(crate) use crate::adaptor::gateway::workflow::secret_source;
                pub(crate) use crate::adaptor::gateway::workflow::workflow_host::output_limit as workflow_output_limit;
                pub(crate) use crate::adaptor::gateway::workflow::workflow_host::runtime_session as workflow_runtime_session;
                pub(crate) use crate::domain::workflow::services::reference as workflow_reference;
                pub(crate) use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
                pub(crate) use crate::infrastructure::process::command_runner as workflow_command_runner;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::*;
                pub(crate) mod delegate {
                    pub(crate) use super::*;

                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::delegate::*;
                    pub(crate) mod delegate_tests {
                        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/delegate_test.rs"));
                    }
                }
                pub(crate) mod isolated_worktree {

                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::isolated_worktree::*;
                }
                pub(crate) mod isolated_worktree_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/isolated_worktree_test.rs"));
                }
                pub(crate) mod node_startup {
                    pub(crate) use super::*;

                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::node_startup::*;
                    pub(crate) mod node_startup_tests {
                        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/node_startup_test.rs"));
                    }
                }
                pub(crate) mod prompt_rendering {
                    pub use releash_lib::test_support::integration::adaptor::gateway::workflow::workflow_host::prompt_rendering::*;
                    pub(crate) mod prompt_rendering_integration_tests {
                        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/prompt_rendering_test.rs"));
                    }
                }
                pub(crate) mod secret_redaction_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/secret_redaction_test.rs"));
                }
                pub(crate) mod shutdown_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/shutdown_test.rs"));
                }
                pub(crate) mod test_helpers {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workflow/workflow_host/test_helpers.rs"));
                }
                pub(crate) mod workflow_host_persistence_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/workflow_host_test.rs"
                    ));
                }
                pub(crate) use workflow_host_persistence_tests::workflow_host_tests as workflow_host_integration_tests;
            }
            pub(crate) mod worktree_context {
                pub(crate) use crate::adaptor::gateway::local_event_store::node_events;
                pub(crate) use crate::adaptor::gateway::workflow::stored_definition;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::worktree_context::*;
                pub(crate) mod worktree_context_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/worktree_context_test.rs"
                    ));
                }
            }
            pub(crate) mod worktree_gateway {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workflow::worktree_gateway::*;
                pub(crate) mod worktree_gateway_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workflow/worktree_gateway_test.rs"
                    ));
                }
            }
        }
        pub(crate) mod workspace_state {
            pub use releash_lib::test_support::integration::adaptor::gateway::workspace_state::*;
            pub(crate) mod repository_impl {
                pub use releash_lib::test_support::integration::adaptor::gateway::workspace_state::repository_impl::*;
                pub(crate) mod repository_impl_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workspace_state/repository_impl_test.rs"));
                }
            }
        }
        pub(crate) mod workspace_tree {
            pub use releash_lib::test_support::integration::adaptor::gateway::workspace_tree::*;
            pub(crate) mod query_service {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workspace_tree::query_service::*;
                pub(crate) mod query_service_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workspace_tree/query_service_test.rs"));
                }
            }
            pub(crate) mod repository {
                pub(crate) use std::sync::Arc;

                pub use releash_lib::test_support::integration::adaptor::gateway::workspace_tree::repository::*;
                pub(crate) mod legacy_projection_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/adaptor/gateway/workspace_tree/legacy_projection_test.rs"));
                }
                pub(crate) mod repository_integration_tests {
                    include!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/support/internal/adaptor/gateway/workspace_tree/repository_test.rs"
                    ));
                }
            }
        }
    }
    pub(crate) mod presenter {
        pub(crate) use releash_lib::test_support::integration::client_wire as client;

        pub(crate) mod connect {
            pub use releash_lib::test_support::integration::adaptor::presenter::connect::*;
        }
        pub(crate) mod error {
            pub use releash_lib::test_support::integration::adaptor::presenter::error::*;
        }
        pub(crate) mod provider_lifecycle_response {
            pub use releash_lib::test_support::integration::adaptor::presenter::provider_lifecycle_response::*;
        }
        pub(crate) mod state_subscription {
            pub use releash_lib::test_support::integration::adaptor::presenter::state_subscription::*;
        }
        pub(crate) mod state_subscription_wire {
            pub use releash_lib::test_support::integration::adaptor::presenter::state_subscription_wire::*;
        }
        pub(crate) mod terminal_event_hub {
            pub use releash_lib::test_support::integration::adaptor::presenter::terminal_event_hub::*;
        }
        pub(crate) mod terminal_subscription {
            pub use releash_lib::test_support::integration::adaptor::presenter::terminal_subscription::*;
        }
        pub(crate) mod workflow {
            pub use releash_lib::test_support::integration::adaptor::presenter::workflow::*;
        }
        pub(crate) mod workflow_api {
            pub use releash_lib::test_support::integration::adaptor::presenter::workflow_api::*;
        }
    }
}
pub(crate) mod agent_session_tui_acceptance {
    pub(crate) use std::sync::Arc;
    pub(crate) use std::time::Duration;

    pub use releash_lib::test_support::integration::agent_session_tui_acceptance::*;
    pub(crate) mod agent_session_tui_acceptance_tests {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/internal/agent_session_tui_acceptance_test.rs"
        ));
    }
}
pub(crate) mod cli {
    pub(crate) mod file_direct {
        pub use releash_lib::test_support::integration::cli::file_direct::*;
    }

    pub(crate) mod api_client {
        pub(crate) use std::path::Path;

        pub use releash_lib::test_support::integration::cli::api_client::*;
        pub(crate) mod api_client_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/api_client_test.rs"
            ));
        }
    }
    pub(crate) mod common {

        pub use releash_lib::test_support::integration::cli::common::*;
        pub(crate) mod common_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/common_test.rs"
            ));
        }
        pub(crate) mod test_helpers_common {
            pub use releash_lib::test_support::integration::cli::common::test_helpers_common::*;
        }
        pub(crate) mod test_support {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/common_test_support.rs"
            ));
        }
    }
    pub(crate) mod diagnostics {
        pub(crate) use std::path::Path;

        pub use releash_lib::test_support::integration::cli::diagnostics::*;
        pub(crate) mod diagnostics_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/diagnostics_test.rs"
            ));
        }
    }
    pub(crate) mod hook {
        pub use releash_lib::test_support::integration::cli::hook::*;
        pub(crate) mod hook_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/hook_test.rs"
            ));
        }
    }
    pub(crate) mod output {
        pub(crate) use std::path::Path;

        pub use releash_lib::test_support::integration::cli::output::*;
        pub(crate) mod output_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/output_test.rs"
            ));
        }
    }
    pub(crate) mod review {

        pub use releash_lib::test_support::integration::cli::review::*;
        pub(crate) mod review_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/review_test.rs"
            ));
        }
        pub(crate) mod review_integration_tests_extra {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/review_test_extra.rs"
            ));
        }
    }
    pub(crate) mod test_helpers {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/internal/cli/test_helpers.rs"
        ));
    }
    pub(crate) mod workflow {
        pub(crate) use crate::cli::file_direct;
        pub(crate) use std::path::Path;

        pub use releash_lib::test_support::integration::cli::workflow::*;
        pub(crate) mod workflow_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/cli/workflow_test.rs"
            ));
        }
    }
}
pub(crate) mod client_api_acceptance {
    pub use releash_lib::test_support::integration::client_api_acceptance::*;
}
pub(crate) mod common {
    pub(crate) mod operation_context {
        pub use releash_lib::test_support::integration::common::operation_context::*;
    }
    pub(crate) mod priority {
        pub use releash_lib::test_support::integration::common::priority::*;
    }
    pub(crate) mod retry {
        pub use releash_lib::test_support::integration::common::retry::*;
    }
}
pub(crate) mod desktop_api {
    pub use releash_lib::test_support::integration::desktop_api::*;
}
pub(crate) mod domain {
    pub(crate) mod agent_session {
        pub(crate) mod provider_terminal_gateway {}

        pub(crate) mod provider_session_title_gateway {}

        pub(crate) mod provider_launch_gateway {}

        pub(crate) mod provider_launch {}

        pub(crate) mod provider_history_gateway {}

        pub(crate) mod provider_availability_gateway {}

        pub(crate) mod launch_identity {}

        pub use releash_lib::test_support::integration::domain::agent_session::*;
        pub(crate) mod aggregates {
            pub(crate) mod provider_registry {}

            pub(crate) mod agent_session {}

            pub use releash_lib::test_support::integration::domain::agent_session::aggregates::*;
        }
        pub(crate) mod repository {
            pub use releash_lib::test_support::integration::domain::agent_session::repository::*;
        }
    }
    pub(crate) mod app_config {
        pub use releash_lib::test_support::integration::domain::app_config::*;
        pub(crate) mod repository {
            pub use releash_lib::test_support::integration::domain::app_config::repository::*;
        }
        pub(crate) mod value_objects {
            pub use releash_lib::test_support::integration::domain::app_config::value_objects::*;
        }
    }
    pub(crate) mod app_data_gc {
        pub use releash_lib::test_support::integration::domain::app_data_gc::*;
    }
    pub(crate) mod code {
        pub(crate) mod services {
            pub(crate) mod hunk {
                pub use releash_lib::test_support::integration::domain::code::services::hunk::*;
            }
        }

        pub(crate) mod repository {}

        pub use releash_lib::test_support::integration::domain::code::*;
    }
    pub(crate) mod comment {
        pub use releash_lib::test_support::integration::domain::comment::*;
    }
    pub(crate) mod daemon {
        pub(crate) mod identity {}

        pub use releash_lib::test_support::integration::domain::daemon::*;
    }
    pub(crate) mod external_editor {
        pub(crate) mod gateway {}

        pub use releash_lib::test_support::integration::domain::external_editor::*;
        pub(crate) mod services {
            pub use releash_lib::test_support::integration::domain::external_editor::services::*;
            pub(crate) mod services_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/domain/external_editor/services_test.rs"
                ));
            }
        }
    }
    pub(crate) mod failure {
        pub use releash_lib::test_support::integration::domain::failure::*;
    }
    pub(crate) mod git_host {
        pub use releash_lib::test_support::integration::domain::git_host::*;
        pub(crate) mod value_objects {
            pub(crate) mod pr {}

            pub(crate) mod cache {}

            pub(crate) mod issue {
                pub use releash_lib::test_support::integration::domain::git_host::value_objects::issue::*;
            }
        }
    }
    pub(crate) mod local_event {
        pub(crate) mod repository {}

        pub(crate) mod query {}

        pub(crate) mod mutation {}

        pub(crate) mod identifiers {}

        pub(crate) mod failure {}

        pub(crate) mod events {}

        pub(crate) mod batch {}

        pub use releash_lib::test_support::integration::domain::local_event::*;
    }
    pub(crate) mod notion {
        pub(crate) mod value_objects {}

        pub(crate) mod gateway {}

        pub use releash_lib::test_support::integration::domain::notion::*;
    }
    pub(crate) mod provider_lifecycle {
        pub(crate) mod value_objects {
            pub(crate) mod scoped_provider_lifecycle_event {}

            pub(crate) mod provider_lifecycle_unavailable {}

            pub(crate) mod provider_lifecycle_slot_id {}

            pub(crate) mod provider_lifecycle_signal {}

            pub(crate) mod provider_lifecycle_scope {}

            pub(crate) mod provider_lifecycle_outcome {}

            pub(crate) mod provider_lifecycle_ingress_result {}

            pub(crate) mod provider_lifecycle_event {}

            pub(crate) mod provider_kind {}

            pub(crate) mod armed_provider_lifecycle {}
        }

        pub(crate) mod repository {}

        pub(crate) mod entities {
            pub(crate) mod provider_lifecycle_binding {}

            pub(crate) mod provider_hook_health {}
        }

        pub use releash_lib::test_support::integration::domain::provider_lifecycle::*;
    }
    pub(crate) mod repository {
        pub(crate) mod value_objects {
            pub(crate) mod repo_path {}
        }

        pub(crate) mod entities {
            pub(crate) mod worktree {}
        }

        pub use releash_lib::test_support::integration::domain::repository::*;
        pub(crate) mod file_watcher {
            pub use releash_lib::test_support::integration::domain::repository::file_watcher::*;
        }
        pub(crate) mod worktree_operation {
            pub use releash_lib::test_support::integration::domain::repository::worktree_operation::*;
        }
    }
    pub(crate) mod terminal_surface {
        pub(crate) mod value_objects {
            pub(crate) mod terminal_surface_owner {}

            pub(crate) mod terminal_surface_checkpoint {}

            pub(crate) mod terminal_process_state {}

            pub(crate) mod terminal_process_launch {}
        }

        pub(crate) mod gateway {}

        pub use releash_lib::test_support::integration::domain::terminal_surface::*;
        pub(crate) mod entities {
            pub(crate) mod terminal_surface {}

            pub use releash_lib::test_support::integration::domain::terminal_surface::entities::*;
        }
    }
    pub(crate) mod workflow {
        pub(crate) mod gateway {}

        pub(crate) use crate::domain::workflow::services::secret_masker;
        pub(crate) use crate::domain::workflow::services::validation;

        pub use releash_lib::test_support::integration::domain::workflow::*;
        pub(crate) mod entities {
            pub(crate) mod workflow_execution {
                pub(crate) mod delegate {}

                pub use releash_lib::test_support::integration::domain::workflow::entities::workflow_execution::*;
                pub(crate) mod mod_integration_tests {
                    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/domain/workflow/entities/workflow_execution/mod_test.rs"));
                }
            }
        }
        pub(crate) mod repository {
            pub use releash_lib::test_support::integration::domain::workflow::repository::*;
        }
        pub(crate) mod services {
            pub(crate) mod secret_masker {
                pub use releash_lib::test_support::integration::domain::workflow::services::secret_masker::*;
            }

            pub(crate) mod fact_replay {
                pub use releash_lib::test_support::integration::domain::workflow::services::fact_replay::*;
            }
            pub(crate) mod reference {
                pub use releash_lib::test_support::integration::domain::workflow::services::reference::*;
            }
            pub(crate) mod routing {
                pub use releash_lib::test_support::integration::domain::workflow::services::routing::*;
            }
            pub(crate) mod validation {
                pub use releash_lib::test_support::integration::domain::workflow::services::validation::*;
            }
        }
        pub(crate) mod value_objects {
            pub(crate) mod worktree_origin {}

            pub(crate) mod state {}

            pub(crate) mod runtime_projection {}

            pub(crate) mod runtime_event {}

            pub(crate) mod predicate {}

            pub(crate) mod node_fact {}

            pub(crate) mod node_execution {}

            pub(crate) mod ids {}

            pub(crate) mod field_path {}

            pub(crate) mod failure {}

            pub(crate) mod facet {}

            pub(crate) mod execution_metadata {}

            pub(crate) mod execution {}

            pub(crate) mod definition {}

            pub use releash_lib::test_support::integration::domain::workflow::value_objects::*;
        }
    }
    pub(crate) mod workspace_state {
        pub(crate) mod repository {}

        pub use releash_lib::test_support::integration::domain::workspace_state::*;
        pub(crate) mod services {
            pub use releash_lib::test_support::integration::domain::workspace_state::services::*;
            pub(crate) mod services_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/domain/workspace_state/services_test.rs"
                ));
            }
        }
        pub(crate) mod value_objects {
            pub use releash_lib::test_support::integration::domain::workspace_state::value_objects::*;
            pub(crate) mod workspace_tabs_state {
                pub use releash_lib::test_support::integration::domain::workspace_state::value_objects::workspace_tabs_state::*;
            }
        }
    }
    pub(crate) mod workspace_tree {
        pub(crate) mod value_objects {}

        pub(crate) mod repository {}

        pub(crate) mod entities {}

        pub use releash_lib::test_support::integration::domain::workspace_tree::*;
    }
}
pub(crate) mod infrastructure {
    pub(crate) mod state_subscription {}

    pub(crate) mod app_data_path {
        pub use releash_lib::test_support::integration::infrastructure::app_data_path::*;
    }
    pub(crate) mod file_lock {
        pub(crate) use std::fs::File;
        pub(crate) use std::time::Instant;

        pub use releash_lib::test_support::integration::infrastructure::file_lock::*;
        pub(crate) mod file_lock_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/infrastructure/file_lock_test.rs"
            ));
        }
    }
    pub(crate) mod file_watcher {
        pub use releash_lib::test_support::integration::infrastructure::file_watcher::*;
        pub(crate) mod mod_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/infrastructure/file_watcher/mod_test.rs"
            ));
        }
    }
    pub(crate) mod local_api {
        pub(crate) mod client_token {}

        pub use releash_lib::test_support::integration::infrastructure::local_api::*;
        pub(crate) mod client {
            pub(crate) use reqwest::blocking::Client;
            pub(crate) use std::time::Duration;
            pub(crate) use url::Url;

            pub use releash_lib::test_support::integration::infrastructure::local_api::client::*;
            pub(crate) mod client_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/local_api/client_test.rs"
                ));
            }
        }
        pub(crate) mod discovery {
            pub use releash_lib::test_support::integration::infrastructure::local_api::discovery::*;
            pub(crate) mod discovery_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/local_api/discovery_test.rs"
                ));
            }
        }
        pub(crate) mod server {

            pub(crate) use axum::Router;

            pub use releash_lib::test_support::integration::infrastructure::local_api::server::*;
            pub(crate) mod server_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/local_api/server_test.rs"
                ));
            }
        }
    }
    pub(crate) mod local_event_store_connection {
        pub(crate) use rusqlite::Connection;
        pub(crate) use std::time::Duration;

        pub use releash_lib::test_support::integration::infrastructure::local_event_store_connection::*;
        pub(crate) mod local_event_store_connection_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/infrastructure/local_event_store_connection_test.rs"
            ));
        }
    }
    pub(crate) mod local_log {

        pub use releash_lib::test_support::integration::infrastructure::local_log::*;
        pub(crate) mod local_log_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/infrastructure/local_log_test.rs"
            ));
        }
    }
    pub(crate) mod lua {
        pub(crate) mod evaluator {
            pub(crate) use std::collections::BTreeMap;
            pub(crate) use std::fs;
            pub(crate) use std::path::Path;

            pub use releash_lib::test_support::integration::infrastructure::lua::evaluator::*;
            pub(crate) mod evaluator_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/lua/evaluator_test.rs"
                ));
            }
        }
    }
    pub(crate) mod platform {
        pub(crate) mod path_aliases {
            pub(crate) use std::path::PathBuf;

            pub use releash_lib::test_support::integration::infrastructure::platform::path_aliases::*;
            pub(crate) mod path_aliases_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/platform/path_aliases_test.rs"
                ));
            }
        }
    }
    pub(crate) mod process {
        pub(crate) mod command_runner {
            pub use releash_lib::test_support::integration::infrastructure::process::command_runner::*;
            pub(crate) mod command_runner_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/process/command_runner_test.rs"
                ));
            }
        }
        pub(crate) mod executable_probe {
            pub use releash_lib::test_support::integration::infrastructure::process::executable_probe::*;
            pub(crate) mod executable_probe_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/process/executable_probe_test.rs"
                ));
            }
        }
        pub(crate) mod fd_limit {
            pub use releash_lib::test_support::integration::infrastructure::process::fd_limit::*;
            pub(crate) mod fd_limit_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/process/fd_limit_test.rs"
                ));
            }
        }
        pub(crate) mod output {
            pub(crate) use std::io;

            pub use releash_lib::test_support::integration::infrastructure::process::output::*;
            pub(crate) mod process_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/process/output_test.rs"
                ));
            }
        }
        pub(crate) mod search_path {
            pub use releash_lib::test_support::integration::infrastructure::process::search_path::*;
            pub(crate) mod search_path_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/process/search_path_test.rs"
                ));
            }
        }
    }
    pub(crate) mod provider_lifecycle {
        pub(crate) mod health_marker {}

        pub use releash_lib::test_support::integration::infrastructure::provider_lifecycle::*;
    }
    pub(crate) mod telemetry {
        pub(crate) mod crash {
            pub use releash_lib::test_support::integration::infrastructure::telemetry::crash::*;
        }
        pub(crate) mod metrics {
            pub use releash_lib::test_support::integration::infrastructure::telemetry::metrics::*;
        }
    }
    pub(crate) mod terminal {
        pub(crate) mod checkpoint_journal {}

        pub(crate) mod native_pty {
            pub use releash_lib::test_support::integration::infrastructure::terminal::native_pty::*;
        }
        pub(crate) mod shell_integration {
            pub use releash_lib::test_support::integration::infrastructure::terminal::shell_integration::*;
            pub(crate) mod shell_integration_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/terminal/shell_integration_test.rs"
                ));
            }
        }
        pub(crate) mod terminal_emulator {

            pub use releash_lib::test_support::integration::infrastructure::terminal::terminal_emulator::*;
            pub(crate) mod terminal_emulator_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/infrastructure/terminal/terminal_emulator_test.rs"
                ));
            }
        }
    }
    pub(crate) mod timer {
        pub use releash_lib::test_support::integration::infrastructure::timer::*;
    }
}
pub(crate) mod test_support {
    pub use releash_lib::test_support::integration::test_support::*;

    pub(crate) mod git {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/internal/test_support/git.rs"
        ));
    }
    pub(crate) mod retry {
        pub use releash_lib::test_support::integration::test_support::retry::*;
    }
    pub(crate) mod state_subscription {
        pub(crate) use crate::test_support::state_subscription_reads::Fixture as StateReadsFixture;

        pub use releash_lib::test_support::integration::test_support::state_subscription::*;
    }
    pub(crate) mod state_subscription_reads {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/state_subscription_reads.rs"
        ));
    }
    pub(crate) mod state_subscription_scenarios {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/state_subscription_scenarios.rs"
        ));
    }
}
pub(crate) mod usecase {
    pub(crate) mod repository_error {}

    pub(crate) mod client_connection {}

    pub(crate) mod agent_session {
        pub(crate) mod usecase {}

        pub(crate) mod provider_availability {}

        pub(crate) mod agent_session_query {}

        pub(crate) mod agent_session_lifecycle {}

        pub(crate) mod agent_session_launch {}

        pub(crate) mod agent_session_history {}

        pub use releash_lib::test_support::integration::usecase::agent_session::*;
        pub(crate) mod agent_session_initial_instruction {
            pub use releash_lib::test_support::integration::usecase::agent_session::agent_session_initial_instruction::*;
        }
        pub(crate) mod agent_session_initial_instruction_tests {
            include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/internal/usecase/agent_session/agent_session_initial_instruction_test.rs"));
        }
        pub(crate) mod agent_session_lifecycle_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/agent_session/agent_session_lifecycle_test.rs"
            ));
        }
        pub(crate) mod agent_session_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/agent_session/agent_session_test.rs"
            ));
        }
        pub(crate) mod test_helpers_provider_availability {
            pub use releash_lib::test_support::integration::usecase::agent_session::test_helpers_provider_availability::*;
        }
    }
    pub(crate) mod app_config {
        pub(crate) mod usecase {}

        pub use releash_lib::test_support::integration::usecase::app_config::*;
    }
    pub(crate) mod app_data_gc {
        pub use releash_lib::test_support::integration::usecase::app_data_gc::*;
    }
    pub(crate) mod application_lifecycle {
        pub(crate) mod test_helpers {
            pub use releash_lib::test_support::integration::usecase::application_lifecycle::test_helpers::*;
        }
    }
    pub(crate) mod code_dto {
        pub use releash_lib::test_support::integration::usecase::code_dto::*;
    }
    pub(crate) mod comment {
        pub(crate) mod dto {}

        pub(crate) use std::path::Path;
        pub(crate) use std::sync::Arc;

        pub use releash_lib::test_support::integration::usecase::comment::*;
        pub(crate) mod mod_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/comment/mod_test.rs"
            ));
        }
    }
    pub(crate) mod daemon {
        pub use releash_lib::test_support::integration::usecase::daemon::*;
    }
    pub(crate) mod failure {
        pub use releash_lib::test_support::integration::usecase::failure::*;
    }
    pub(crate) mod fetched {
        pub use releash_lib::test_support::integration::usecase::fetched::*;
    }
    pub(crate) mod git_host {
        pub(crate) mod git_host_usecase {}

        pub use releash_lib::test_support::integration::usecase::git_host::*;
    }
    pub(crate) mod notion {
        pub(crate) mod error {
            pub use releash_lib::test_support::integration::usecase::notion::error::*;
        }
        pub(crate) mod usecase {
            pub use releash_lib::test_support::integration::usecase::notion::usecase::*;
        }
    }
    pub(crate) mod provider_dto {
        pub use releash_lib::test_support::integration::usecase::provider_dto::*;
    }
    pub(crate) mod provider_lifecycle {
        pub(crate) mod ingress {}

        pub(crate) mod hook_health {}

        pub use releash_lib::test_support::integration::usecase::provider_lifecycle::*;
    }
    pub(crate) mod repo_paths_usecase {
        pub use releash_lib::test_support::integration::usecase::repo_paths_usecase::*;
    }
    pub(crate) mod repository_dto {
        pub use releash_lib::test_support::integration::usecase::repository_dto::*;
    }
    pub(crate) mod repository_state {
        pub use releash_lib::test_support::integration::usecase::repository_state::*;
        pub(crate) mod runtime {
            pub use releash_lib::test_support::integration::usecase::repository_state::runtime::*;
            pub(crate) mod test_helpers_runtime {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/usecase/repository_state/test_helpers_runtime.rs"
                ));
            }
            pub(crate) mod tests_support {
                pub(crate) use super::test_helpers_runtime::CanonicalWorktreePathNormalizer;
                pub use releash_lib::test_support::integration::repository_state_helpers::*;
            }
        }
        pub(crate) mod scanner {
            pub use releash_lib::test_support::integration::usecase::repository_state::scanner::*;
        }
        pub(crate) mod service {
            pub(crate) use service_integration_tests::tests as service_tests;

            pub(crate) use std::sync::Arc;

            pub use releash_lib::test_support::integration::usecase::repository_state::service::*;
            pub(crate) mod service_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/usecase/repository_state/service_test.rs"
                ));
            }
        }
        pub(crate) mod snapshot {
            pub use releash_lib::test_support::integration::usecase::repository_state::snapshot::*;
        }
        pub(crate) mod worker {
            pub use releash_lib::test_support::integration::usecase::repository_state::worker::*;
        }
        pub(crate) mod worktree {
            pub use releash_lib::test_support::integration::usecase::repository_state::worktree::*;
        }
    }
    pub(crate) mod repository_usecase {
        pub(crate) use std::sync::Arc;

        pub use releash_lib::test_support::integration::usecase::repository_usecase::*;
        pub(crate) mod repository_usecase_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/repository_usecase_test.rs"
            ));
        }
    }
    pub(crate) mod retry {
        pub use releash_lib::test_support::integration::usecase::retry::*;
    }
    pub(crate) mod review_usecase {
        pub use releash_lib::test_support::integration::usecase::review_usecase::*;
    }
    pub(crate) mod state_subscription {
        pub(crate) mod value {}

        pub(crate) mod target {}

        pub(crate) mod reads {}

        pub(crate) use std::sync::Arc;

        pub use releash_lib::test_support::integration::usecase::state_subscription::*;
        pub(crate) mod state_subscription_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/state_subscription_test.rs"
            ));
        }
        pub(crate) mod state_subscription_test_helpers {

            pub use releash_lib::test_support::integration::usecase::state_subscription::state_subscription_test_helpers::*;
        }
    }
    pub(crate) mod terminal_surface {
        pub(crate) mod application {
            pub use releash_lib::test_support::integration::usecase::terminal_surface::application::*;
        }
        pub(crate) mod lifecycle_usecase {
            pub use releash_lib::test_support::integration::usecase::terminal_surface::lifecycle_usecase::*;
        }
        pub(crate) mod output {
            pub use releash_lib::test_support::integration::usecase::terminal_surface::output::*;
        }
        pub(crate) mod spawn_usecase {
            pub use releash_lib::test_support::integration::usecase::terminal_surface::spawn_usecase::*;
        }
        pub(crate) mod test_helpers_io {
            pub use releash_lib::test_support::integration::usecase::terminal_surface::test_helpers_io::*;
        }
    }
    pub(crate) mod watcher {
        pub(crate) use std::sync::Arc;

        pub use releash_lib::test_support::integration::usecase::watcher::*;
        pub(crate) mod watcher_integration_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/watcher_test.rs"
            ));
        }
        pub(crate) mod watcher_test_helpers {
            pub use releash_lib::test_support::integration::usecase::watcher::watcher_test_helpers::*;
        }
    }
    pub(crate) mod workflow {
        pub(crate) use releash_lib::test_support::integration::workflow_archive_helpers::NoopArchiveRepository;

        pub(crate) mod runtime_command {}

        pub use releash_lib::test_support::integration::usecase::workflow::*;
        pub(crate) mod command {
            pub(crate) mod submit_output {}

            pub(crate) mod retry_node {}

            pub(crate) mod approval {}

            pub(crate) mod abort_execution {}

            pub use releash_lib::test_support::integration::usecase::workflow::command::*;
        }
        pub(crate) mod control_plane {
            pub use releash_lib::test_support::integration::usecase::workflow::control_plane::*;
        }
        pub(crate) mod delegate {
            pub use releash_lib::test_support::integration::usecase::workflow::delegate::*;
        }
        pub(crate) mod diagnostic_dto {
            pub use releash_lib::test_support::integration::usecase::workflow::diagnostic_dto::*;
        }
        pub(crate) mod dto {
            pub use releash_lib::test_support::integration::usecase::workflow::dto::*;
        }
        pub(crate) mod execution_archive {
            pub(crate) mod execution_archive_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/usecase/workflow/execution_archive_test.rs"
                ));
            }
        }
        pub(crate) mod mod_tests {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/internal/usecase/workflow/mod_test.rs"
            ));
        }

        pub(crate) mod node_startup {
            pub use releash_lib::test_support::integration::usecase::workflow::node_startup::*;
        }
        pub(crate) mod output {
            pub use releash_lib::test_support::integration::usecase::workflow::output::*;
        }
        pub(crate) mod ports {
            pub use releash_lib::test_support::integration::usecase::workflow::ports::*;
        }
        pub(crate) mod query_service {
            pub use releash_lib::test_support::integration::usecase::workflow::query_service::*;
        }
        pub(crate) mod runtime_driver {
            pub use releash_lib::test_support::integration::usecase::workflow::runtime_driver::*;
        }
        pub(crate) mod runtime_error {
            pub use releash_lib::test_support::integration::usecase::workflow::runtime_error::*;
        }
        pub(crate) mod runtime_resolver {
            pub use releash_lib::test_support::integration::usecase::workflow::runtime_resolver::*;
        }
        pub(crate) mod runtime_snapshot {
            pub use releash_lib::test_support::integration::usecase::workflow::runtime_snapshot::*;
        }
        pub(crate) mod startup {
            pub use releash_lib::test_support::integration::usecase::workflow::startup::*;
        }
        pub(crate) mod workspace_tree {

            pub(crate) mod workspace_tree_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/usecase/workflow/workspace_tree_test.rs"
                ));
            }
        }
    }
    pub(crate) mod workspace_state {
        pub(crate) mod dto {
            pub use releash_lib::test_support::integration::usecase::workspace_state::dto::*;
        }
        pub(crate) mod usecase {
            pub use releash_lib::test_support::integration::usecase::workspace_state::usecase::*;
        }
    }
    pub(crate) mod workspace_tree {
        pub(crate) mod worktree_path {}

        pub(crate) mod query_service {}

        pub use releash_lib::test_support::integration::usecase::workspace_tree::*;
        pub(crate) mod list {
            pub(crate) use std::sync::Arc;

            pub use releash_lib::test_support::integration::usecase::workspace_tree::list::*;
            pub(crate) mod list_integration_tests {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/internal/usecase/workspace_tree/list_test.rs"
                ));
            }
        }
    }
    pub(crate) mod worktree_operation {
        pub use releash_lib::test_support::integration::usecase::worktree_operation::*;
    }
}
