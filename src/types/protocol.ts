import type { GitFileStatus } from "./git";

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

// --- ファイル・Diff ---

export interface GitStatusSync {
	files: GitFileStatus[];
}

export interface FileContentRequest {
	path: string;
	diff_base?: "branch-base" | "staged";
}

interface FileContentResponse {
	path: string;
	original: string;
	modified: string;
	staged?: string;
}

export interface FileChange {
	path: string;
	kind: string;
}

// --- Git操作 ---

export interface GitStage {
	paths: string[];
}

export interface GitUnstage {
	paths: string[];
}

interface GitStageResult {
	success: boolean;
	error?: string;
	files: GitFileStatus[];
}

export interface GitStageHunk {
	patch: string;
}

// --- Git Commit / Push / BranchInfo ---

export interface GitCommitRequest {
	message: string;
}

export interface GitCommitResult {
	success: boolean;
	hash?: string;
	error?: string;
}

export type GitPushRequest = Record<string, never>;

export interface GitPushResult {
	success: boolean;
	output?: string;
	error?: string;
}

export type BranchInfoRequest = Record<string, never>;

export interface BranchInfoResponse {
	branch: string;
}

// --- コメント ---

export interface AddComment {
	file_path: string;
	line_number: number;
	end_line?: number;
	content: string;
}

export interface CommentItem {
	id: string;
	file_path: string;
	line_number: number;
	end_line?: number;
	content: string;
	status: "unsent" | "sent";
	created_at: number;
}

export interface DeleteComment {
	id: string;
}

export interface UpdateComment {
	id: string;
	content: string;
}

export interface CommentSync {
	comments: CommentItem[];
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
	has_pr?: boolean;
	pr_number?: number;
	pr_url?: string;
}

export interface WorktreeListResponse {
	worktrees: WorktreeEntryMsg[];
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
	is_default: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	has_pr?: boolean;
	pr_number?: number;
	pr_url?: string;
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
}

// --- 制御 ---

export interface ErrorMsg {
	code: string;
	message: string;
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
	| { type: "git_status_sync"; payload: GitStatusSync }
	| { type: "file_content_request"; payload: FileContentRequest }
	| { type: "file_content_response"; payload: FileContentResponse }
	| { type: "file_change"; payload: FileChange }
	| { type: "git_stage"; payload: GitStage }
	| { type: "git_unstage"; payload: GitUnstage }
	| { type: "git_stage_result"; payload: GitStageResult }
	| { type: "git_stage_hunk"; payload: GitStageHunk }
	| { type: "git_commit_request"; payload: GitCommitRequest }
	| { type: "git_commit_result"; payload: GitCommitResult }
	| { type: "git_push_request"; payload: GitPushRequest }
	| { type: "git_push_result"; payload: GitPushResult }
	| { type: "branch_info_request"; payload: BranchInfoRequest }
	| { type: "branch_info_response"; payload: BranchInfoResponse }
	| { type: "git_status_request"; payload: Record<string, never> }
	| { type: "add_comment"; payload: AddComment }
	| { type: "delete_comment"; payload: DeleteComment }
	| { type: "update_comment"; payload: UpdateComment }
	| { type: "comments_sync"; payload: CommentSync }
	| { type: "worktree_list_request"; payload: WorktreeListRequest }
	| { type: "worktree_list_response"; payload: WorktreeListResponse }
	| { type: "worktree_select_request"; payload: WorktreeSelectRequest }
	| { type: "worktree_select_response"; payload: WorktreeSelectResponse }
	| { type: "branch_list_sync"; payload: BranchListSync }
	| { type: "agent_state_sync"; payload: AgentStateSync }
	| { type: "error"; payload: ErrorMsg };

export function serializeMessage(msg: WsMessage): string {
	return JSON.stringify(msg);
}

export function deserializeMessage(json: string): WsMessage {
	return JSON.parse(json) as WsMessage;
}
