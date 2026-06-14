import type { WorkflowStatePayload } from "@/types/workflow";
import type {
	AgentEditorContext,
	ImageAttachment,
	MentionReference,
	MessagePart,
	MessageRole,
	ModelInfo,
	PermissionMode,
	QueuedAgentTurn,
	SessionState,
	SessionSummary,
	SlashCommand,
	TurnPhase,
} from "./session";

// --- 認証 ---

interface AuthChallenge {
	challenge: string;
}

interface AuthResponse {
	hmac: string;
}

interface AuthResult {
	success: boolean;
	message?: string;
}

// --- ターミナル ---

interface PtyOutputMsg {
	pty_id: number;
	data: string;
}

export interface PtyExitMsg {
	pty_id: number;
	exit_code: number | null;
}

export interface PtyInput {
	pty_id: number;
	data: string;
}

export interface PtyResize {
	pty_id: number;
	rows: number;
	cols: number;
}

export interface PtyReady {
	pty_id: number;
	cols: number;
	rows: number;
	label?: string;
	worktree_path?: string;
}

export interface PtyOutputRequest {
	pty_id: number;
}

export interface PtySpawnRequest {
	cols: number;
	rows: number;
	label?: string;
}

export interface PtySpawnResponse {
	success: boolean;
	pty_id?: number;
	error?: string;
}

export interface PtyKillRequest {
	pty_id: number;
}

export interface PtyKillResponse {
	success: boolean;
	pty_id: number;
	error?: string;
}

// --- Worktree ---

export type WorktreeListRequest = Record<string, never>;

export interface WorktreeEntryMsg {
	name: string;
	path: string;
	branch: string;
	is_main: boolean;
	is_locked: boolean;
	dirty_count: number;
	base_branch: string | null;
	repo_path?: string;
}

export interface WorktreeListResponse {
	worktrees: WorktreeEntryMsg[];
}

// PR ステータスは worktree 一覧の返却後に後追い配信される（2 段階化）。
export interface WorktreePrEntry {
	path: string;
	pr_number: number;
	pr_url: string;
}

export interface WorktreePrStatusSync {
	entries: WorktreePrEntry[];
}

export interface WorktreeSelectRequest {
	path: string;
}

export interface WorktreeSelectResponse {
	success: boolean;
	path: string;
	error?: string;
}

// --- ブランチリスト同期 ---

interface BranchCardMsg {
	name: string;
	is_main_worktree: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	ahead: number;
	behind: number;
	has_upstream: boolean;
	base_ahead: number;
}

export interface BranchListSync {
	branches: BranchCardMsg[];
}

// --- エージェント状態 ---

export type AgentState = "running" | "done" | "error" | "waiting";

export interface AgentStateSync {
	worktree_path: string;
	state: AgentState;
	exit_code: number | null;
	timestamp: number;
	session_id: string | null;
	pty_id?: string | null;
}

// --- バックエンド ---

export type BackendListRequest = Record<string, never>;

export interface BackendInfoMsg {
	id: string;
	name: string;
	available: boolean;
	available_models: ModelInfo[];
}

export interface BackendListResponse {
	backends: BackendInfoMsg[];
	default_id: string | null;
}

export interface AgentSessionStartRequest {
	worktree_path: string;
	backend_id?: string | null;
	permission_mode?: PermissionMode | null;
}

export interface AgentSessionStartResponse {
	success: boolean;
	session_id?: string | null;
	backend_id?: string | null;
	error?: string | null;
}

export interface AgentSessionSnapshotMessage {
	id: string;
	role: MessageRole;
	content?: string;
	thinking?: string;
	parts?: MessagePart[];
	timestamp: number;
	mentions?: MentionReference[];
}

export interface AgentSessionSnapshot {
	id: string;
	worktreePath: string;
	messages: AgentSessionSnapshotMessage[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	permissionMode: PermissionMode | string;
	selectedModel?: string | null;
	backendId?: string | null;
	workflowStepSession?: boolean;
	turnPhase: TurnPhase;
	availableModels: ModelInfo[];
	pendingQueue?: QueuedAgentTurn[];
	pendingQueueCount?: number;
}

export interface AgentSessionsRequest {
	worktree_path: string;
}

export interface AgentSessionsResponse {
	success: boolean;
	worktree_path: string;
	sessions: SessionSummary[];
	active_session?: AgentSessionSnapshot | null;
	error?: string | null;
}

export interface AgentSessionGetRequest {
	session_id: string;
}

export interface AgentSessionGetResponse {
	success: boolean;
	session_id: string;
	session?: AgentSessionSnapshot | null;
	error?: string | null;
}

export interface AgentMessageRequest {
	session_id?: string | null;
	worktree_path: string;
	content: string;
	permission_mode?: PermissionMode | null;
	backend_id?: string | null;
	images?: ImageAttachment[];
	mentions?: MentionReference[];
	editor_context?: AgentEditorContext | null;
}

export interface AgentMessageResponse {
	success: boolean;
	session_id?: string | null;
	human_message_id?: string | null;
	agent_message_id?: string | null;
	queued_turn_id?: string | null;
	pending_queue?: QueuedAgentTurn[];
	pending_queue_count?: number;
	sessions?: SessionSummary[];
	backend_id?: string | null;
	error?: string | null;
}

export interface AgentInterruptRequest {
	session_id: string;
}

export interface AgentInterruptResponse {
	success: boolean;
	session_id: string;
	error?: string | null;
}

export interface AgentQueueCancelRequest {
	session_id: string;
	queued_turn_id?: string | null;
}

export interface AgentQueueCancelResponse {
	success: boolean;
	session_id: string;
	canceled_count: number;
	pending_queue?: QueuedAgentTurn[];
	pending_queue_count?: number;
	error?: string | null;
}

export interface AgentSlashCommandsRequest {
	worktree_path: string;
}

export interface AgentSlashCommandsResponse {
	success: boolean;
	worktree_path: string;
	commands: SlashCommand[];
	error?: string | null;
}

export interface AgentSupportedCommandsUpdated {
	chat_session_id: string;
	commands: SlashCommand[];
}

export interface AgentMentionFilesRequest {
	request_id: string;
	worktree_path: string;
	query: string;
}

export interface AgentMentionFilesResponse {
	success: boolean;
	request_id: string;
	worktree_path: string;
	query: string;
	files: string[];
	error?: string | null;
}

export interface AgentImagePrepareRequest {
	request_id: string;
	data: number[];
}

export interface AgentImagePrepareResponse {
	success: boolean;
	request_id: string;
	attachment?: ImageAttachment | null;
	error?: string | null;
}

export interface AgentPermissionResponseRequest {
	session_id: string;
	request_id: string;
	behavior: "allow" | "deny";
	message?: string | null;
	updated_input?: Record<string, unknown> | null;
}

export interface AgentPermissionResponseResponse {
	success: boolean;
	session_id: string;
	request_id: string;
	error?: string | null;
}

export interface AgentModelSetRequest {
	session_id: string;
	model_id: string;
}

export interface AgentModelSetResponse {
	success: boolean;
	session_id: string;
	model_id?: string | null;
	error?: string | null;
}

export interface AgentPermissionModeSetRequest {
	session_id: string;
	permission_mode: PermissionMode;
}

export interface AgentPermissionModeSetResponse {
	success: boolean;
	session_id: string;
	permission_mode: PermissionMode;
	error?: string | null;
}

export interface AgentStreamSync {
	session_id: string;
	message_id: string;
	parts: MessagePart[];
}

// --- Review comments ---

export type ReviewActorKind = "human" | "agent";
export type ReviewThreadState = "open" | "resolved";
export type ReviewErrorCode =
	| "invalid_input"
	| "not_found"
	| "already_resolved"
	| "permission_denied"
	| "io"
	| "serialize";

export interface ReviewActor {
	kind: ReviewActorKind;
	backendId?: string | null;
	model?: string | null;
	displayName: string;
}

export interface ReviewTarget {
	filePath?: string | null;
	lineNumber?: number | null;
	endLine?: number | null;
}

export interface ReviewComment {
	id: string;
	threadId: string;
	author: ReviewActor;
	content: string;
	createdAt: number;
}

export interface ReviewResolveInfo {
	actor: ReviewActor;
	outcome: string;
	summary: string;
	resolvedAt: number;
}

export interface ReviewThread {
	id: string;
	worktreeName: string;
	author: ReviewActor;
	target: ReviewTarget;
	state: ReviewThreadState;
	comments: ReviewComment[];
	resolve?: ReviewResolveInfo | null;
	createdAt: number;
	updatedAt: number;
	version: number;
	canResolve: boolean;
}

export type AuthorScope = "mine" | "other";

export interface ReviewThreadFilter {
	file?: string | null;
	state?: ReviewThreadState | null;
	author?: AuthorScope | null;
	unread?: boolean | null;
	threadId?: string[];
}

export interface ReviewErrorPayload {
	code: ReviewErrorCode;
	message: string;
}

export interface ReviewListRequest {
	worktreeName?: string | null;
	filter?: ReviewThreadFilter | null;
}

export interface ReviewListResponse {
	success: boolean;
	worktreeName?: string | null;
	threads: ReviewThread[];
	error?: ReviewErrorPayload | null;
}

export interface ReviewGetRequest {
	worktreeName?: string | null;
	threadId: string;
}

export interface ReviewThreadResponse {
	success: boolean;
	worktreeName?: string | null;
	thread?: ReviewThread | null;
	error?: ReviewErrorPayload | null;
}

export interface ReviewCreateRequest {
	worktreeName?: string | null;
	target: ReviewTarget;
	content: string;
}

export interface ReviewAppendCommentRequest {
	worktreeName?: string | null;
	threadId: string;
	content: string;
}

export interface ReviewResolveRequest {
	worktreeName?: string | null;
	threadId: string;
	outcome: string;
	summary: string;
}

export interface ReviewHistoryRequest {
	worktreeName?: string | null;
	threadId: string;
}

export type ReviewHistoryEntry =
	| {
			kind: "thread_created";
			id: string;
			threadId: string;
			commentId: string;
			actor: ReviewActor;
			target: ReviewTarget;
			content: string;
			at: number;
	  }
	| {
			kind: "comment_appended";
			id: string;
			threadId: string;
			commentId: string;
			actor: ReviewActor;
			content: string;
			at: number;
	  }
	| {
			kind: "thread_resolved";
			id: string;
			threadId: string;
			actor: ReviewActor;
			outcome: string;
			summary: string;
			at: number;
	  };

export interface ReviewHistoryResponse {
	success: boolean;
	worktreeName?: string | null;
	events: ReviewHistoryEntry[];
	error?: ReviewErrorPayload | null;
}

// --- 制御 ---

export interface ErrorMsg {
	code: string;
	message: string;
}

// --- ブランチ情報 ---

export type BranchInfoRequest = Record<string, never>;

export interface BranchInfoResponse {
	branch: string;
}

// --- 統合メッセージ型 ---

export type WsMessage =
	| { type: "auth_challenge"; payload: AuthChallenge }
	| { type: "auth_response"; payload: AuthResponse }
	| { type: "auth_result"; payload: AuthResult }
	| { type: "pty_output"; payload: PtyOutputMsg }
	| { type: "pty_exit"; payload: PtyExitMsg }
	| { type: "pty_input"; payload: PtyInput }
	| { type: "pty_resize"; payload: PtyResize }
	| { type: "pty_ready"; payload: PtyReady }
	| { type: "pty_output_request"; payload: PtyOutputRequest }
	| { type: "pty_spawn_request"; payload: PtySpawnRequest }
	| { type: "pty_spawn_response"; payload: PtySpawnResponse }
	| { type: "pty_kill_request"; payload: PtyKillRequest }
	| { type: "pty_kill_response"; payload: PtyKillResponse }
	| { type: "worktree_list_request"; payload: WorktreeListRequest }
	| { type: "worktree_list_response"; payload: WorktreeListResponse }
	| { type: "worktree_select_request"; payload: WorktreeSelectRequest }
	| { type: "worktree_select_response"; payload: WorktreeSelectResponse }
	| { type: "worktree_pr_status_sync"; payload: WorktreePrStatusSync }
	| { type: "branch_info_request"; payload: BranchInfoRequest }
	| { type: "branch_info_response"; payload: BranchInfoResponse }
	| { type: "branch_list_sync"; payload: BranchListSync }
	| { type: "agent_state_sync"; payload: AgentStateSync }
	| { type: "backend_list_request"; payload: BackendListRequest }
	| { type: "backend_list_response"; payload: BackendListResponse }
	| { type: "agent_session_start_request"; payload: AgentSessionStartRequest }
	| { type: "agent_session_start_response"; payload: AgentSessionStartResponse }
	| { type: "agent_sessions_request"; payload: AgentSessionsRequest }
	| { type: "agent_sessions_response"; payload: AgentSessionsResponse }
	| { type: "agent_session_get_request"; payload: AgentSessionGetRequest }
	| { type: "agent_session_get_response"; payload: AgentSessionGetResponse }
	| { type: "agent_message_request"; payload: AgentMessageRequest }
	| { type: "agent_message_response"; payload: AgentMessageResponse }
	| { type: "agent_interrupt_request"; payload: AgentInterruptRequest }
	| { type: "agent_interrupt_response"; payload: AgentInterruptResponse }
	| { type: "agent_queue_cancel_request"; payload: AgentQueueCancelRequest }
	| { type: "agent_queue_cancel_response"; payload: AgentQueueCancelResponse }
	| {
			type: "agent_slash_commands_request";
			payload: AgentSlashCommandsRequest;
	  }
	| {
			type: "agent_slash_commands_response";
			payload: AgentSlashCommandsResponse;
	  }
	| {
			type: "agent_mention_files_request";
			payload: AgentMentionFilesRequest;
	  }
	| {
			type: "agent_mention_files_response";
			payload: AgentMentionFilesResponse;
	  }
	| {
			type: "agent_image_prepare_request";
			payload: AgentImagePrepareRequest;
	  }
	| {
			type: "agent_image_prepare_response";
			payload: AgentImagePrepareResponse;
	  }
	| {
			type: "agent_permission_response_request";
			payload: AgentPermissionResponseRequest;
	  }
	| {
			type: "agent_permission_response_response";
			payload: AgentPermissionResponseResponse;
	  }
	| { type: "agent_model_set_request"; payload: AgentModelSetRequest }
	| { type: "agent_model_set_response"; payload: AgentModelSetResponse }
	| {
			type: "agent_permission_mode_set_request";
			payload: AgentPermissionModeSetRequest;
	  }
	| {
			type: "agent_permission_mode_set_response";
			payload: AgentPermissionModeSetResponse;
	  }
	| { type: "agent_stream_sync"; payload: AgentStreamSync }
	| { type: "review_list_request"; payload: ReviewListRequest }
	| { type: "review_list_response"; payload: ReviewListResponse }
	| { type: "review_get_request"; payload: ReviewGetRequest }
	| { type: "review_thread_response"; payload: ReviewThreadResponse }
	| { type: "review_create_request"; payload: ReviewCreateRequest }
	| {
			type: "review_append_comment_request";
			payload: ReviewAppendCommentRequest;
	  }
	| { type: "review_resolve_request"; payload: ReviewResolveRequest }
	| { type: "review_history_request"; payload: ReviewHistoryRequest }
	| { type: "review_history_response"; payload: ReviewHistoryResponse }
	| { type: "workflow_state_sync"; payload: WorkflowStatePayload }
	| { type: "error"; payload: ErrorMsg };

export function serializeMessage(msg: WsMessage): string {
	return JSON.stringify(msg);
}

export function deserializeMessage(json: string): WsMessage {
	return JSON.parse(json) as WsMessage;
}
