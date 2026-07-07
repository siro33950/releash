import type { AgentState } from "./protocol";

export type PermissionMode = "ask" | "edit" | "full";
export type LegacyPermissionMode = "readonly" | "edit" | "full";
export type PlanMode = boolean;

export const PERMISSION_MODES: readonly PermissionMode[] = [
	"ask",
	"edit",
	"full",
] as const;

/**
 * UI 表示用の抽象パーミッションモードラベル（共通定義）。
 * ModeSelector など全 UI で共有し、
 * 3 択の表示差分や変更漏れを防ぐ。
 */
export const PERMISSION_MODE_LABELS: Record<PermissionMode, string> = {
	ask: "Ask",
	edit: "Edit",
	full: "Full",
};

export function normalizePermissionMode(
	mode: PermissionMode | LegacyPermissionMode | string | null | undefined,
): PermissionMode {
	const normalized = typeof mode === "string" ? mode.trim().toLowerCase() : "";
	switch (normalized) {
		case "ask":
		case "edit":
		case "full":
			return normalized;
		case "readonly":
			return "ask";
		default:
			return "edit";
	}
}

export interface ModelInfo {
	id?: string;
	displayName?: string;
	display_name?: string;
	backend?: string;
	modelId?: string;
	model_id?: string;
	value?: string;
}

function getModelInfoModelId(model: ModelInfo): string {
	return model.modelId ?? model.model_id ?? model.value ?? model.id ?? "";
}

export function getModelInfoDisplayName(model: ModelInfo): string {
	return (
		model.displayName ??
		model.display_name ??
		model.value ??
		model.modelId ??
		model.model_id ??
		model.id ??
		""
	);
}

export function getModelInfoId(model: ModelInfo): string {
	if (model.id) return model.id;
	const modelId = getModelInfoModelId(model);
	return model.backend && modelId ? `${model.backend}:${modelId}` : modelId;
}

export function getModelInfoBackend(model: ModelInfo): string {
	return model.backend ?? "";
}

export function normalizeModelSelectionId(
	models: ModelInfo[],
	selected: string | null | undefined,
): string {
	if (!selected) return "";
	const exact = models.find((model) => getModelInfoId(model) === selected);
	if (exact) return getModelInfoId(exact);
	const byModelId = models.find(
		(model) => getModelInfoModelId(model) === selected,
	);
	return byModelId ? getModelInfoId(byModelId) : selected;
}

export interface SlashCommand {
	name: string;
	description: string;
	argumentHint?: string;
}

export interface AgentSkill {
	name: string;
	description: string;
	scope: "personal" | "project" | "system" | "admin";
}

export interface PermissionRequest {
	id: string;
	toolUseId?: string | null;
	toolName: string;
	kind?: "tool_approval" | "plan_approval" | "question" | "permission_grant";
	input?: Record<string, unknown> | null;
	plan?: string | null;
	allowedPrompts?: Array<{ tool: string; prompt: string }>;
	questions?: Array<{
		question: string;
		header?: string | null;
		options: Array<{ label: string; description?: string | null }>;
		multiSelect: boolean;
	}>;
	title?: string;
	displayName?: string;
	description?: string | null;
	decisionReason?: string | null;
}

export type MessageRole = "human" | "agent" | "system";

export type SessionState =
	| "active"
	| "idle"
	| "done"
	| "error"
	| "closed"
	| "archived";

export type ContextCarryState = "resumed" | "reinjected" | "failed";

export type TurnPhase = "idle" | "streaming" | "waiting_permission";

export interface AgentStallObservation {
	turnPhase: TurnPhase;
	idleSecs: number;
	signalCount: number;
	capReached: boolean;
}

interface WorkflowStepContext {
	runId: string;
	workflowName: string;
	stepName: string;
	runIndex: number;
	parentStepName?: string | null;
	parentRunIndex?: number | null;
	order: number;
}

export type MessagePart =
	| { type: "thinking"; content: string; parentToolUseId?: string }
	| { type: "text"; content: string; parentToolUseId?: string }
	| { type: "error"; content: string; parentToolUseId?: string }
	| {
			type: "tool_use";
			tool: string;
			input: Record<string, unknown>;
			id: string;
			parentToolUseId?: string;
	  }
	| {
			type: "tool_result";
			content: string;
			isError: boolean;
			toolUseId?: string;
			parentToolUseId?: string;
			contentRef?: ToolOutputRef;
			summary?: ToolOutputSummary;
	  }
	| {
			type: "permission";
			request: PermissionRequest;
			status: "pending" | "allowed" | "denied" | "cancelled";
			answers?: Record<string, string | string[]>;
			parentToolUseId?: string;
	  }
	| {
			type: "task_status";
			taskToolUseId: string;
			status:
				| "started"
				| "completed"
				| "failed"
				| "stopped"
				| "progress"
				| "backgrounded";
			description?: string;
			summary?: string;
	  }
	| {
			type: "todo_list_snapshot";
			items: Array<{ text: string; completed: boolean }>;
	  }
	| {
			type: "system_notification";
			notificationType: "compaction";
			status: "in_progress" | "completed" | "error";
			label: string;
			detail?: string;
			hookId?: string;
	  }
	| {
			type: "image";
			data: string;
			mediaType: string;
	  }
	| {
			type: "image_ref";
			attachment: AttachmentRef;
	  };

interface AttachmentRef {
	id: string;
	mediaType: string;
	byteSize: number;
}

export interface ToolOutputRef {
	id: string;
	byteSize: number;
}

export interface ToolOutputSummary {
	lineCount: number;
	byteSize: number;
	isError: boolean;
	truncated: boolean;
}

export interface SessionToolOutput {
	content: string;
	byteSize: number;
}

export type ActivityEntry =
	| {
			type: "tool_use";
			tool: string;
			input: Record<string, unknown>;
			id: string;
	  }
	| {
			type: "tool_result";
			content: string;
			isError: boolean;
			toolUseId?: string;
			contentRef?: ToolOutputRef;
			summary?: ToolOutputSummary;
	  }
	| {
			type: "permission_result";
			toolName: string;
			status: string;
			summary: string;
	  };

export interface ChatMessage {
	id: string;
	role: MessageRole;
	parts: MessagePart[];
	timestamp: number;
	mentions?: MentionReference[];
}

/** Rust backend ChatMessage format (for DB persistence) */
export interface LegacyChatMessage {
	id: string;
	role: MessageRole;
	content: string;
	thinking?: string;
	activities?: ActivityEntry[];
	timestamp: number;
	mentions?: MentionReference[];
}

export interface ChatSession {
	id: string;
	worktreePath: string;
	messages: ChatMessage[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	contextCarry?: ContextCarryState | null;
	permissionMode: PermissionMode;
	planMode?: PlanMode;
	permissionProfileId?: string | null;
	backendId?: string | null;
	workflowStepSession?: boolean;
	workflowStepContext?: WorkflowStepContext | null;
}

export function getTextContent(parts: MessagePart[]): string {
	return parts
		.filter((p): p is { type: "text"; content: string } => p.type === "text")
		.map((p) => p.content)
		.join("");
}

export interface SessionSummary {
	id: string;
	worktreePath: string;
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	firstMessage: string;
	messageCount: number;
	agentSessionId?: string | null;
	contextCarry?: ContextCarryState | null;
	permissionMode: PermissionMode;
	planMode?: PlanMode;
	permissionProfileId?: string | null;
	backendId?: string | null;
	workflowStepSession?: boolean;
	workflowStepContext?: WorkflowStepContext | null;
}

export interface BackendInfo {
	id: string;
	name: string;
	available: boolean;
	availableModels: ModelInfo[];
	capabilities?: {
		steering: boolean;
	};
}

export interface QueuedAgentTurn {
	id: string;
	contentPreview: string;
	createdAt: number;
	permissionMode: PermissionMode;
	imageCount: number;
}

export interface TokenUsage {
	inputTokens: number;
	outputTokens: number;
	totalTokens?: number;
	contextWindowTokens?: number;
}

/**
 * Rust の `agent_status::SessionStatus` に対応するステータス。
 * ChatSession 単位で Rust が算出する派生ステータスをそのまま消費する。
 */
export interface SessionStatus {
	chat_session_id: string;
	worktree_id: string;
	worktree_path: string;
	pty_id: string | null;
	agent_state: AgentState;
	turn_phase: TurnPhase;
	session_state: SessionState;
	pending_permission: boolean;
	pending_permission_request?: PermissionRequest | null;
	last_activity_at: number;
	workflow_step?: string | null;
	workflow_execution_state?: string | null;
}

/**
 * Rust の `agent_status::WorkspaceStatus` に対応する集約ステータス。
 * 1 worktree 配下の全 SessionStatus を集約した結果。
 */
export interface WorkspaceStatus {
	worktree_id: string;
	worktree_path: string;
	aggregated_state: AgentState;
	running_count: number;
	waiting_count: number;
	error_count: number;
	session_count: number;
	last_activity_at: number;
}

type ImagePart = Extract<MessagePart, { type: "image" }>;
type ImageRefPart = Extract<MessagePart, { type: "image_ref" }>;
export type DisplayImagePart = ImagePart | ImageRefPart;

export interface ImageAttachment {
	data: string;
	mediaType: string;
}

export interface AgentEditorContext {
	activeEditorPath?: string | null;
	openEditorPaths?: string[];
	selection?: AgentEditorSelection | null;
}

export interface AgentEditorSelection {
	filePath: string;
	startLine: number;
	endLine: number;
}

export interface MentionReference {
	filePath: string;
	startLine?: number;
	endLine?: number;
}
