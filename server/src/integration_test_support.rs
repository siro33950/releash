pub mod code {

    pub use crate::adaptor::gateway::code::branch_diff::get_branch_diff_summary;
    pub use crate::adaptor::gateway::code::diff_compute::diff_buffers;
    pub use crate::adaptor::gateway::code::file_content::binary_by_attributes;
    pub use crate::adaptor::gateway::code::file_content::review_blob_at_branch_base;
    pub use crate::adaptor::gateway::code::file_content::review_blob_at_ref;
    pub use crate::adaptor::gateway::code::file_content::review_blob_staged;
    pub use crate::adaptor::gateway::code::file_content::FileContentGateway;

    pub use crate::adaptor::gateway::code::resolve_merge_base_commit;
    pub use crate::adaptor::gateway::code::staging::git_stage;
    pub use crate::adaptor::gateway::code::staging::git_stage_hunk;
    pub use crate::adaptor::gateway::code::staging::git_unstage;
    pub use crate::adaptor::gateway::code::staging::git_unstage_hunk;
    pub use crate::adaptor::gateway::code::staging::StagingGateway;
    pub use crate::adaptor::gateway::code::test_helpers::set_git_program;
    pub use crate::domain::code::error::CodeError;
    pub use crate::domain::code::services::hunk::assign_hunk_ids;
    pub use crate::domain::code::services::hunk::compute_change_groups;
    pub use crate::domain::code::services::hunk::generate_group_patch;
    pub use crate::domain::code::value_objects::hunk::ChangeGroup;
    pub use crate::domain::code::value_objects::hunk::Hunk;
}
pub mod daemon {
    pub use crate::adaptor::controller::daemon::client_priority_interceptor;
    pub use crate::adaptor::controller::daemon::compose;
    pub use crate::adaptor::controller::daemon::default_timeout;
    pub use crate::adaptor::controller::daemon::migrate_legacy_execution_archives;
    pub use crate::adaptor::controller::daemon::shutdown_with_deadline;
    pub use crate::adaptor::controller::daemon::Daemon;
    pub use crate::adaptor::gateway::daemon::serving;
    pub use crate::adaptor::gateway::daemon::InMemoryDaemonRepository;
    pub use crate::domain::daemon::DaemonRequest;
    pub use crate::domain::daemon::StopAcceptance;
    pub use crate::domain::daemon::StopRequest;

    pub use crate::usecase::daemon::DaemonUsecase;
}
pub mod persistence {
    pub use crate::adaptor::gateway::app_data_gc::apply_canonical_runtime_owners;
    pub use crate::adaptor::gateway::app_data_gc::build_startup_gc_request;
    pub use crate::adaptor::gateway::app_data_gc::StdGcFileSystem;
    pub use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
    pub use crate::adaptor::gateway::local_event_store::clock::FakeStoreClock;
    pub use crate::adaptor::gateway::local_event_store::envelope::EventCodecError;
    pub use crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry;
    pub use crate::adaptor::gateway::local_event_store::envelope::LocalEventPayloadCodec;
    pub use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
    pub use crate::adaptor::gateway::local_event_store::fault::MaintenanceFaultPoint;
    pub use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
    pub use crate::adaptor::gateway::local_event_store::layout::DATABASE_FILE;
    pub use crate::adaptor::gateway::local_event_store::maintenance::cleanup_vacuum_artifacts;
    pub use crate::adaptor::gateway::local_event_store::maintenance::prepare_vacuum_database;
    pub use crate::adaptor::gateway::local_event_store::maintenance::read_freelist_stats;
    pub use crate::adaptor::gateway::local_event_store::maintenance::replace_canonical_database;
    pub use crate::adaptor::gateway::local_event_store::maintenance::should_reclaim;
    pub use crate::adaptor::gateway::local_event_store::maintenance::verify_owner_only_permissions;
    pub use crate::adaptor::gateway::local_event_store::maintenance::CanonicalReplacementFailure;
    pub use crate::adaptor::gateway::local_event_store::maintenance::MaintenanceFailure;

    pub use crate::adaptor::gateway::local_event_store::node_events::append_node_event;
    pub use crate::adaptor::gateway::local_event_store::node_events::delete_tree;
    pub use crate::adaptor::gateway::local_event_store::node_events::first_root_row_of_tree;
    pub use crate::adaptor::gateway::local_event_store::node_events::first_row_for_tree_with_event_types;
    pub use crate::adaptor::gateway::local_event_store::node_events::first_row_of_tree;
    pub use crate::adaptor::gateway::local_event_store::node_events::latest_root_rows_for_trees;
    pub use crate::adaptor::gateway::local_event_store::node_events::latest_row_for_node_with_event_types;
    pub use crate::adaptor::gateway::local_event_store::node_events::list_tree_roots;
    pub use crate::adaptor::gateway::local_event_store::node_events::read_tree;
    pub use crate::adaptor::gateway::local_event_store::node_events::read_tree_after;
    pub use crate::adaptor::gateway::local_event_store::node_events::rows_for_event_types;
    pub use crate::adaptor::gateway::local_event_store::node_events::tree_heads;
    pub use crate::adaptor::gateway::local_event_store::node_events::NewNodeEventRow;
    pub use crate::adaptor::gateway::local_event_store::node_events::NodeEventRow;
    pub use crate::adaptor::gateway::local_event_store::read_only::validate_reader_snapshot;
    pub use crate::adaptor::gateway::local_event_store::read_only::DatabaseFileIdentity;
    pub use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
    pub use crate::adaptor::gateway::local_event_store::read_only::STORE_NOT_READY;
    pub use crate::adaptor::gateway::local_event_store::reader::canonical_runtime_owner_snapshot;
    pub use crate::adaptor::gateway::local_event_store::reader::storage_unavailable;
    pub use crate::adaptor::gateway::local_event_store::reader::ReaderPool;
    pub use crate::adaptor::gateway::local_event_store::reader::MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT;
    pub use crate::adaptor::gateway::local_event_store::reader::READER_POOL_SIZE;
    pub use crate::adaptor::gateway::local_event_store::reader::READ_QUEUE_MAX_DEPTH;
    pub use crate::adaptor::gateway::local_event_store::schema::create_store_metadata;
    pub use crate::adaptor::gateway::local_event_store::schema::evolve_schema;
    pub use crate::adaptor::gateway::local_event_store::schema::initialize_schema;
    pub use crate::adaptor::gateway::local_event_store::schema::require_foreign_key_integrity;
    pub use crate::adaptor::gateway::local_event_store::schema::require_index;
    pub use crate::adaptor::gateway::local_event_store::schema::require_schema_object_absent;
    pub use crate::adaptor::gateway::local_event_store::schema::table_columns;
    pub use crate::adaptor::gateway::local_event_store::schema::validate_current_schema;
    pub use crate::adaptor::gateway::local_event_store::schema::InitialStoreMetadata;
    pub use crate::adaptor::gateway::local_event_store::schema::APPLICATION_ID;
    pub use crate::adaptor::gateway::local_event_store::schema::CURRENT_SCHEMA_VERSION;

    pub use crate::adaptor::gateway::local_event_store::store::classify_existing_database;

    pub use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
    pub use crate::adaptor::gateway::local_event_store::store::LocalEventStoreConfig;
    pub use crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError;

    pub use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    pub use crate::adaptor::gateway::local_event_store::writer::PreparedNodeEvent;
    pub use crate::adaptor::gateway::local_event_store::writer::WriteJob;
    pub use crate::adaptor::gateway::local_event_store::writer::CRITICAL_LANE_MAX_BYTES;
    pub use crate::adaptor::gateway::local_event_store::writer::CRITICAL_LANE_MAX_REQUESTS;
    pub use crate::adaptor::gateway::local_event_store::writer::MAX_BATCH_DECODED_BYTES;
    pub use crate::adaptor::gateway::local_event_store::writer::MAX_BATCH_EVENTS;
    pub use crate::adaptor::gateway::local_event_store::writer::MAX_BATCH_STATE_MUTATIONS;
    pub use crate::adaptor::gateway::local_event_store::writer::NORMAL_LANE_MAX_BYTES;
    pub use crate::adaptor::gateway::local_event_store::writer::NORMAL_LANE_MAX_REQUESTS;
    pub use crate::domain::app_data_gc::GcCategory;
    pub use crate::usecase::app_data_gc::archive_removed_execution_trees;
    pub use crate::usecase::app_data_gc::run_startup_gc;
    pub use crate::usecase::app_data_gc::CacheGcRecord;
    pub use crate::usecase::app_data_gc::CanonicalRuntimeOwners;
    pub use crate::usecase::app_data_gc::ExecutionTreeGc;
    pub use crate::usecase::app_data_gc::LiveWorktree;
    pub use crate::usecase::app_data_gc::LiveWorktreeResolution;
    pub use crate::usecase::app_data_gc::LiveWorktreeSet;
}
pub mod platform {
    pub use crate::acceptance_test_support::{workflow_read, workflow_services};
    pub use crate::adaptor::controller::agent_session_launch_retention::run;
    pub use crate::adaptor::controller::agent_session_launch_retention::RETENTION;
    pub use crate::adaptor::controller::agent_session_wiring::compose_agent_sessions;
    pub use crate::adaptor::controller::agent_session_wiring::AgentSessionCompositionInput;
    pub use crate::adaptor::controller::app_data_composition::ProductionAppDataComposition;
    pub use crate::adaptor::controller::repository_scan::start;
    pub use crate::adaptor::controller::repository_scan::RepositoryScanWorkerRuntime;
    pub use crate::adaptor::controller::repository_scan::DEBOUNCE;
    pub use crate::adaptor::controller::state::AppState;
    pub use crate::adaptor::controller::terminal_surface_runtime::initialize_background_work_for_acceptance;
    pub use crate::adaptor::controller::terminal_surface_runtime::TerminalSurfaceRuntime;
    pub use crate::adaptor::controller::wiring::build_code_usecase;
    pub use crate::adaptor::controller::wiring::build_git_host_usecase;
    pub use crate::adaptor::controller::wiring::build_repository_usecase;
    pub use crate::adaptor::controller::wiring::build_review_comment_usecase;
    pub use crate::adaptor::controller::wiring::build_terminal_surface_application_for_tests;
    pub use crate::adaptor::controller::wiring::build_workflow_runtime_usecase;
    pub use crate::adaptor::controller::wiring::build_workflow_services_with_gateways;
    pub use crate::adaptor::controller::wiring::build_workflow_services_with_repository_worktrees;
    pub use crate::adaptor::controller::wiring::build_workflow_usecase;
    pub use crate::adaptor::controller::wiring::build_workflow_usecase_and_store;
    pub use crate::adaptor::controller::wiring::build_workspace_list_usecase;
    pub use crate::adaptor::controller::wiring::wire_delegate_continuation;
    pub use crate::adaptor::controller::wiring::wire_workflow_startup;
    pub use crate::adaptor::controller::workflow_startup::recover;
    pub use crate::adaptor::gateway::application_lifecycle::DaemonShutdownGateway;
    pub use crate::adaptor::gateway::comment::acquire_worktree_file_lock;
    pub use crate::adaptor::gateway::comment::lock_file;
    pub use crate::adaptor::gateway::comment::state_dir;
    pub use crate::adaptor::gateway::comment::state_file;
    pub use crate::adaptor::gateway::comment::worktree_storage_key;
    pub use crate::adaptor::gateway::comment::FileReviewEventStore;
    pub use crate::adaptor::gateway::comment::SystemReviewClock;
    pub use crate::adaptor::gateway::comment::UuidReviewIdGenerator;
    pub use crate::adaptor::gateway::external_editor::launcher_impl::open_path_with;
    pub use crate::adaptor::gateway::external_editor::scanner_impl::MacInstalledEditorGateway;
    pub use crate::adaptor::gateway::external_editor::settings_gateway_impl::EditorSettingsConfigGateway;
    pub use crate::adaptor::gateway::failure_records::FailureRecordStore;
    pub use crate::adaptor::gateway::git_host::cache::InMemoryTtlCache;
    pub use crate::adaptor::gateway::git_host::cache::LatestPrStatuses;
    pub use crate::adaptor::gateway::git_host::discovery::get_origin_url;
    pub use crate::adaptor::gateway::git_host::discovery::is_github_repository;
    pub use crate::adaptor::gateway::git_host::github::GhCommandOutput;
    pub use crate::adaptor::gateway::git_host::github::GhCommandRunner;
    pub use crate::adaptor::gateway::git_host::github::GitHubGitHostGateway;
    pub use crate::adaptor::gateway::git_host::github::SystemGhCommandRunner;
    pub use crate::adaptor::gateway::git_host::github::GH_TIMEOUT;
    pub use crate::adaptor::gateway::identity::RandomIdentityIssuer;
    pub use crate::adaptor::gateway::notion::service_impl::send;
    pub use crate::adaptor::gateway::notion::service_impl::send_with_retry;
    pub use crate::adaptor::gateway::notion::service_impl::NotionApiGatewayImpl;
    pub use crate::common::test_helpers::CancelAfter;
    pub use crate::domain::failure::StorageFailureSource;
    pub use crate::domain::notion::value_objects::NotionTask;
    pub use crate::infrastructure::local_api::LocalApiDiscovery;
    pub use crate::infrastructure::telemetry::metrics::resource::ProcessResourceObserver;
    pub use crate::usecase::repository_dto::BranchDto;
    pub use crate::usecase::repository_state::test_helpers::tests_support::IdentityWorktreePathNormalizer;
    pub use crate::usecase::repository_state::test_helpers::tests_support::TestRepositoryStateWorkerRuntime;

    pub use crate::adaptor::gateway::shared::background_worker::with_blocked_request;
    pub use crate::adaptor::gateway::shared::background_worker::BlockedRequest;
    pub use crate::adaptor::gateway::shared::git_operation::checkout;
    pub use crate::adaptor::gateway::shared::git_operation::detect_default_branch;
    pub use crate::adaptor::gateway::shared::git_operation::get_branch_name_for_repo;
    pub use crate::adaptor::gateway::shared::git_operation::run as git_operation_run;
    pub use crate::adaptor::gateway::shared::git_operation::GitOperationError;
    pub use crate::adaptor::gateway::state_subscription_reads::StateSubscriptionReads;
    pub use crate::adaptor::gateway::workspace_state::repository_impl::state_file as repository_impl_state_file;
    pub use crate::adaptor::gateway::workspace_state::repository_impl::storage_key;
    pub use crate::adaptor::gateway::workspace_state::repository_impl::WorkspaceStateStore;

    pub use crate::adaptor::presenter::state_subscription_wire::payload;
    pub use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;

    pub use crate::agent_session_tui_acceptance::drain_and_close_store;
    pub use crate::common::operation_context::current;
    pub use crate::common::operation_context::ingress;
    pub use crate::common::operation_context::scope;
    pub use crate::common::operation_context::Cancellation;
    pub use crate::common::operation_context::Deadline;
    pub use crate::common::operation_context::OperationContext;
    pub use crate::common::operation_context::OperationStopped;

    pub use crate::common::operation_context::spawn_blocking;
    pub use crate::common::operation_context::sync_scope;
    pub use crate::common::operation_context::wait;
    pub use crate::common::priority::PriorityGate;
    pub use crate::common::retry::AttemptProgress;
    pub use crate::common::retry::RetryLimiter;
    pub use crate::domain::comment::AuthorScope;
    pub use crate::domain::comment::ReviewActor;
    pub use crate::domain::comment::ReviewActorKind;
    pub use crate::domain::comment::ReviewError;
    pub use crate::domain::comment::ReviewEvent;
    pub use crate::domain::comment::ReviewHistoryEntry;
    pub use crate::domain::comment::ReviewTarget;
    pub use crate::domain::comment::ReviewThreadFilter;
    pub use crate::domain::comment::ReviewThreadState;
    pub use crate::domain::comment::MAX_REVIEW_TEXT_BYTES;
    pub use crate::domain::external_editor::gateway::EditorError;
    pub use crate::domain::external_editor::services::scan_applications_in;
    pub use crate::domain::failure::BusinessFailure;
    pub use crate::domain::failure::Failure;
    pub use crate::domain::failure::FailureKey;
    pub use crate::domain::failure::StorageFailure;
    pub use crate::domain::failure::TechnicalFailure;
    pub use crate::domain::failure::TechnicalFailureNature;
    pub use crate::domain::failure::WorkFailure;
    pub use crate::domain::git_host::git_host::GitHostError;
    pub use crate::domain::git_host::git_host::GitHostProvider;
    pub use crate::domain::git_host::value_objects::cache::CacheTtl;
    pub use crate::domain::git_host::value_objects::issue::IssueInfo;
    pub use crate::domain::git_host::value_objects::issue::PrAuthor;
    pub use crate::domain::git_host::value_objects::pr::PrInfo;
    pub use crate::domain::git_host::value_objects::pr::PrStatus;
    pub use crate::domain::local_event::batch::CommitBatchError;
    pub use crate::domain::local_event::batch::CommitBatchResult;
    pub use crate::domain::local_event::batch::CommitOperationKind;
    pub use crate::domain::local_event::batch::CommitResolution;
    pub use crate::domain::local_event::batch::IdempotencyBinding;
    pub use crate::domain::local_event::batch::LocalAtomicBatch;
    pub use crate::domain::local_event::events::DomainEventPage;
    pub use crate::domain::local_event::events::LoadStreamRequest;
    pub use crate::domain::local_event::events::LoadedDomainEvent;
    pub use crate::domain::local_event::events::LocalDomainEvent;
    pub use crate::domain::local_event::events::UncommittedDomainEvent;
    pub use crate::domain::local_event::failure::SessionOperationFailureKind;
    pub use crate::domain::local_event::identifiers::CommitIdentity;
    pub use crate::domain::local_event::identifiers::ExpectedStreamHead;
    pub use crate::domain::local_event::identifiers::StreamId;
    pub use crate::domain::local_event::identifiers::StreamVersion;
    pub use crate::domain::local_event::mutation::AgentSessionRemovalMutation;
    pub use crate::domain::local_event::mutation::LocalStateMutation;
    pub use crate::domain::local_event::query::CanonicalRuntimeOwnerView;
    pub use crate::domain::local_event::query::LocalEventQuery;
    pub use crate::domain::local_event::query::LocalEventQueryError;
    pub use crate::domain::local_event::query::LocalEventQueryResult;
    pub use crate::domain::notion::error::NotionError;
    pub use crate::domain::notion::gateway::NotionApiGateway;
    pub use crate::domain::notion::value_objects::NotionLabelOption;
    pub use crate::domain::notion::value_objects::NotionTaskPage;
    pub use crate::domain::notion::value_objects::NotionTaskQuery;
    pub use crate::domain::notion::value_objects::NotionValidationResult;
    pub use crate::domain::workspace_state::entities::workspace_state::WorkspaceState;
    pub use crate::domain::workspace_state::error::WorkspaceStateError;
    pub use crate::domain::workspace_state::services::filter_missing_files;
    pub use crate::domain::workspace_state::value_objects::workspace_layout_state::WorkspaceLayoutState;
    pub use crate::domain::workspace_state::value_objects::workspace_tabs_state::WorkspaceTabEntry;
    pub use crate::domain::workspace_state::value_objects::workspace_tabs_state::WorkspaceTabsState;
    pub use crate::infrastructure::app_data_path::AppDataPathObserver;
    pub use crate::infrastructure::app_data_path::AppDataPathOperation;
    pub use crate::infrastructure::app_data_path::NoopAppDataPathObserver;
    pub use crate::infrastructure::file_lock::exclusive;
    pub use crate::infrastructure::file_lock::LockError;
    pub use crate::infrastructure::file_watcher::FileWatcherManager;
    pub use crate::infrastructure::local_log::init_with_limits;
    pub use crate::infrastructure::local_log::LocalFileLogger;
    pub use crate::infrastructure::local_log::LocalLogProcess;
    pub use crate::infrastructure::local_log::LocalLogWriter;
    pub use crate::infrastructure::local_log::ACTIVE_FILE_NAME;
    pub use crate::infrastructure::local_log::LOCK_FILE_NAME;
    pub use crate::infrastructure::local_log::LOG_DIRECTORY_NAME;
    pub use crate::infrastructure::local_log::MAX_FILE_COUNT;
    pub use crate::infrastructure::lua::evaluator::evaluate;
    pub use crate::infrastructure::lua::evaluator::LuaData;
    pub use crate::infrastructure::lua::evaluator::LuaEvaluationRequest;
    pub use crate::infrastructure::lua::evaluator::LuaFailureKind;
    pub use crate::infrastructure::lua::evaluator::LuaHost;
    pub use crate::infrastructure::lua::evaluator::LuaHostError;
    pub use crate::infrastructure::lua::evaluator::LuaHostHandle;
    pub use crate::infrastructure::lua::evaluator::LuaLimits;
    pub use crate::infrastructure::lua::evaluator::LuaModule;
    pub use crate::infrastructure::lua::evaluator::LuaModuleValue;
    pub use crate::infrastructure::lua::evaluator::LuaSourceLocation;
    pub use crate::infrastructure::lua::evaluator::LuaTableData;
    pub use crate::infrastructure::lua::evaluator::LuaTableKey;
    pub use crate::infrastructure::lua::evaluator::MAX_TABLE_DEPTH;
    pub use crate::infrastructure::lua::evaluator::MAX_TABLE_ELEMENTS;
    pub use crate::infrastructure::platform::data_dir::default_data_dir_name_for_profile;
    pub use crate::infrastructure::platform::path_aliases::alias_name_for_profile;
    pub use crate::infrastructure::platform::path_aliases::child_env_overrides_from;
    pub use crate::infrastructure::platform::path_aliases::ensure_alias_wrapper;
    pub use crate::infrastructure::platform::path_aliases::prepare_child_env;
    pub use crate::infrastructure::platform::path_aliases::BuildProfile;
    pub use crate::infrastructure::platform::path_aliases::PathAlias;
    pub use crate::infrastructure::platform::path_aliases::PathAliases;
    pub use crate::infrastructure::terminal::checkpoint_journal::IncrementalCheckpointJournal;

    pub use crate::infrastructure::terminal::shell_integration::create_shell_integration_files;
    pub use crate::infrastructure::terminal::terminal_emulator::NativeTerminalCheckpoint;
    pub use crate::infrastructure::terminal::terminal_emulator::NativeTerminalCheckpointRecord;
    pub use crate::infrastructure::terminal::terminal_emulator::NativeTerminalEmulator;
    pub use crate::infrastructure::terminal::terminal_emulator::TerminalCheckpointFileStore;
    pub use crate::infrastructure::timer::delays;
    pub use crate::test_support::EnvVarGuard;
    pub use crate::test_support::TEST_ENV_LOCK;

    pub use crate::test_support::captured_warning_messages;
    pub use crate::test_support::install_capturing_logger;
    pub use crate::test_support::retry::record_retry_failure;
    pub use crate::test_support::retry::shared;
    pub use crate::test_support::retry::shared_limiter;
    pub use crate::test_support::retry::shared_store;
    pub use crate::test_support::retry::test_retrying;
    pub use crate::test_support::retry::test_retrying_with_store;
    pub use crate::usecase::application_lifecycle::test_helpers::FakeShutdown;
    pub use crate::usecase::application_lifecycle::test_helpers::STAGES;
    pub use crate::usecase::code_dto::ReviewFileViewDto;
    pub use crate::usecase::comment::ReviewClock;
    pub use crate::usecase::comment::ReviewEventMutation;
    pub use crate::usecase::comment::ReviewEventStore;
    pub use crate::usecase::comment::ReviewIdGenerator;
    pub use crate::usecase::comment::{
        ReviewCommentUsecase, ReviewContextUsecase, SessionReviewUsecase,
    };
    pub use crate::usecase::fetched::Fetched;
    pub use crate::usecase::git_host::git_host_usecase::GitHostUsecase;
    pub use crate::usecase::notion::error::NotionUsecaseError;
    pub use crate::usecase::notion::usecase::NotionTaskListRequest;
    pub use crate::usecase::notion::usecase::NotionUsecase;
    pub use crate::usecase::provider_dto::AgentSessionProviderDto;
    pub use crate::usecase::repo_paths_usecase::RepoPathsUsecase;
    pub use crate::usecase::repository_dto::FileDiffStatDto;
    pub use crate::usecase::repository_dto::FileStatusDto;

    pub use crate::usecase::repository_state::error::RepositoryStateError;

    pub use crate::usecase::repository_state::runtime::RepositoryStateWorkerRuntime;
    pub use crate::usecase::repository_state::runtime::WorktreePathNormalizer;
    pub use crate::usecase::repository_state::scanner::changes_diff_tree_entries;
    pub use crate::usecase::repository_state::scanner::diff_tree_entries;
    pub use crate::usecase::repository_state::scanner::staged_diff_tree_entries;
    pub use crate::usecase::repository_state::scanner::RepositoryScanner;
    pub use crate::usecase::repository_state::service::RepositoryStateRepository;
    pub use crate::usecase::repository_state::service::RepositoryStateService;
    pub use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
    pub use crate::usecase::repository_state::worker::InvalidateReason;
    pub use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;
    pub use crate::usecase::repository_state::worktree::RepositoryStateWatchSession;
    pub use crate::usecase::repository_state::worktree::RepositoryStateWatcher;
    pub use crate::usecase::repository_state::worktree::WorktreeState;
    pub use crate::usecase::repository_usecase::RepositoryUsecase;
    pub use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
    pub use crate::usecase::review_usecase::ReviewUsecase;
    pub use crate::usecase::test_helpers::watcher::SubscriptionFiles;
    pub use crate::usecase::watcher::UsecaseError as watcher_UsecaseError;
    pub use crate::usecase::watcher::WatcherUsecase;
    pub use crate::usecase::workspace_state::dto::WorkspaceStateDto;
    pub use crate::usecase::workspace_state::usecase::save_workspace_state;

    pub use crate::usecase::worktree_operation::WorktreeMutationGuard;
    pub use crate::usecase::worktree_operation::WorktreeOperations;
}
pub mod process {
    pub use crate::infrastructure::process::child_process::configure_process_group;
    #[cfg(target_os = "macos")]
    pub use crate::infrastructure::process::child_process::group_has_live_members;
    pub use crate::infrastructure::process::child_process::signal_process_group;
    pub use crate::infrastructure::process::child_process::wait_without_reaping;
    pub use crate::infrastructure::process::command_runner::display_cwd;
    pub use crate::infrastructure::process::command_runner::spawn_shell_command;
    pub use crate::infrastructure::process::command_runner::ActiveCommandHandle;
    pub use crate::infrastructure::process::command_runner::CommandRunOutput;
    pub use crate::infrastructure::process::command_runner::CommandRunnerError;
    pub use crate::infrastructure::process::command_runner::OutputLimit;
    pub use crate::infrastructure::process::executable_probe::resolve_executable;
    pub use crate::infrastructure::process::executable_probe::ExecutableProbeResult;
    pub use crate::infrastructure::process::fd_limit::current_limit;
    pub use crate::infrastructure::process::fd_limit::raise_open_file_limit;
    pub use crate::infrastructure::process::fd_limit::target_soft_limit;
    pub use crate::infrastructure::process::output::output;
    pub use crate::infrastructure::process::output::ProcessError;
    pub use crate::infrastructure::process::search_path::capture_login_shell_path_from;
    pub use crate::infrastructure::process::search_path::LoginShellPathError;
    pub use crate::infrastructure::process::search_path::SearchPathSource;
}
pub mod providers {
    pub use crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter;
    pub use crate::usecase::provider_lifecycle::ingress::ProviderPayloadInput;

    pub use crate::acceptance_test_support::write_hook_failure;
    pub use crate::adaptor::gateway::provider_lifecycle::credential_gateway_impl::LocalProviderLifecycleCredentialGateway;
    pub use crate::adaptor::gateway::provider_lifecycle::event_repository_impl::LocalProviderLifecycleEventRepository;
    pub use crate::adaptor::gateway::provider_lifecycle::hook_health_failure_query_impl::LocalProviderHookHealthFailureQuery;
    pub use crate::adaptor::gateway::provider_lifecycle::hook_health_repository_impl::LocalProviderHookHealthRepository;
    pub use crate::adaptor::gateway::provider_lifecycle::launch_spec::ProviderLaunchContext;
    pub use crate::adaptor::gateway::provider_lifecycle::launch_spec::ProviderLaunchSpec;
    pub use crate::domain::provider_lifecycle::entities::provider_hook_health::ProviderHookHealth;
    pub use crate::domain::provider_lifecycle::entities::provider_hook_health::ProviderHookHealthOutcome;
    pub use crate::domain::provider_lifecycle::entities::provider_lifecycle_binding::ProviderLifecycleBinding;
    pub use crate::domain::provider_lifecycle::repository::ProviderHookHealthRepository;
    pub use crate::domain::provider_lifecycle::repository::ProviderHookHealthRepositoryError;
    pub use crate::domain::provider_lifecycle::repository::ProviderLifecycleEventRepository;
    pub use crate::domain::provider_lifecycle::repository::ProviderLifecycleRepositoryError;
    pub use crate::domain::provider_lifecycle::repository::VersionedProviderHookHealth;
    pub use crate::domain::provider_lifecycle::value_objects::armed_provider_lifecycle::ArmedProviderLifecycle;
    pub use crate::domain::provider_lifecycle::value_objects::provider_kind::ProviderKind;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_event::ProviderLifecycleEvent;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_ingress_result::ProviderLifecycleIngressResult;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_outcome::ProviderLifecycleRejection;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_scope::ProviderLifecycleScope;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_signal::ProviderLifecycleSignal;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_signal::ProviderLifecycleSignalKind;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_slot_id::ProviderLifecycleSlotId;
    pub use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_unavailable::ProviderLifecycleUnavailableReason;
    pub use crate::domain::provider_lifecycle::value_objects::scoped_provider_lifecycle_event::ScopedProviderLifecycleEvent;
    pub use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthFailureObservation;
    pub use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthFailureQuery;
    pub use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthFailureQueryError;
    pub use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthReadUsecase;
    pub use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthUsecase;
    pub use crate::usecase::provider_lifecycle::ingress::ProviderExecutionTreeStopCommand;
    pub use crate::usecase::provider_lifecycle::ingress::ProviderExecutionTreeStopTransaction;
    pub use crate::usecase::provider_lifecycle::ingress::ProviderLifecycleIngressUsecase;
    pub use crate::usecase::provider_lifecycle::ingress::ProviderLifecycleIngressUsecaseError;
    pub use crate::usecase::provider_lifecycle::ingress::ProviderSessionStartTransaction;
    pub use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;
}
pub mod repository {

    pub use crate::adaptor::gateway::repository::branch::get_current_branch;
    pub use crate::adaptor::gateway::repository::branch::git_create_branch;
    pub use crate::adaptor::gateway::repository::branch::list_branches;
    pub use crate::adaptor::gateway::repository::file_watcher::FileWatcherGateway;
    pub use crate::adaptor::gateway::repository::git_config::get_branch_base;
    pub use crate::adaptor::gateway::repository::git_config::get_releash_base;
    pub use crate::adaptor::gateway::repository::git_config::prune_stale_branch_bases;
    pub use crate::adaptor::gateway::repository::git_config::resolve_base_commit_oid;
    pub use crate::adaptor::gateway::repository::git_config::resolve_current_base_branch;
    pub use crate::adaptor::gateway::repository::git_config::resolve_effective_base_branch;
    pub use crate::adaptor::gateway::repository::git_config::set_branch_base_override;
    pub use crate::adaptor::gateway::repository::git_config::set_releash_base;
    pub use crate::adaptor::gateway::repository::repo_paths::RepoPathsGateway;
    pub use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
    pub use crate::adaptor::gateway::repository::scanner::DefaultRepositoryScanner;
    pub use crate::adaptor::gateway::repository::state::handle_file_events;
    pub use crate::adaptor::gateway::repository::state::handle_git_events;
    pub use crate::adaptor::gateway::repository::state::FsWorktreePathNormalizer;
    pub use crate::adaptor::gateway::repository::state::NotifyRepositoryStateWatcher;
    pub use crate::adaptor::gateway::repository::state::RepositoryStateRepositoryGateway;
    pub use crate::adaptor::gateway::repository::status::get_git_status;
    pub use crate::adaptor::gateway::repository::status::get_repository_status_scan;
    pub use crate::adaptor::gateway::repository::status::get_status_diff_stats;
    pub use crate::adaptor::gateway::repository::status::reset_status_walk_count_for_tests;
    pub use crate::adaptor::gateway::repository::status::status_walk_count_for_tests;
    pub use crate::adaptor::gateway::repository::util::resolve_branch_base;
    pub use crate::adaptor::gateway::repository::watch::resolve_file_watch_paths;
    pub use crate::adaptor::gateway::repository::watch::resolve_worktree_git_dir;
    pub use crate::adaptor::gateway::repository::worktree::create_worktree;
    pub use crate::adaptor::gateway::repository::worktree::each_worktree;
    pub use crate::adaptor::gateway::repository::worktree::find_main_repo_path;
    pub use crate::adaptor::gateway::repository::worktree::get_main_repo_path;
    pub use crate::adaptor::gateway::repository::worktree::get_worktree_dirty_count;
    pub use crate::adaptor::gateway::repository::worktree::list_worktrees;
    pub use crate::adaptor::gateway::repository::worktree::path_to_worktree_identity;
    pub use crate::adaptor::gateway::repository::worktree::prune_invalid_worktrees;
    pub use crate::adaptor::gateway::repository::worktree::recorded_main_repo_path;
    pub use crate::adaptor::gateway::repository::worktree::registered_worktree_paths;
    pub use crate::adaptor::gateway::repository::worktree::remove_worktree;
    pub use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
    pub use crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks;
    pub use crate::domain::code::repository::FileContentRepository;
    pub use crate::domain::code::repository::StagingRepository;
    pub use crate::domain::repository::worktree_operation::WorktreeOperationLocks;
    pub use crate::usecase::test_helpers::usecase as repository_usecase;
    pub use crate::usecase::test_helpers::FakeRepo;

    pub use crate::domain::app_config::repository::ConfigRepository;
    pub use crate::domain::app_config::repository::ConfigSecretRepository;
    pub use crate::domain::app_config::repository::ConfigUpdate;
    pub use crate::domain::app_config::repository::NotionConfigRepository;
    pub use crate::domain::code::repository::ReviewSideBytes;
    pub use crate::domain::code::repository::ReviewSideMetadata;
    pub use crate::domain::local_event::repository::LocalEventTransactionRepository;

    pub use crate::domain::repository::entities::worktree::Worktree;
    pub use crate::domain::repository::error::RepositoryError;
    pub use crate::domain::repository::file_watcher::FileWatchGateway;

    pub use crate::domain::repository::repository::RepoPathsRepository;

    pub use crate::domain::repository::worktree_operation::WorktreeDeletionTarget;
    pub use crate::domain::workspace_state::repository::WorkspaceStateRepository;
}
pub mod review {
    pub use crate::domain::code::value_objects::review::ReviewBase;
    pub use crate::domain::code::value_objects::review::ReviewSection;
}
pub mod sessions {
    pub use crate::usecase::agent_session::test_helpers::MemoryHookHealthRepository;

    pub use crate::usecase::agent_session::test_helpers::FixedHistory;

    pub use crate::usecase::agent_session::test_helpers::BlockingLaunchTerminal;
    pub use crate::usecase::agent_session::test_helpers::RecordingTerminal;

    pub use crate::usecase::agent_session::test_helpers::hook_health_usecase;

    pub use crate::usecase::agent_session::test_helpers::FailingFirstLifecycleEvents;
    pub use crate::usecase::agent_session::test_helpers::RecordingLaunchGateway;
    pub use crate::usecase::agent_session::test_helpers::RecordingLifecycleEvents;

    pub use crate::adaptor::gateway::agent_session::agent_session_history_gateway::LocalAgentSessionHistoryGateway;
    pub use crate::adaptor::gateway::agent_session::agent_session_history_query_service::LocalAgentSessionHistoryQueryService;
    pub use crate::adaptor::gateway::agent_session::agent_session_query_service::LocalAgentSessionQueryService;
    pub use crate::adaptor::gateway::agent_session::agent_session_repository::open_session_title_candidates;
    pub use crate::adaptor::gateway::agent_session::agent_session_repository::LocalAgentSessionRepository;
    pub use crate::adaptor::gateway::agent_session::agent_session_repository::OPEN_SESSION_LIFECYCLE_EVENT_TYPES;
    pub use crate::adaptor::gateway::agent_session::provider_agent_launch_gateway::digest;
    pub use crate::adaptor::gateway::agent_session::provider_agent_launch_gateway::LocalProviderAgentLaunchGateway;
    pub use crate::adaptor::gateway::agent_session::provider_availability_gateway::LocalProviderExecutableProbeGateway;
    pub use crate::adaptor::gateway::agent_session::session_facts::locate_session;
    pub use crate::adaptor::gateway::agent_session::session_facts::read_session_context;
    pub use crate::adaptor::gateway::agent_session::session_facts::read_session_records;
    pub use crate::adaptor::gateway::agent_session::session_facts::SessionContextReadError;
    pub use crate::adaptor::gateway::agent_session::session_facts::SessionLocation;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSession;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionArchiveOutcome;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionInitialInstructionOutcome;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionLifecycle;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionMutationOutcome;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionProcessExitOutcome;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionRecoveryResult;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionRemovalAuthorization;
    pub use crate::domain::agent_session::aggregates::agent_session::AgentSessionTreeLocation;
    pub use crate::domain::agent_session::aggregates::agent_session::ManagedPtyPresence;
    pub use crate::domain::agent_session::aggregates::provider_registry::ProviderExecutable;
    pub use crate::domain::agent_session::aggregates::provider_registry::ProviderUnavailableReason;
    pub use crate::domain::agent_session::aggregates::provider_registry::ResolvedProviderExecutable;
    pub use crate::domain::agent_session::launch_identity::launch_resource_id;
    pub use crate::domain::agent_session::provider_availability_gateway::ProviderAvailabilityReader;
    pub use crate::domain::agent_session::provider_availability_gateway::ProviderExecutableConfigRepository;
    pub use crate::domain::agent_session::provider_availability_gateway::ProviderExecutableConfigRepositoryError;
    pub use crate::domain::agent_session::provider_availability_gateway::ProviderExecutableProbeGateway;
    pub use crate::domain::agent_session::provider_availability_gateway::ProviderExecutableProbeGatewayError;
    pub use crate::domain::agent_session::provider_history_gateway::AgentSessionHistoryGateway;
    pub use crate::domain::agent_session::provider_history_gateway::AgentSessionHistoryGatewayError;
    pub use crate::domain::agent_session::provider_history_gateway::AgentSessionHistoryMetadata;
    pub use crate::domain::agent_session::provider_history_gateway::AgentSessionOwnershipQuery;
    pub use crate::domain::agent_session::provider_history_gateway::ProviderSessionTitleEntry;
    pub use crate::domain::agent_session::provider_launch::ProviderLaunchOptions;
    pub use crate::domain::agent_session::provider_launch::ProviderSessionLaunch;
    pub use crate::domain::agent_session::provider_launch_gateway::PreparedProviderLaunch;
    pub use crate::domain::agent_session::provider_launch_gateway::ProviderAgentLaunchGateway;
    pub use crate::domain::agent_session::provider_launch_gateway::ProviderAgentLaunchGatewayError;
    pub use crate::domain::agent_session::provider_session_title_gateway::ProviderSessionTitleGateway;
    pub use crate::domain::agent_session::provider_session_title_gateway::ProviderSessionTitleGatewayError;
    pub use crate::domain::agent_session::provider_session_title_gateway::ProviderSessionTitleRequest;
    pub use crate::domain::agent_session::provider_terminal_gateway::ProviderAgentTerminalGateway;
    pub use crate::domain::agent_session::provider_terminal_gateway::ProviderAgentTerminalGatewayError;
    pub use crate::domain::agent_session::provider_terminal_gateway::ProviderAgentTerminalInputGateway;
    pub use crate::domain::agent_session::repository::AgentSessionRepository;
    pub use crate::domain::agent_session::repository::AgentSessionRepositoryError;
    pub use crate::domain::agent_session::repository::VersionedAgentSession;
    pub use crate::usecase::agent_session::agent_session_history::AgentSessionHistoryCandidateDto;
    pub use crate::usecase::agent_session::agent_session_history::AgentSessionHistoryPageDto;
    pub use crate::usecase::agent_session::agent_session_history::AgentSessionHistoryQueryError;
    pub use crate::usecase::agent_session::agent_session_history::AgentSessionHistoryQueryService;
    pub use crate::usecase::agent_session::agent_session_history::AgentSessionHistoryReadUsecase;
    pub use crate::usecase::agent_session::agent_session_history::AgentSessionHistoryRequest;
    pub use crate::usecase::agent_session::agent_session_initial_instruction::AgentSessionInitialInstructionDeliveryOutcome;
    pub use crate::usecase::agent_session::agent_session_initial_instruction::AgentSessionInitialInstructionError;
    pub use crate::usecase::agent_session::agent_session_initial_instruction::AgentSessionInitialInstructionUsecase;
    pub use crate::usecase::agent_session::agent_session_launch::AgentSessionExecutionTreeLifecycle;
    pub use crate::usecase::agent_session::agent_session_launch::AgentSessionHistoryResumeOutcome;
    pub use crate::usecase::agent_session::agent_session_launch::AgentSessionHistoryResumeRequest;
    pub use crate::usecase::agent_session::test_helpers::captured_terminal_spawn_failure;
    pub use crate::usecase::agent_session::test_helpers::provider_runtime;
    pub use crate::usecase::agent_session::test_helpers::session_location;
    pub use crate::usecase::agent_session::test_helpers::started_execution_trees;
    pub use crate::usecase::agent_session::test_helpers::workflow_location;

    pub use crate::usecase::agent_session::test_helpers::FixedAvailability;
    pub use crate::usecase::agent_session::test_helpers::RecordingStartedExecutionTrees;

    pub use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchRequest;
    pub use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchUsecase;
    pub use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchUsecaseError;
    pub use crate::usecase::agent_session::agent_session_launch::ExecutionTreeCache;
    pub use crate::usecase::agent_session::agent_session_launch::ExecutionTreeCacheReleaseError;
    pub use crate::usecase::agent_session::agent_session_launch::ProviderAgentRuntime;
    pub use crate::usecase::agent_session::agent_session_launch::StartedExecutionTreeRegistrar;
    pub use crate::usecase::agent_session::agent_session_launch::StartedExecutionTreeRegistrationError;
    pub use crate::usecase::agent_session::agent_session_launch::WorkflowAgentSessionLaunchRequest;
    pub use crate::usecase::agent_session::agent_session_launch::WorktreeMutationAdmission;
    pub use crate::usecase::agent_session::agent_session_lifecycle::AgentSessionGarbageCollectionOutcome;
    pub use crate::usecase::agent_session::agent_session_lifecycle::AgentSessionLifecycleUsecase;
    pub use crate::usecase::agent_session::agent_session_lifecycle::AgentSessionLifecycleUsecaseError;
    pub use crate::usecase::agent_session::agent_session_lifecycle::AgentSessionOpenOutcome;
    pub use crate::usecase::agent_session::agent_session_query::AgentSessionItemDto;
    pub use crate::usecase::agent_session::agent_session_query::AgentSessionLifecycleDto;
    pub use crate::usecase::agent_session::agent_session_query::AgentSessionOperationsDto;
    pub use crate::usecase::agent_session::agent_session_query::AgentSessionQueryError;
    pub use crate::usecase::agent_session::agent_session_query::AgentSessionQueryService;
    pub use crate::usecase::agent_session::agent_session_query::AgentSessionTreeLocationDto;
    pub use crate::usecase::agent_session::agent_session_read::AgentSessionGarbageCollectionPort;
    pub use crate::usecase::agent_session::agent_session_read::AgentSessionReadUsecase;
    pub use crate::usecase::agent_session::provider_availability::ProviderAvailabilityUsecase;
    pub use crate::usecase::agent_session::test_helpers::FakeProviderExecutableConfigRepository;
    pub use crate::usecase::agent_session::test_helpers::FakeProviderExecutableProbeGateway;
    pub use crate::usecase::agent_session::usecase::AgentSessionUsecase;
    pub use crate::usecase::agent_session::usecase::AgentSessionUsecaseError;
}
pub mod settings {
    pub use crate::adaptor::controller::client::app_config::commands::update_workflow_config_shared;
    pub use crate::adaptor::controller::client::app_config::shared::register_shared;
    pub use crate::adaptor::gateway::app_config::config_models::config_to_domain;
    pub use crate::adaptor::gateway::app_config::config_models::NotionPropertyMappingModel;
    pub use crate::adaptor::gateway::app_config::config_models::NotionRepoConfigModel;
    pub use crate::adaptor::gateway::app_config::config_models::ReleashConfig;
    pub use crate::adaptor::gateway::app_config::repository_impl::load_or_create_config;
    pub use crate::adaptor::gateway::app_config::repository_impl::read_config_if_exists;
    pub use crate::adaptor::gateway::app_config::repository_impl::write_config;
    pub use crate::adaptor::gateway::app_config::repository_impl::write_config_tmp_file;
    pub use crate::adaptor::gateway::app_config::repository_impl::AppConfig;
    pub use crate::domain::app_config::error::AppConfigError;
    pub use crate::domain::app_config::value_objects::AppConfigDocument;
    pub use crate::domain::app_config::value_objects::NotionLabelProperty;
    pub use crate::domain::app_config::value_objects::NotionPropertyMapping;
    pub use crate::domain::app_config::value_objects::NotionRepoConfig;
    pub use crate::usecase::app_config::usecase::AppConfigUsecase;
    pub use crate::usecase::app_config::usecase::WorkflowConfigInput;
}
pub mod subscriptions {
    pub use crate::test_support::state_subscription::test_subscriptions;
    pub use crate::test_support::state_subscription::CapturingNotifier;

    pub use crate::adaptor::presenter::state_subscription::PublishedState;
    pub use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
    pub use crate::infrastructure::state_subscription::Delivery;
    pub use crate::infrastructure::state_subscription::Event;
    pub use crate::test_support::state_subscription::changes;
    pub use crate::test_support::state_subscription::deps;
    pub use crate::test_support::state_subscription::pending_read_driver;
    pub use crate::test_support::state_subscription::read_driver;
    pub use crate::test_support::state_subscription::registration;
    pub use crate::test_support::state_subscription::repository_driver;
    pub use crate::test_support::state_subscription::same;
    pub use crate::test_support::state_subscription::scan_driver;
    pub use crate::test_support::state_subscription::take_changes;

    pub use crate::usecase::state_subscription::reads::StateReadError;
    pub use crate::usecase::state_subscription::reads::StateReadFailure;
    pub use crate::usecase::state_subscription::reads::StateSubscriptionRead;
    pub use crate::usecase::state_subscription::reads::WorkspaceStateReads;
    pub use crate::usecase::state_subscription::target::StateChangeSource;
    pub use crate::usecase::state_subscription::target::SubscriptionTarget;
    pub use crate::usecase::state_subscription::target::WatchRequirement;
    pub use crate::usecase::state_subscription::value::StateValue;
    pub use crate::usecase::state_subscription::StateSubscriptionOutput;
    pub use crate::usecase::state_subscription::StateSubscriptionUsecase;
    pub use crate::usecase::test_helpers::state_subscription::notion_target;
    pub use crate::usecase::test_helpers::state_subscription::FakeDelivery;
    pub use crate::usecase::test_helpers::state_subscription::RecordingOutput;
}
pub mod telemetry {
    pub use crate::adaptor::gateway::telemetry::TelemetryGateway;
    pub use crate::infrastructure::telemetry::crash::init_crash_reporting;
    pub use crate::infrastructure::telemetry::crash::report_error;
    pub use crate::infrastructure::telemetry::metrics::first_repo_snapshot_recorded_for_tests;
    pub use crate::infrastructure::telemetry::metrics::lock_test_telemetry;
    pub use crate::infrastructure::telemetry::metrics::record_first_repo_snapshot_ready;
    pub use crate::infrastructure::telemetry::metrics::reset_test_metrics;
    pub use crate::infrastructure::telemetry::metrics::set_performance_configured;
    pub use crate::infrastructure::telemetry::metrics::set_performance_enabled;
    pub use crate::infrastructure::telemetry::metrics::set_startup_origin;
    pub use crate::infrastructure::telemetry::metrics::test_metric_records;
}
pub mod terminal {
    pub use crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway;
    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::test_support::attach_checkpoint;
    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::test_support::compact_runtime_checkpoint;

    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::test_support::remove_checkpoint_target;
    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::test_support::replace_checkpoint_flush;
    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::test_support::BackgroundCheckpointFixture;

    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor;
    pub use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::CHECKPOINT_JOURNAL_COMPACTION_BYTES;
    pub use crate::domain::terminal_surface::entities::terminal_surface::TerminalSurface;
    pub use crate::domain::terminal_surface::gateway::TerminalRuntimeSpawnRequest;
    pub use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
    pub use crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError;
    pub use crate::domain::terminal_surface::value_objects::terminal_process_launch::TerminalProcessLaunch;
    pub use crate::domain::terminal_surface::value_objects::terminal_surface_checkpoint::TerminalSurfaceCheckpoint;
    pub use crate::domain::terminal_surface::value_objects::terminal_surface_checkpoint::TERMINAL_SURFACE_SCROLLBACK_ROWS;
    pub use crate::domain::terminal_surface::value_objects::terminal_surface_owner::TerminalSurfaceOwner;
    pub use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
    pub use crate::usecase::terminal_surface::lifecycle_usecase::kill_runtime_generation;
    pub use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
    pub use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
    pub use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;
    pub use crate::usecase::terminal_surface::spawn_usecase::get_or_spawn;
    pub use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;
}
pub mod transport {

    pub use crate::acceptance_test_support::build_client_dependencies;
    pub use crate::adaptor::controller::api::auth::{require_client, ClientTokens};
    pub use crate::adaptor::controller::api::build_router;
    pub use crate::adaptor::controller::api::client::ClientApiDeps;

    pub use crate::adaptor::controller::api::client::router;

    pub use crate::adaptor::controller::client::dependencies::ClientDependencies;
    pub use crate::adaptor::controller::client::dispatch::invalid_request;
    pub use crate::adaptor::controller::client::dispatch::required;
    pub use crate::adaptor::controller::client::dispatch::ClientCommandDispatch;
    pub use crate::adaptor::controller::client::worktree_mutation::admit;

    pub use crate::adaptor::gateway::notion::service_impl::build_client;
    pub use crate::adaptor::presenter::client::from_value;
    pub use crate::adaptor::presenter::connect::classified_error;

    pub use crate::adaptor::presenter::connect::ConnectFailure;
    pub use crate::adaptor::presenter::connect_wire::rpc;
    pub use crate::adaptor::presenter::connect_wire::to_rpc;
    pub use crate::adaptor::presenter::connect_wire::to_wire;
    pub use crate::client_api_acceptance::connect_client;
    pub use crate::client_api_acceptance::request_client;
    pub use crate::client_api_acceptance::ClientEndpoint;
    pub use crate::infrastructure::local_api::client_token::BearerToken;
    pub use crate::infrastructure::local_api::discovery::lookup_process_start_time;
    pub use crate::infrastructure::local_api::discovery::process_start_time;
    pub use crate::infrastructure::local_api::discovery::LocalApiDiscovery;
    pub use crate::infrastructure::local_api::discovery::LocalApiDiscoveryFile;
    pub use crate::infrastructure::local_api::discovery::ProcessStartTimeLookup;
    pub use crate::infrastructure::local_api::server::test_binding;
    pub use crate::infrastructure::local_api::server::LocalApiServer;
    pub use crate::infrastructure::local_api::LocalApiServerError;

    pub use crate::infrastructure::local_event_store_connection::configure_busy_handler;
    pub use crate::infrastructure::local_event_store_connection::open_existing_writer;
    pub use crate::infrastructure::local_event_store_connection::open_reader;
    pub use crate::infrastructure::local_event_store_connection::open_writer;

    pub use crate::test_support::client_api_deps;
}
pub mod workflow {
    pub use crate::adaptor::controller::client::workflow::delete_facet_inner;
    pub use crate::adaptor::controller::client::workflow::duplicate_facet_inner;
    pub use crate::adaptor::controller::client::workflow::get_facet_inner;
    pub use crate::adaptor::controller::client::workflow::list_facet_summaries_inner;
    pub use crate::adaptor::controller::client::workflow::list_facets_inner;
    pub use crate::adaptor::controller::client::workflow::open_facet_in_editor_inner;
    pub use crate::adaptor::controller::client::workflow::save_facet_inner;
    pub use crate::adaptor::controller::client::workflow::shared::register_shared;
    pub use crate::adaptor::gateway::workflow::builtin::is_builtin_facet;
    pub use crate::adaptor::gateway::workflow::builtin::is_builtin_workflow;
    pub use crate::adaptor::gateway::workflow::stored_definition::decode_started;
    pub use crate::adaptor::gateway::workflow::test_helpers::definition;
    pub use crate::adaptor::gateway::workflow::test_helpers::node_started;
    pub use crate::adaptor::gateway::workflow::test_helpers::started_event;
    pub use crate::adaptor::gateway::workflow::test_helpers::TREE;
    pub use crate::adaptor::gateway::workflow::workflow_host::isolated_worktree::NodePreparation;
    pub use crate::domain::workflow::entities::workflow_execution::LeafKind;
    pub use crate::domain::workflow::services::fact_replay::derive_node_artifact;
    pub use crate::domain::workflow::services::secret_masker::mask_sensitive_artifact;
    pub use crate::domain::workflow::validation::validate_all;
    pub use crate::domain::workflow::validation::validate_name;
    pub use crate::domain::workflow::value_objects::definition::EnvironmentVariableName;
    pub use crate::domain::workflow::value_objects::definition::InputParameterRef;
    pub use crate::domain::workflow::value_objects::field_path::FieldPath;
    pub use crate::usecase::workflow::dto::facet_summary_to_dto;
    pub use crate::usecase::workflow::node_startup::NodeStartupGateway;
    pub use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
    pub use crate::usecase::workflow::test_helpers::NoopArchiveRepository;

    pub use crate::adaptor::gateway::workflow::builtin::get_builtin_facet;
    pub use crate::adaptor::gateway::workflow::builtin::list_builtin_facet_keys;
    pub use crate::adaptor::gateway::workflow::builtin::list_builtin_workflows;

    pub use crate::adaptor::gateway::workflow::definition_repository::WorkflowDefinitionFileRepository;
    pub use crate::adaptor::gateway::workflow::definition_repository::WorkflowDefinitionFileSourceGateway;
    pub use crate::adaptor::gateway::workflow::diagnostics::DiagnosticScope;

    pub use crate::adaptor::gateway::workflow::diagnostics::collect_all_facet_keys;
    pub use crate::adaptor::gateway::workflow::diagnostics::diagnose_all;
    pub use crate::adaptor::gateway::workflow::diagnostics::diagnose_directory;
    pub use crate::adaptor::gateway::workflow::diagnostics::diagnose_lua_workflow_source;
    pub use crate::adaptor::gateway::workflow::diagnostics::diagnose_workflow_facet_references;
    pub use crate::adaptor::gateway::workflow::diagnostics::diagnose_workflow_source;
    pub use crate::adaptor::gateway::workflow::diagnostics::load_workflows_in_scope;
    pub use crate::adaptor::gateway::workflow::diagnostics::WorkflowSourceDiagnostics;
    pub use crate::adaptor::gateway::workflow::diagnostics_gateway::WorkflowDiagnosticsFileGateway;
    pub use crate::adaptor::gateway::workflow::editor_gateway::NoopWorkflowExternalEditorGateway;
    pub use crate::adaptor::gateway::workflow::editor_gateway::WorkflowExternalEditorGateway;

    pub use crate::adaptor::gateway::workflow::editor_gateway::resolve_facet_editor_path;
    pub use crate::adaptor::gateway::workflow::editor_gateway::resolve_workflow_editor_path;
    pub use crate::adaptor::gateway::workflow::event_repository::WorkflowEventLogRepository;
    pub use crate::adaptor::gateway::workflow::execution_archive_repository::ExecutionTreeArchiveFactRepository;
    pub use crate::adaptor::gateway::workflow::execution_projection_repository::WorkflowExecutionProjectionLogRepository;
    pub use crate::adaptor::gateway::workflow::facet::delete_facet;
    pub use crate::adaptor::gateway::workflow::facet::list_facet_summaries;
    pub use crate::adaptor::gateway::workflow::facet::list_facets;
    pub use crate::adaptor::gateway::workflow::facet::load_facet;
    pub use crate::adaptor::gateway::workflow::facet::resolve_facet_path;
    pub use crate::adaptor::gateway::workflow::facet::save_facet;
    pub use crate::adaptor::gateway::workflow::facet::validate_facet_key;
    pub use crate::adaptor::gateway::workflow::facet::FacetError;
    pub use crate::adaptor::gateway::workflow::facet::FacetKind;
    pub use crate::adaptor::gateway::workflow::facet_repository::WorkflowFacetFileRepository;
    pub use crate::adaptor::gateway::workflow::fact_codec::decode;
    pub use crate::adaptor::gateway::workflow::fact_codec::encode_detail;
    pub use crate::adaptor::gateway::workflow::fact_codec::event_type;
    pub use crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed;
    pub use crate::adaptor::gateway::workflow::fact_log::append_facts_for_events;
    pub use crate::adaptor::gateway::workflow::fact_log::append_pending_rows;
    pub use crate::adaptor::gateway::workflow::fact_log::append_single_fact;
    pub use crate::adaptor::gateway::workflow::fact_log::fold_tree_from;
    pub use crate::adaptor::gateway::workflow::fact_log::list_tree_ids;
    pub use crate::adaptor::gateway::workflow::fact_log::node_meta_from_row;
    pub use crate::adaptor::gateway::workflow::fact_log::pending_single_fact;
    pub use crate::adaptor::gateway::workflow::fact_log::read_records_for_event_types;
    pub use crate::adaptor::gateway::workflow::fact_log::read_tree_records;
    pub use crate::adaptor::gateway::workflow::fact_log::read_tree_records_from;
    pub use crate::adaptor::gateway::workflow::fact_log::reconcile_tree_pass;
    pub use crate::adaptor::gateway::workflow::fact_log::resolve_unknown_append;
    pub use crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend;
    pub use crate::adaptor::gateway::workflow::fact_log::FactReadError;
    pub use crate::adaptor::gateway::workflow::fact_log::PendingFactRow;
    pub use crate::adaptor::gateway::workflow::lua::field_span::MAX_SPAN_SOURCE_BYTES;
    pub use crate::adaptor::gateway::workflow::lua::handle;
    pub use crate::adaptor::gateway::workflow::lua::load_lua_workflow;
    pub use crate::adaptor::gateway::workflow::lua::load_lua_workflow_with_limits;
    pub use crate::adaptor::gateway::workflow::lua::stubs::facet_document_url;
    pub use crate::adaptor::gateway::workflow::lua::stubs::generate_editor_support;
    pub use crate::adaptor::gateway::workflow::lua::stubs::LUARC;
    pub use crate::adaptor::gateway::workflow::lua::test_handle_index;
    pub use crate::adaptor::gateway::workflow::lua::LuaFacetCatalog;
    pub use crate::adaptor::gateway::workflow::lua::LuaWorkflowDefinition;
    pub use crate::adaptor::gateway::workflow::lua::LuaWorkflowError;
    pub use crate::adaptor::gateway::workflow::lua::RuleDraft;
    pub use crate::adaptor::gateway::workflow::lua::SourceDraft;
    pub use crate::adaptor::gateway::workflow::lua::SourceRoot;
    pub use crate::adaptor::gateway::workflow::lua::WorkflowLuaHost;
    pub use crate::adaptor::gateway::workflow::lua::FN_ALL;
    pub use crate::adaptor::gateway::workflow::lua::FN_ANY;
    pub use crate::adaptor::gateway::workflow::lua::FN_WHEN;
    pub use crate::adaptor::gateway::workflow::lua::HANDLE_CHILD;
    pub use crate::adaptor::gateway::workflow::lua::HANDLE_NODE;
    pub use crate::adaptor::gateway::workflow::lua::HANDLE_PREDICATE;
    pub use crate::adaptor::gateway::workflow::lua::HANDLE_RULE;
    pub use crate::adaptor::gateway::workflow::lua::HANDLE_SOURCE;
    pub use crate::adaptor::gateway::workflow::lua::MAX_HOST_ARENA_ENTRIES;
    pub use crate::adaptor::gateway::workflow::mapper::event_draft_to_event;
    pub use crate::adaptor::gateway::workflow::mapper::schema_workflow_to_domain;
    pub use crate::adaptor::gateway::workflow::node_process::WorkflowNodeProcesses;
    pub use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
    pub use crate::adaptor::gateway::workflow::node_session_boundary::ProviderWorkflowAgentSessionPort;
    pub use crate::adaptor::gateway::workflow::node_session_boundary::WorkflowAgentSessionPort;
    pub use crate::adaptor::gateway::workflow::node_session_boundary::WorkflowSessionLaunchConfig;
    pub use crate::adaptor::gateway::workflow::runtime_command_gateway::WorkflowRuntimeCommandGateway;
    pub use crate::adaptor::gateway::workflow::runtime_command_gateway::WorkflowRuntimeCommandGatewayDeps;
    pub use crate::adaptor::gateway::workflow::runtime_resolver::resolve_workflow_by_name;
    pub use crate::adaptor::gateway::workflow::runtime_resolver::AppConfigManagedWorktreeResolver;
    pub use crate::adaptor::gateway::workflow::secret_source::collect_configured_secret_values;
    pub use crate::adaptor::gateway::workflow::span_map::YamlSpanMap;
    pub use crate::adaptor::gateway::workflow::startup_repository::HostWorkflowStartup;
    pub use crate::adaptor::gateway::workflow::startup_repository::StoredWorkflowStartupRepository;
    pub use crate::adaptor::gateway::workflow::storage::delete_workflow;
    pub use crate::adaptor::gateway::workflow::storage::diagnose_workflow_file;
    pub use crate::adaptor::gateway::workflow::storage::list_workflows;
    pub use crate::adaptor::gateway::workflow::storage::list_workflows_with_facets;
    pub use crate::adaptor::gateway::workflow::storage::load_workflow;
    pub use crate::adaptor::gateway::workflow::storage::parse_workflow_source;
    pub use crate::adaptor::gateway::workflow::storage::resolve_and_validate_workflow_facets;
    pub use crate::adaptor::gateway::workflow::storage::resolve_workflow_path;
    pub use crate::adaptor::gateway::workflow::storage::save_workflow;
    pub use crate::adaptor::gateway::workflow::storage::save_workflow_source;
    pub use crate::adaptor::gateway::workflow::storage::workflow_files;
    pub use crate::adaptor::gateway::workflow::storage::StorageError;
    pub use crate::adaptor::gateway::workflow::storage::DUPLICATE_NAME_DESCRIPTION;
    pub use crate::adaptor::gateway::workflow::stored_definition::read_tree_context;
    pub use crate::adaptor::gateway::workflow::test_support::append_canonical_events;
    pub use crate::adaptor::gateway::workflow::test_support::seed_canonical_execution;
    pub use crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition;
    pub use crate::adaptor::gateway::workflow::test_support::seed_workflow_session_facts;
    pub use crate::adaptor::gateway::workflow::test_support::WorkflowSessionFactSeed;
    pub use crate::adaptor::gateway::workflow::workflow_host::activation::RuntimeActivationGate;
    pub use crate::adaptor::gateway::workflow::workflow_host::build_command_artifact;
    pub use crate::adaptor::gateway::workflow::workflow_host::command_env;
    pub use crate::adaptor::gateway::workflow::workflow_host::command_preparation::CommandExecutionInput;
    pub use crate::adaptor::gateway::workflow::workflow_host::current_timestamp;
    pub use crate::adaptor::gateway::workflow::workflow_host::delegate::DelegateInjectionOrigin;
    pub use crate::adaptor::gateway::workflow::workflow_host::delegate::HostDelegateContinuation;
    pub use crate::adaptor::gateway::workflow::workflow_host::node_startup::HostNodeStartup;
    pub use crate::adaptor::gateway::workflow::workflow_host::output_limit::MAX_OUTPUT_SIZE;
    pub use crate::adaptor::gateway::workflow::workflow_host::output_limit::TRUNCATION_MARKER;
    pub use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering::render_parameter_references;

    pub use crate::adaptor::gateway::workflow::test_helpers::ConfiguredWorktreeGateway;
    pub use crate::adaptor::gateway::workflow::workflow_host::runtime_session::broadcast_state;
    pub use crate::adaptor::gateway::workflow::workflow_host::ControlPlaneCommitCandidate;
    pub use crate::adaptor::gateway::workflow::workflow_host::WorkflowExecutionInsert;
    pub use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies;
    pub use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
    pub use crate::adaptor::gateway::workflow::worktree_context::execution_worktree_path;
    pub use crate::adaptor::gateway::workflow::worktree_context::workspace_worktree_path_with;
    pub use crate::adaptor::gateway::workflow::worktree_context::StoredWorkspaceWorktreePathQuery;
    pub use crate::adaptor::gateway::workflow::worktree_context::WorktreeContextReadError;
    pub use crate::adaptor::gateway::workflow::worktree_gateway::canonicalize_managed_worktree_path_inner;
    pub use crate::adaptor::gateway::workflow::worktree_gateway::normalize_worktree_filter_path;
    pub use crate::adaptor::gateway::workflow::worktree_gateway::PassthroughManagedWorktreeGateway;
    pub use crate::adaptor::gateway::workflow::worktree_gateway::RepositoryIsolatedWorktreeGateway;
    pub use crate::adaptor::presenter::workflow::workflow_execution_to_view;
    pub use crate::domain::workflow::entities::workflow_execution::delegate::DelegateInjection;
    pub use crate::domain::workflow::entities::workflow_execution::AppliedNodeCompletionHandshake;
    pub use crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision;
    pub use crate::domain::workflow::entities::workflow_execution::ExecutionTree as workflow_execution_ExecutionTree;
    pub use crate::domain::workflow::entities::workflow_execution::ExecutionTreeRestore;
    pub use crate::domain::workflow::entities::workflow_execution::NodeStart;
    pub use crate::domain::workflow::entities::workflow_execution::PendingAdvance;
    pub use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
    pub use crate::domain::workflow::entities::workflow_execution::TransitionOutcome;
    pub use crate::domain::workflow::entities::workflow_execution::WorkflowDefaults;
    pub use crate::domain::workflow::error::WorkflowError;
    pub use crate::domain::workflow::gateway::IsolatedWorktreeGateway;
    pub use crate::domain::workflow::gateway::ManagedWorktreeGateway;
    pub use crate::domain::workflow::gateway::NodeProcessReader;
    pub use crate::domain::workflow::repository::ExecutionTreeArchiveCandidate;
    pub use crate::domain::workflow::repository::ExecutionTreeArchiveRecord;
    pub use crate::domain::workflow::repository::ExecutionTreeArchiveRepository;
    pub use crate::domain::workflow::repository::FacetRepository;
    pub use crate::domain::workflow::repository::WorkflowDefinitionRepository;
    pub use crate::domain::workflow::repository::WorkflowStartupRepository;
    pub use crate::domain::workflow::services::fact_replay::derive_read_model;
    pub use crate::domain::workflow::services::fact_replay::derive_session_facts;
    pub use crate::domain::workflow::services::fact_replay::fold_execution_tree;
    pub use crate::domain::workflow::services::fact_replay::tree_fold_count;
    pub use crate::domain::workflow::services::fact_replay::without_tree_fold;
    pub use crate::domain::workflow::services::reference::resolve_command_environment;
    pub use crate::domain::workflow::services::reference::resolve_entry_bindings;
    pub use crate::domain::workflow::services::routing::route_in_scope;
    pub use crate::domain::workflow::services::routing::RouteDecision;
    pub use crate::domain::workflow::services::secret_masker::mask_sensitive_text;
    pub use crate::domain::workflow::services::validation::validate;
    pub use crate::domain::workflow::services::validation::ValidationError;
    pub use crate::domain::workflow::value_objects::definition::ChildEntry;
    pub use crate::domain::workflow::value_objects::definition::CommandSpec;
    pub use crate::domain::workflow::value_objects::definition::EffectiveRules;
    pub use crate::domain::workflow::value_objects::definition::FacetRefs as definition_FacetRefs;
    pub use crate::domain::workflow::value_objects::definition::FanoutSpec;
    pub use crate::domain::workflow::value_objects::definition::InputParam;
    pub use crate::domain::workflow::value_objects::definition::InputSourceRef;
    pub use crate::domain::workflow::value_objects::definition::ItemsSource;
    pub use crate::domain::workflow::value_objects::definition::NodeCompletion;
    pub use crate::domain::workflow::value_objects::definition::NodeDefinition;
    pub use crate::domain::workflow::value_objects::definition::NodeKind;
    pub use crate::domain::workflow::value_objects::definition::NodeKindName;
    pub use crate::domain::workflow::value_objects::definition::Rule;
    pub use crate::domain::workflow::value_objects::definition::SchemaDef;
    pub use crate::domain::workflow::value_objects::definition::SequenceSpec;
    pub use crate::domain::workflow::value_objects::definition::SessionDelegate;
    pub use crate::domain::workflow::value_objects::definition::SessionPermission;
    pub use crate::domain::workflow::value_objects::definition::SessionSpec;
    pub use crate::domain::workflow::value_objects::definition::WorkflowDefinition;
    pub use crate::domain::workflow::value_objects::definition::WorkflowSourceFormat;
    pub use crate::domain::workflow::value_objects::definition::WorkflowSummary;
    pub use crate::domain::workflow::value_objects::definition::WorktreeMode;
    pub use crate::domain::workflow::value_objects::execution::Artifact;
    pub use crate::domain::workflow::value_objects::execution::ExecutionOrigin;
    pub use crate::domain::workflow::value_objects::execution::ExecutionStatus;
    pub use crate::domain::workflow::value_objects::execution::ExecutionTree;
    pub use crate::domain::workflow::value_objects::execution::Fanout;
    pub use crate::domain::workflow::value_objects::execution_metadata::ExecutionStatusFilter;
    pub use crate::domain::workflow::value_objects::execution_metadata::WorkflowExecutionSummary;
    pub use crate::domain::workflow::value_objects::facet::FacetKind as facet_FacetKind;

    pub use crate::domain::workflow::value_objects::failure::NodeExecutionFailureKind;
    pub use crate::domain::workflow::value_objects::ids::ExecutionTreeId;
    pub use crate::domain::workflow::value_objects::node_execution::ExecutionParentRef;
    pub use crate::domain::workflow::value_objects::node_execution::NodeCompletionSignal;
    pub use crate::domain::workflow::value_objects::node_execution::NodeCompletionSignalState;
    pub use crate::domain::workflow::value_objects::node_execution::NodeExecutionStatus;
    pub use crate::domain::workflow::value_objects::node_execution::NodeProcessPresence;
    pub use crate::domain::workflow::value_objects::node_fact::AbortRequestedFact;
    pub use crate::domain::workflow::value_objects::node_fact::AgentActivityObservedFact;
    pub use crate::domain::workflow::value_objects::node_fact::AgentSessionActivity;
    pub use crate::domain::workflow::value_objects::node_fact::ArchiveRequestedFact;
    pub use crate::domain::workflow::value_objects::node_fact::ArtifactProducedFact;
    pub use crate::domain::workflow::value_objects::node_fact::CommandSpawnedFact;
    pub use crate::domain::workflow::value_objects::node_fact::ExecutionTreeLaunch;
    pub use crate::domain::workflow::value_objects::node_fact::NodeFact;
    pub use crate::domain::workflow::value_objects::node_fact::NodeFactMeta;
    pub use crate::domain::workflow::value_objects::node_fact::NodeFactRecord;
    pub use crate::domain::workflow::value_objects::node_fact::ProcessExitedFact;
    pub use crate::domain::workflow::value_objects::node_fact::RuntimeFailureObservedFact;
    pub use crate::domain::workflow::value_objects::node_fact::SessionAttachedFact;
    pub use crate::domain::workflow::value_objects::node_fact::SessionExecutionTreeRootFacts;
    pub use crate::domain::workflow::value_objects::node_fact::StartedFact;
    pub use crate::domain::workflow::value_objects::node_fact::StopReceivedFact;
    pub use crate::domain::workflow::value_objects::node_fact::SubmitReceivedFact;
    pub use crate::domain::workflow::value_objects::node_fact::TreeRootFact;
    pub use crate::domain::workflow::value_objects::predicate::Predicate;
    pub use crate::domain::workflow::value_objects::runtime_event::WorkflowEvent;
    pub use crate::domain::workflow::value_objects::runtime_projection::TokenUsage;
    pub use crate::domain::workflow::value_objects::state::RuntimeExecutionState;
    pub use crate::domain::workflow::value_objects::worktree_origin::IsolatedWorktree;
    pub use crate::usecase::workflow::command::abort_execution::AbortExecutionCommand;
    pub use crate::usecase::workflow::command::abort_execution::WorkflowAbortExecutionUsecase;
    pub use crate::usecase::workflow::command::approval::ApprovalCommand;
    pub use crate::usecase::workflow::command::retry_node::ResumeSessionNodeCommand;
    pub use crate::usecase::workflow::command::retry_node::RetryNodeCommand;
    pub use crate::usecase::workflow::command::start_execution::ResolvedStartExecutionCommand;
    pub use crate::usecase::workflow::command::start_execution::StartExecutionCommand;
    pub use crate::usecase::workflow::command::submit_output::SubmitOutputArtifact;
    pub use crate::usecase::workflow::command::submit_output::SubmitOutputCommand;
    pub use crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit;
    pub use crate::usecase::workflow::control_plane::WorkflowControlPlaneGateway;
    pub use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;
    pub use crate::usecase::workflow::delegate::DelegateContinuationGateway;
    pub use crate::usecase::workflow::delegate::DelegateContinuationUsecase;
    pub use crate::usecase::workflow::diagnostic_dto::DiagnosticReport;
    pub use crate::usecase::workflow::diagnostic_dto::DiagnosticSpan;
    pub use crate::usecase::workflow::diagnostic_dto::DiagnosticStage;
    pub use crate::usecase::workflow::diagnostic_dto::Severity;
    pub use crate::usecase::workflow::WorkflowReadUsecase;
    pub use crate::usecase::workflow::WorkflowUsecase;

    pub use crate::usecase::workflow::node_startup::FailedNodeStart;
    pub use crate::usecase::workflow::output::WorkflowOutputUsecase;
    pub use crate::usecase::workflow::ports::ExecutionTreeProcessGateway;
    pub use crate::usecase::workflow::ports::ExternalEditorGateway;
    pub use crate::usecase::workflow::ports::WorkflowAbortExecutionGateway;
    pub use crate::usecase::workflow::ports::WorkflowDefinitionSourceGateway;
    pub use crate::usecase::workflow::ports::WorkflowDiagnosticsGateway;
    pub use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;
    pub use crate::usecase::workflow::ports::WorkflowEventDraft;
    pub use crate::usecase::workflow::ports::WorkflowEventRepository;
    pub use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;
    pub use crate::usecase::workflow::ports::WorkflowRuntimeShutdownGateway;
    pub use crate::usecase::workflow::ports::WorkflowRuntimeStateGateway;
    pub use crate::usecase::workflow::ports::WorkflowSourceSaveError;
    pub use crate::usecase::workflow::ports::WorkflowStartExecutionGateway;
    pub use crate::usecase::workflow::query_service::WorkflowGetOutputResult;
    pub use crate::usecase::workflow::query_service::WorkflowQueryService;
    pub use crate::usecase::workflow::runtime_command::WorkflowRuntimeUsecase;
    pub use crate::usecase::workflow::runtime_driver::NodeOutcome;

    pub use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
    pub use crate::usecase::workflow::runtime_resolver::ManagedWorktreeResolver;
    pub use crate::usecase::workflow::runtime_resolver::ManagedWorktreeResolverError;
    pub use crate::usecase::workflow::runtime_resolver::WorkflowDefinitionResolver;
    pub use crate::usecase::workflow::runtime_resolver::WorkflowDefinitionResolverError;
    pub use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;
    pub use crate::usecase::workflow::startup::check_startup_definition;
    pub use crate::usecase::workflow::startup::WorkflowStartupGateway;
    pub use crate::usecase::workflow::startup::WorkflowStartupUsecase;
    pub use crate::usecase::workflow::workspace_tree::WorkspaceNodeContentDto;
}
pub mod workspace {
    pub async fn node_for_execution(
        repository: &dyn crate::domain::workspace_tree::WorkspaceTreeRepository,
        workspace: &crate::domain::workspace_tree::WorkspaceIdentity,
        id: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
        crate::domain::workflow::WorkflowError,
    > {
        let tree = repository
            .load_trees(std::slice::from_ref(workspace))
            .await
            .remove(0)?;
        Ok(tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(id))
            .cloned())
    }

    pub use crate::adaptor::controller::client::workspace_tree::shared::register_shared;
    pub use crate::adaptor::gateway::workspace_tree::query_service::SqliteWorkspaceQueryService;

    pub use crate::adaptor::gateway::workspace_tree::repository::SqliteWorkspaceTreeRepository;
    pub use crate::domain::workspace_tree::entities::WorkspaceTree;
    pub use crate::domain::workspace_tree::repository::WorkspaceTreeRepository;
    pub use crate::domain::workspace_tree::value_objects::WorkspaceIdentity;
    pub use crate::domain::workspace_tree::value_objects::WorkspaceNodeKind;
    pub use crate::domain::workspace_tree::value_objects::WorkspaceNodeStatus;
    pub use crate::domain::workspace_tree::value_objects::WorkspaceNodeStatusClassification;
    pub use crate::domain::workspace_tree::value_objects::WorkspaceTreeNode;
    pub use crate::domain::workspace_tree::visible::WorkspaceVisibleNode;
    pub use crate::usecase::workspace_tree::list::WorkspaceList;
    pub use crate::usecase::workspace_tree::list::WorkspaceListUsecase;
    pub use crate::usecase::workspace_tree::list::WorkspaceListWorktree;
    pub use crate::usecase::workspace_tree::query_service::WorkspaceQueryService;
    pub use crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService;
    pub use crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository;
    pub use crate::usecase::workspace_tree::worktree_path::WorkspaceWorktreePathQuery;
}

pub mod wire {
    pub use crate::adaptor::presenter::client::ReviewThreadDto;
    pub fn from_message<M: prost::Message>(
        name: &str,
        message: &M,
    ) -> Result<serde_json::Value, String> {
        crate::adaptor::presenter::client::from_message(name, message)
    }
    pub use crate::adaptor::presenter::client::command_error;
    pub use crate::adaptor::presenter::client::command_request;
    pub use crate::adaptor::presenter::client::command_result;
    pub use crate::adaptor::presenter::client::test_helpers::command_name;
    pub use crate::adaptor::presenter::client::test_helpers::command_request_from_value;

    pub use crate::adaptor::presenter::client::state_payload;
    pub use crate::adaptor::presenter::client::state_subscription_event;
    pub use crate::adaptor::presenter::client::terminal_event;
    pub use crate::adaptor::presenter::client::terminal_surface_owner_v1;
    pub use crate::adaptor::presenter::client::AppendReviewCommentRequest;
    pub use crate::adaptor::presenter::client::BuildDiffFileTreeRequest;
    pub use crate::adaptor::presenter::client::CommandError;

    pub use crate::adaptor::presenter::client::CommandRequest;
    pub use crate::adaptor::presenter::client::CommandResult;
    pub use crate::adaptor::presenter::client::CreateReviewThreadRequest;
    pub use crate::adaptor::presenter::client::CreateWorktreeRequest;
    pub use crate::adaptor::presenter::client::DeleteReviewThreadRequest;
    pub use crate::adaptor::presenter::client::FetchIssuesRequest;
    pub use crate::adaptor::presenter::client::GetOrSpawnTerminalSurfaceRequest;
    pub use crate::adaptor::presenter::client::GitCreateBranchRequest;
    pub use crate::adaptor::presenter::client::GitStageRequest;
    pub use crate::adaptor::presenter::client::GitUnstageRequest;
    pub use crate::adaptor::presenter::client::KillTerminalSurfaceRequest;
    pub use crate::adaptor::presenter::client::ListDiffFileEntryInput;
    pub use crate::adaptor::presenter::client::Liststring;
    pub use crate::adaptor::presenter::client::RefreshWorkspacesRequest;
    pub use crate::adaptor::presenter::client::RenameWorkspaceSessionNodeRequest;
    pub use crate::adaptor::presenter::client::ResizeTerminalSurfaceRequest;
    pub use crate::adaptor::presenter::client::ResolveReviewThreadRequest;
    pub use crate::adaptor::presenter::client::ResultBool;
    pub use crate::adaptor::presenter::client::ResultString;
    pub use crate::adaptor::presenter::client::SaveWorkspaceStateRequest;
    pub use crate::adaptor::presenter::client::StartStateSubscriptionRequest;
    pub use crate::adaptor::presenter::client::StateSubscriptionEvent;
    pub use crate::adaptor::presenter::client::TerminalSurfaceOwnerV1;
    pub use crate::adaptor::presenter::client::TerminalSurfaceOwnerV1Workspace;
    pub use crate::adaptor::presenter::client::Unit;
    pub use crate::adaptor::presenter::client::UpdateAppSettingsRequest;
    pub use crate::adaptor::presenter::client::UpdateLoginItemPreferenceRequest;
    pub use crate::adaptor::presenter::client::UpdateWorkflowConfigRequest;
    pub use crate::adaptor::presenter::client::WindowSettings;
    pub use crate::adaptor::presenter::client::WorkflowSection;
    pub use crate::adaptor::presenter::client::WorkspaceStateDto;
    pub use crate::adaptor::presenter::client::WritePathsToTerminalSurfaceRequest;
    pub use crate::adaptor::presenter::client::WriteTerminalSurfaceRequest;
    pub use crate::adaptor::presenter::client::COMMAND_NAMES;
}

pub mod fixtures {
    pub use crate::adaptor::gateway::agent_session::test_helpers::FailingSearchPathSource as availability_FailingSearchPathSource;
    pub use crate::domain::workflow::services::test_helpers::session_attached;
    pub use crate::usecase::repository_state::test_helpers::EmptyScanner as repository_state_EmptyScanner;
    pub use crate::usecase::test_helpers::Files as watcher_Files;

    pub use crate::adaptor::gateway::workflow::test_helpers::predicate_yaml as fixtures_adaptor_gateway_workflow_diagnostics_predicate_yaml;
    pub use crate::adaptor::gateway::workflow::test_helpers::RecordingWorkflowAgentSessions as adaptor_gateway_workflow_workflow_host_RecordingWorkflowAgentSessions;
    pub use crate::adaptor::gateway::workflow::test_helpers::EFFECT_AGENT_SESSION_ID as adaptor_gateway_workflow_workflow_host_EFFECT_AGENT_SESSION_ID;
    pub use crate::usecase::repository_state::test_helpers::counting_service as repository_state_counting_service;

    pub use crate::usecase::repository_state::test_helpers::CountingScanner as repository_state_CountingScanner;

    pub use crate::usecase::repository_state::test_helpers::TestRepositoryStateRepository as repository_state_TestRepositoryStateRepository;

    pub use crate::adaptor::controller::api::test_helpers::dispatch as fixtures_adaptor_controller_api_client_dispatch;
    pub use crate::adaptor::gateway::agent_session::test_helpers::metadata as fixtures_adaptor_gateway_agent_session_agent_session_history_query_service_metadata;
    pub use crate::adaptor::gateway::terminal_surface::test_helpers::insert_test_session as fixtures_adaptor_gateway_terminal_surface_runtime_gateway_impl_insert_test_session;
    pub use crate::domain::workflow::entities::workflow_execution::test_helpers::execution_id_of as fixtures_domain_workflow_entities_workflow_execution_mod_execution_id_of;
    pub use crate::domain::workflow::entities::workflow_execution::test_helpers::id_source as fixtures_domain_workflow_entities_workflow_execution_mod_id_source;
    pub use crate::domain::workflow::entities::workflow_execution::test_helpers::started_names as fixtures_domain_workflow_entities_workflow_execution_mod_started_names;

    pub use crate::adaptor::gateway::repository::test_helpers::event as fixtures_adaptor_gateway_repository_state_event;

    pub use crate::adaptor::gateway::repository::test_helpers::state_with_subscriptions as fixtures_adaptor_gateway_repository_state_state_with_subscriptions;

    pub use crate::adaptor::gateway::provider_lifecycle::test_helpers::context as fixtures_adaptor_gateway_provider_lifecycle_mod_context;
    pub use crate::adaptor::gateway::provider_lifecycle::test_helpers::slot_id as fixtures_adaptor_gateway_provider_lifecycle_mod_slot_id;

    pub use crate::adaptor::gateway::local_event_store::test_helpers::scope as fixtures_adaptor_gateway_local_event_store_provider_lifecycle_codec_scope;
    pub use crate::domain::comment::test_helpers::agent as fixtures_domain_comment_mod_agent;
    pub use crate::infrastructure::local_api::test_helpers::discovery as fixtures_infrastructure_local_api_client_discovery;
    pub use crate::usecase::terminal_surface::test_helpers::workspace_owner as fixtures_usecase_terminal_surface_spawn_usecase_workspace_owner;
    pub use crate::usecase::test_helpers::start_read as fixtures_infrastructure_state_subscription_start_read;
    pub use crate::usecase::test_helpers::stop_read as fixtures_infrastructure_state_subscription_stop_read;
}

pub mod installation {
    pub use crate::infrastructure::platform::installation::{
        cli_link, create_cli_link, read_only, run_admin_command, CliLink,
    };
}

pub mod data_dir {
    pub use crate::infrastructure::platform::data_dir::*;
}

pub mod client_protocol {
    pub use crate::infrastructure::client_protocol::{rpc, wire};
}
