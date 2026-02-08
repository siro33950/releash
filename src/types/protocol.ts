import type { GitFileStatus } from "./git";

// --- 認証 ---

export interface AuthChallenge {
	challenge: string;
}

export interface AuthResponse {
	hmac: string;
}

export interface AuthResult {
	success: boolean;
	message?: string;
}

// --- ターミナル ---

export interface PtyOutputMsg {
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
}

// --- ファイル・Diff ---

export interface GitStatusSync {
	files: GitFileStatus[];
}

export interface FileContentRequest {
	path: string;
}

export interface FileContentResponse {
	path: string;
	original: string;
	modified: string;
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

export interface GitStageResult {
	success: boolean;
	error?: string;
	files: GitFileStatus[];
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

export interface CommentSync {
	comments: CommentItem[];
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
	| { type: "git_status_sync"; payload: GitStatusSync }
	| { type: "file_content_request"; payload: FileContentRequest }
	| { type: "file_content_response"; payload: FileContentResponse }
	| { type: "file_change"; payload: FileChange }
	| { type: "git_stage"; payload: GitStage }
	| { type: "git_unstage"; payload: GitUnstage }
	| { type: "git_stage_result"; payload: GitStageResult }
	| { type: "git_status_request"; payload: Record<string, never> }
	| { type: "add_comment"; payload: AddComment }
	| { type: "comments_sync"; payload: CommentSync }
	| { type: "error"; payload: ErrorMsg };

export type WsMessageType = WsMessage["type"];

export function serializeMessage(msg: WsMessage): string {
	return JSON.stringify(msg);
}

export function deserializeMessage(json: string): WsMessage {
	return JSON.parse(json) as WsMessage;
}
