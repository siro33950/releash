import { invoke } from "@tauri-apps/api/core";
import type {
	BackendInfo,
	ChatMessage,
	ChatSession,
	ImageAttachment,
	LegacyChatMessage,
	MentionReference,
	MessagePart,
	MessageRole,
	ModelInfo,
	PermissionMode,
	SessionState,
	SessionSummary,
	TurnPhase,
} from "@/types/session";

interface LegacyChatSession {
	id: string;
	worktreePath: string;
	messages: (LegacyChatMessage & { parts?: MessagePart[] })[];
	state: SessionState;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	permissionMode?: PermissionMode;
	backendId?: string | null;
	workflowStepSession?: boolean;
}

export function legacyToParts(msg: LegacyChatMessage): MessagePart[] {
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
						request_id: "",
						tool_name: a.toolName,
						input: {},
						tool_use_id: "",
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

function convertLegacyMessage(
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

function convertLegacySession(session: LegacyChatSession): ChatSession {
	return {
		...session,
		messages: session.messages.map(convertLegacyMessage),
		permissionMode: session.permissionMode ?? "acceptEdits",
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
	selectedModel: string | null;
	availableModels: ModelInfo[];
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
	permissionMode?: PermissionMode;
	backendId?: string | null;
	selectedModel?: string | null;
	availableModels?: ModelInfo[];
	workflowStepSession?: boolean;
	turnPhase: TurnPhase;
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
			permissionMode: raw.permissionMode,
			backendId: raw.backendId,
			workflowStepSession: raw.workflowStepSession,
		}),
		turnPhase: raw.turnPhase,
		selectedModel: raw.selectedModel ?? null,
		availableModels: raw.availableModels ?? [],
	};
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

export async function createSession(
	worktreePath: string,
	backendId?: string | null,
): Promise<ChatSession> {
	const raw = await invoke<LegacyChatSession>("create_session", {
		worktreePath,
		backendId: backendId ?? null,
	});
	return convertLegacySession(raw);
}

export async function closeSession(sessionId: string): Promise<void> {
	return invoke("close_session", { sessionId });
}

export interface RestoreSessionResponse {
	restoredWorkflowStep: boolean;
}

export async function restoreSession(
	sessionId: string,
): Promise<RestoreSessionResponse> {
	return invoke<RestoreSessionResponse>("restore_session", { sessionId });
}

export async function openWorkflowStepTab(
	chatSessionId: string,
): Promise<void> {
	return invoke("open_workflow_step_tab", { chatSessionId });
}

export async function listClosedSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	return invoke<SessionSummary[]>("list_closed_sessions", { worktreePath });
}

export async function addMessage(
	sessionId: string,
	role: MessageRole,
	content: string,
): Promise<ChatMessage> {
	const raw = await invoke<LegacyChatMessage>("add_message", {
		sessionId,
		role,
		content,
	});
	return convertLegacyMessage(raw);
}

interface RawSendMessageResponse {
	session: LegacyChatSession;
	humanMessage: LegacyChatMessage & { parts?: MessagePart[] };
	agentMessage: (LegacyChatMessage & { parts?: MessagePart[] }) | null;
	sessions: SessionSummary[];
}

export interface SendMessageResponse {
	session: ChatSession;
	humanMessage: ChatMessage;
	agentMessage: ChatMessage | null;
	sessions: SessionSummary[];
}

export async function sendAgentMessage(
	chatSessionId: string | null,
	worktreePath: string,
	content: string,
	permissionMode: string,
	backendId?: string | null,
	images?: ImageAttachment[],
	mentions?: MentionReference[],
): Promise<SendMessageResponse> {
	const raw = await invoke<RawSendMessageResponse>("send_agent_message", {
		chatSessionId,
		worktreePath,
		content,
		permissionMode,
		backendId: backendId ?? null,
		images: images && images.length > 0 ? images : undefined,
		mentions: mentions && mentions.length > 0 ? mentions : undefined,
	});
	return {
		session: convertLegacySession(raw.session),
		humanMessage: convertLegacyMessage(raw.humanMessage),
		agentMessage: raw.agentMessage
			? convertLegacyMessage(raw.agentMessage)
			: null,
		sessions: raw.sessions,
	};
}

export async function sendWorkflowApprovalChatMessage(
	chatSessionId: string,
	worktreePath: string,
	content: string,
	permissionMode: string,
	images?: ImageAttachment[],
	mentions?: MentionReference[],
): Promise<SendMessageResponse> {
	const raw = await invoke<RawSendMessageResponse>(
		"send_workflow_approval_chat_message",
		{
			chatSessionId,
			worktreePath,
			content,
			permissionMode,
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
		sessions: raw.sessions,
	};
}

interface RawInitSessionsResponse {
	sessions: SessionSummary[];
	activeSession: RawGetSessionResponse | null;
}

export interface InitSessionsResponse {
	sessions: SessionSummary[];
	activeSession: GetSessionResponse | null;
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
	return { sessions: raw.sessions, activeSession };
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

export async function updateSessionState(
	sessionId: string,
	newState: SessionState,
): Promise<void> {
	return invoke("update_session_state", { sessionId, newState });
}

export async function updateSessionAgentInfo(
	sessionId: string,
	agentSessionId: string | null,
): Promise<void> {
	return invoke("update_session_agent_info", {
		sessionId,
		agentSessionId,
	});
}

export interface BackendListResult {
	backends: BackendInfo[];
	defaultId: string | null;
}

export async function listAgentBackends(): Promise<BackendListResult> {
	return invoke<BackendListResult>("list_agent_backends");
}
