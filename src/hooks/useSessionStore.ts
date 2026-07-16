import { invoke } from "@tauri-apps/api/core";
import {
	type AgentEditorContext,
	type BackendInfo,
	type ChatMessage,
	type ChatSession,
	type ContextCarryState,
	type ImageAttachment,
	type LegacyChatMessage,
	type MentionReference,
	type MessagePart,
	type ModelInfo,
	normalizePermissionMode,
	type PermissionMode,
	type PermissionRequest,
	type QueuedAgentTurn,
	type SessionState,
	type SessionSummary,
	type SessionToolOutput,
	type TokenUsage,
	type TurnPhase,
} from "@/types/session";

export interface LegacyChatSession {
	id: string;
	worktreePath: string;
	messages: (LegacyChatMessage & { parts?: MessagePart[] })[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	contextCarry?: ContextCarryState | null;
	permissionMode: string;
	planMode?: boolean;
	permissionProfileId?: string | null;
	backendId?: string | null;
	workflowNodeSession?: boolean;
}

const INITIAL_SESSION_PAGE_LIMIT = 50;

function legacyToParts(msg: LegacyChatMessage): MessagePart[] {
	const parts: MessagePart[] = [];
	if (msg.thinking) {
		parts.push({ type: "thinking", content: msg.thinking });
	}
	if (msg.activities) {
		for (const a of msg.activities) {
			if (a.type === "tool_use") {
				parts.push({
					type: "tool_use",
					tool: a.tool,
					input: a.input,
					id: a.id,
				});
			} else if (a.type === "tool_result") {
				parts.push({
					type: "tool_result",
					content: a.content,
					isError: a.isError,
					...(a.toolUseId && { toolUseId: a.toolUseId }),
				});
			} else if (a.type === "permission_result") {
				parts.push({
					type: "permission",
					request: {
						id: "",
						toolName: a.toolName,
						kind: "tool_approval",
						input: {},
					},
					status: a.status === "allowed" ? "allowed" : "denied",
				});
			}
		}
	}
	if (msg.content) {
		parts.push({ type: "text", content: msg.content });
	}
	return parts;
}

export function convertLegacyMessage(
	msg: LegacyChatMessage & { parts?: MessagePart[] },
): ChatMessage {
	return {
		id: msg.id,
		role: msg.role,
		parts: msg.parts ?? legacyToParts(msg),
		timestamp: msg.timestamp,
		mentions: msg.mentions,
	};
}

export function convertLegacySession(session: LegacyChatSession): ChatSession {
	return {
		...session,
		messages: session.messages.map(convertLegacyMessage),
		permissionMode: normalizePermissionMode(session.permissionMode),
		...(session.planMode !== undefined ? { planMode: session.planMode } : {}),
		contextCarry: session.contextCarry ?? null,
		permissionProfileId: session.permissionProfileId ?? null,
		backendId: session.backendId ?? null,
	};
}

export async function listSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	return invoke<SessionSummary[]>("list_sessions", { worktreePath });
}

export interface GetSessionResponse {
	session: ChatSession;
	turnPhase: TurnPhase;
	selectedModel: string;
	availableModels: ModelInfo[];
	canChangeBackend: boolean;
	pendingQueue?: QueuedAgentTurn[];
	pendingQueueCount?: number;
	pendingPermissionRequest?: PermissionRequest | null;
	pendingPermissionStateRevision?: number | null;
	latestTokenUsage?: TokenUsage | null;
	initialPage?: {
		nextCursor: string | null;
		hasMore: boolean;
		totalCount: number;
	};
}

interface RawSessionPage {
	messages: (LegacyChatMessage & { parts?: MessagePart[] })[];
	messageMetadata?: MessagePageMetadata[];
	nextCursor?: string | null;
	hasMore: boolean;
	totalCount: number;
	latestTokenUsage?: TokenUsage | null;
}

interface MessagePageMetadata {
	messageId: string;
	tokenMeta?: unknown;
	runMeta?: unknown;
}

export interface GetSessionPageResponse {
	messages: ChatMessage[];
	messageMetadata: MessagePageMetadata[];
	nextCursor: string | null;
	hasMore: boolean;
	totalCount: number;
	latestTokenUsage: TokenUsage | null;
}

export interface LoadedMessagePage {
	requestCursor: string | null;
	count: number;
}

export interface AgentChatEvictionPlanRequest {
	active?: {
		sessionId: string;
		messageCount: number;
		oldestVisibleIndex: number;
		loadedPages: LoadedMessagePage[];
		turnPhase: TurnPhase;
	} | null;
	sessions?: Array<{
		sessionId: string;
		messageCount: number;
		evictionRank: number;
		protected: boolean;
		loading: boolean;
	}>;
}

export interface ActiveMessageEvictionPlan {
	sessionId: string;
	direction: "older";
	count: number;
	nextCursor: string | null;
	hasMore: boolean;
	loadedPages: LoadedMessagePage[];
}

export interface AgentChatEvictionPlan {
	active?: ActiveMessageEvictionPlan | null;
	evictSessionIds: string[];
}

interface RawGetSessionResponse {
	// Flattened from Rust GetSessionResponse (#[serde(flatten)])
	id: string;
	worktreePath: string;
	messages: (LegacyChatMessage & { parts?: MessagePart[] })[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	contextCarry?: ContextCarryState | null;
	permissionMode: string;
	planMode?: boolean;
	permissionProfileId?: string | null;
	backendId?: string | null;
	selectedModel: string;
	availableModels?: ModelInfo[];
	canChangeBackend?: boolean;
	pendingQueue?: QueuedAgentTurn[];
	pendingQueueCount?: number;
	pendingPermissionRequest?: PermissionRequest | null;
	pendingPermissionStateRevision?: number | null;
	latestTokenUsage?: TokenUsage | null;
	workflowNodeSession?: boolean;
	turnPhase: TurnPhase;
	initialPage?: {
		nextCursor?: string | null;
		hasMore: boolean;
		totalCount: number;
	};
}

function convertRawGetSessionResponse(
	raw: RawGetSessionResponse,
): GetSessionResponse {
	return {
		session: convertLegacySession({
			id: raw.id,
			worktreePath: raw.worktreePath,
			messages: raw.messages,
			state: raw.state,
			createdAt: raw.createdAt,
			updatedAt: raw.updatedAt,
			agentSessionId: raw.agentSessionId,
			contextCarry: raw.contextCarry,
			permissionMode: raw.permissionMode,
			planMode: raw.planMode,
			permissionProfileId: raw.permissionProfileId,
			backendId: raw.backendId,
			workflowNodeSession: raw.workflowNodeSession,
		}),
		turnPhase: raw.turnPhase,
		selectedModel: raw.selectedModel,
		availableModels: raw.availableModels ?? [],
		canChangeBackend: raw.canChangeBackend ?? false,
		pendingQueue: raw.pendingQueue ?? [],
		pendingQueueCount: raw.pendingQueueCount ?? 0,
		pendingPermissionRequest: raw.pendingPermissionRequest ?? null,
		pendingPermissionStateRevision: raw.pendingPermissionStateRevision ?? 0,
		latestTokenUsage: raw.latestTokenUsage ?? null,
		initialPage: raw.initialPage
			? {
					nextCursor: raw.initialPage.nextCursor ?? null,
					hasMore: raw.initialPage.hasMore,
					totalCount: raw.initialPage.totalCount,
				}
			: undefined,
	};
}

function convertRawSessionPage(raw: RawSessionPage): GetSessionPageResponse {
	return {
		messages: raw.messages.map(convertLegacyMessage),
		messageMetadata: raw.messageMetadata ?? [],
		nextCursor: raw.nextCursor ?? null,
		hasMore: raw.hasMore,
		totalCount: raw.totalCount,
		latestTokenUsage: raw.latestTokenUsage ?? null,
	};
}

export async function getSessionPage(
	sessionId: string,
	cursor: string | null = null,
	limit: number = INITIAL_SESSION_PAGE_LIMIT,
): Promise<GetSessionPageResponse | null> {
	const raw = await invoke<RawSessionPage | null>("get_session_page", {
		sessionId,
		cursor,
		limit,
	});
	return raw ? convertRawSessionPage(raw) : null;
}

export async function planAgentChatEviction(
	request: AgentChatEvictionPlanRequest,
): Promise<AgentChatEvictionPlan> {
	return invoke<AgentChatEvictionPlan>("plan_agent_chat_eviction", { request });
}

export async function getSession(
	sessionId: string,
): Promise<GetSessionResponse | null> {
	const raw = await invoke<RawGetSessionResponse | null>("get_session", {
		sessionId,
	});
	if (!raw) return null;
	return convertRawGetSessionResponse(raw);
}

export async function getSessionToolOutput(
	sessionId: string,
	toolOutputId: string,
): Promise<SessionToolOutput | null> {
	return invoke<SessionToolOutput | null>("get_session_tool_output", {
		sessionId,
		toolOutputId,
	});
}

export async function createSession(
	worktreePath: string,
	permissionMode: PermissionMode,
	backendId?: string | null,
	modelId?: string | null,
): Promise<ChatSession> {
	const raw = await invoke<LegacyChatSession>("create_session", {
		worktreePath,
		permissionMode,
		backendId: backendId ?? null,
		modelId: modelId ?? null,
	});
	return convertLegacySession(raw);
}

export async function createWorkspaceSession(
	requestId: string,
	worktreePath: string,
	permissionMode: PermissionMode,
	backendId?: string | null,
	modelId?: string | null,
): Promise<string> {
	return invoke<string>("create_workspace_session", {
		requestId,
		worktreePath,
		permissionMode,
		backendId: backendId ?? null,
		modelId: modelId ?? null,
	});
}

export async function closeSession(sessionId: string): Promise<void> {
	return invoke("close_session", { sessionId });
}

export async function archiveSession(sessionId: string): Promise<void> {
	return invoke("archive_session", { sessionId });
}

export async function archiveOpenSession(sessionId: string): Promise<void> {
	return invoke("archive_open_session", { sessionId });
}

export async function forkSession(sessionId: string): Promise<ChatSession> {
	const raw = await invoke<LegacyChatSession>("fork_session", { sessionId });
	return convertLegacySession(raw);
}

export async function setSessionTitle(
	sessionId: string,
	title: string | null,
): Promise<SessionSummary> {
	return invoke<SessionSummary>("set_session_title", {
		sessionId,
		title,
	});
}

export interface RestoreSessionResponse {
	restoredWorkflowNode: boolean;
}

export async function restoreSession(
	sessionId: string,
): Promise<RestoreSessionResponse> {
	return invoke<RestoreSessionResponse>("restore_session", { sessionId });
}

export async function listClosedSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	return invoke<SessionSummary[]>("list_closed_sessions", { worktreePath });
}

interface RawSendMessageResponse {
	session: LegacyChatSession;
	humanMessage: LegacyChatMessage & { parts?: MessagePart[] };
	agentMessage: (LegacyChatMessage & { parts?: MessagePart[] }) | null;
	queuedTurn?: QueuedAgentTurn | null;
	pendingQueue?: QueuedAgentTurn[];
	pendingQueueCount?: number;
	canChangeBackend?: boolean;
	sessions: SessionSummary[];
}

export interface SendMessageResponse {
	session: ChatSession;
	humanMessage: ChatMessage;
	agentMessage: ChatMessage | null;
	queuedTurn: QueuedAgentTurn | null;
	pendingQueue: QueuedAgentTurn[];
	pendingQueueCount: number;
	canChangeBackend: boolean;
	sessions: SessionSummary[];
}

export async function sendAgentMessage(
	chatSessionId: string | null,
	worktreePath: string,
	content: string,
	permissionMode: PermissionMode,
	planMode: boolean,
	backendId?: string | null,
	images?: ImageAttachment[],
	mentions?: MentionReference[],
	editorContext?: AgentEditorContext,
	modelId?: string | null,
): Promise<SendMessageResponse> {
	const args: {
		chatSessionId: string | null;
		worktreePath: string;
		content: string;
		permissionMode: PermissionMode;
		planMode: boolean;
		backendId: string | null;
		modelId: string | null;
		images?: ImageAttachment[];
		mentions?: MentionReference[];
		editorContext?: AgentEditorContext;
	} = {
		chatSessionId,
		worktreePath,
		content,
		permissionMode,
		planMode,
		backendId: backendId ?? null,
		modelId: modelId ?? null,
		images: images && images.length > 0 ? images : undefined,
		mentions: mentions && mentions.length > 0 ? mentions : undefined,
	};
	if (editorContext) {
		args.editorContext = editorContext;
	}
	const raw = await invoke<RawSendMessageResponse>("send_agent_message", args);
	return {
		session: convertLegacySession(raw.session),
		humanMessage: convertLegacyMessage(raw.humanMessage),
		agentMessage: raw.agentMessage
			? convertLegacyMessage(raw.agentMessage)
			: null,
		queuedTurn: raw.queuedTurn ?? null,
		pendingQueue: raw.pendingQueue ?? [],
		pendingQueueCount: raw.pendingQueueCount ?? 0,
		canChangeBackend: raw.canChangeBackend ?? false,
		sessions: raw.sessions,
	};
}

export async function sendWorkflowApprovalChatMessage(
	executionId: string,
	content: string,
	permissionMode: PermissionMode,
	planMode: boolean,
	images?: ImageAttachment[],
	mentions?: MentionReference[],
): Promise<SendMessageResponse> {
	const raw = await invoke<RawSendMessageResponse>(
		"send_workflow_approval_chat_message",
		{
			executionId,
			content,
			permissionMode,
			planMode,
			images: images && images.length > 0 ? images : undefined,
			mentions: mentions && mentions.length > 0 ? mentions : undefined,
		},
	);
	return {
		session: convertLegacySession(raw.session),
		humanMessage: convertLegacyMessage(raw.humanMessage),
		agentMessage: raw.agentMessage
			? convertLegacyMessage(raw.agentMessage)
			: null,
		queuedTurn: raw.queuedTurn ?? null,
		pendingQueue: raw.pendingQueue ?? [],
		pendingQueueCount: raw.pendingQueueCount ?? 0,
		canChangeBackend: raw.canChangeBackend ?? false,
		sessions: raw.sessions,
	};
}

export interface CancelQueuedTurnResponse {
	sessionId: string;
	canceledCount: number;
	pendingQueue: QueuedAgentTurn[];
	pendingQueueCount: number;
}

export async function cancelAgentQueuedTurn(
	chatSessionId: string,
	queuedTurnId?: string | null,
): Promise<CancelQueuedTurnResponse> {
	return invoke<CancelQueuedTurnResponse>("cancel_agent_queued_turn", {
		chatSessionId,
		queuedTurnId: queuedTurnId ?? null,
	});
}

interface RawInitSessionsResponse {
	sessions: SessionSummary[];
	activeSession: RawGetSessionResponse | null;
	permissionMode?: string;
	planMode?: boolean;
}

export interface InitSessionsResponse {
	sessions: SessionSummary[];
	activeSession: GetSessionResponse | null;
	permissionMode: PermissionMode;
	planMode: boolean;
}

export async function initAgentSessions(
	worktreePath: string,
): Promise<InitSessionsResponse> {
	const raw = await invoke<RawInitSessionsResponse>("init_agent_sessions", {
		worktreePath,
	});
	const activeSession = raw.activeSession
		? convertRawGetSessionResponse(raw.activeSession)
		: null;
	return {
		sessions: raw.sessions,
		activeSession,
		permissionMode: normalizePermissionMode(raw.permissionMode),
		planMode: raw.planMode ?? false,
	};
}

export async function setSessionBackend(
	chatSessionId: string,
	backendId: string,
): Promise<GetSessionResponse> {
	const raw = await invoke<RawGetSessionResponse>("set_session_backend", {
		chatSessionId,
		backendId,
	});
	return convertRawGetSessionResponse(raw);
}

export interface BackendListResult {
	backends: BackendInfo[];
	defaultId: string | null;
}

export async function listAgentBackends(): Promise<BackendListResult> {
	return invoke<BackendListResult>("list_agent_backends");
}
