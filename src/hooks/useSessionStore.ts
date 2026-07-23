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
	type TurnInterruption,
	type TurnPhase,
} from "@/types/session";

export interface LegacyChatSession {
	id: string;
	worktreePath: string;
	messages: (LegacyChatMessage & { parts?: MessagePart[] })[];
	state: SessionState;
	errorReason?: string | null;
	createdAt: number;
	updatedAt: number;
	agentSessionId?: string | null;
	contextCarry?: ContextCarryState | null;
	permissionMode: string;
	planMode?: boolean;
	permissionProfileId?: string | null;
	backendId?: string | null;
	workflowNodeSession?: boolean;
	sessionRevision?: string;
	activeTurnId?: string | null;
}

const INITIAL_SESSION_PAGE_LIMIT = 50;
const MAX_OPERATION_MIRROR_ENTRIES = 512;

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

export type AgentSessionNoticeOperation =
	| "send"
	| "load_session"
	| "load_older"
	| "cancel_queue"
	| "resume_queue"
	| "close_session"
	| "restore_session"
	| "archive_session"
	| "fork_session"
	| "set_title"
	| "respond_permission"
	| "set_backend";

export type AgentSessionNoticeUpdate =
	| {
			action: "failure";
			operation: AgentSessionNoticeOperation;
			message: string;
	  }
	| { action: "success"; operation: AgentSessionNoticeOperation }
	| { action: "dismiss" };

export interface AgentSessionNoticeSnapshot {
	sessionId: string;
	revision: string;
	notice: { message: string } | null;
}

export interface SafeOperationFailure {
	kind: string;
	retryable: boolean;
	label: string;
	detail: string | null;
	correlation_id: string;
}

export type SessionFeedbackAction = "dismiss" | "retry_resolution";

export interface SessionFeedbackActionIdentity {
	action: SessionFeedbackAction;
	action_id: string;
	origin_revision: string;
}

export interface SessionFeedbackEntry {
	feedback_id: string;
	attempt_id: string;
	session_id: string;
	operation: AgentSessionNoticeOperation;
	revision: string;
	actions: SessionFeedbackAction[];
	action_identities: SessionFeedbackActionIdentity[];
	failure: SafeOperationFailure;
}

export interface SessionFeedbackPage {
	entries: SessionFeedbackEntry[];
	next_cursor: string | null;
}

export type SessionFeedbackRetryOutcome =
	| { type: "resolved" }
	| { type: "failed"; entry: SessionFeedbackEntry };

export async function listAgentSessionFeedback(
	sessionId: string,
	limit = 32,
	cursor: string | null = null,
): Promise<SessionFeedbackPage> {
	return invoke<SessionFeedbackPage>("list_agent_session_feedback", {
		sessionId,
		limit,
		cursor,
	});
}

export async function dismissAgentSessionFeedback(
	sessionId: string,
	feedbackId: string,
	expectedRevision: string,
	actionId: string,
): Promise<void> {
	return invoke<void>("dismiss_agent_session_feedback", {
		sessionId,
		feedbackId,
		expectedRevision,
		actionId,
	});
}

export async function retryAgentSessionFeedback(
	sessionId: string,
	feedbackId: string,
	expectedRevision: string,
	actionId: string,
): Promise<SessionFeedbackRetryOutcome> {
	return invoke<SessionFeedbackRetryOutcome>("retry_agent_session_feedback", {
		sessionId,
		feedbackId,
		expectedRevision,
		actionId,
	});
}

export async function getAgentSessionNotice(
	sessionId: string,
): Promise<AgentSessionNoticeSnapshot> {
	return invoke<AgentSessionNoticeSnapshot>("get_agent_session_notice", {
		sessionId,
	});
}

export async function updateAgentSessionNotice(
	sessionId: string,
	update: AgentSessionNoticeUpdate,
): Promise<AgentSessionNoticeSnapshot> {
	return invoke<AgentSessionNoticeSnapshot>("update_agent_session_notice", {
		sessionId,
		update,
	});
}

export async function listSessions(
	worktreePath: string,
): Promise<SessionSummary[]> {
	const sessions = await invoke<RawSessionSummaryDtoV1[]>("list_sessions", {
		worktreePath,
	});
	return sessions.map(convertSessionSummaryDtoV1);
}

export interface GetSessionResponse {
	session: ChatSession;
	turnPhase: TurnPhase;
	selectedModel: string;
	availableModels: ModelInfo[];
	canChangeBackend: boolean;
	pendingQueue?: QueuedAgentTurn[];
	pendingQueueCount?: number;
	queuePaused: boolean;
	pendingPermissionRequest?: PermissionRequest | null;
	pendingPermissionStateRevision?: string | null;
	latestTokenUsage?: TokenUsage | null;
	initialPage?: {
		nextCursor: string | null;
		hasMore: boolean;
		totalCount: number;
	};
}

interface RawChatMessageDtoV1 {
	id: string;
	role: LegacyChatMessage["role"];
	content: string;
	thinking: string | null;
	activities: unknown[] | null;
	parts: Array<Record<string, unknown>> | null;
	streaming_final_seq: string;
	timestamp_ms: string;
	mentions: Array<{
		file_path: string;
		start_line: string | null;
		end_line: string | null;
	}> | null;
}

interface RawTokenUsageDtoV1 {
	input_tokens: string;
	output_tokens: string;
	total_tokens: string | null;
	context_window_tokens: string | null;
}

interface RawQueuedAgentTurnDtoV1 {
	id: string;
	content_preview: string;
	created_at_ms: string;
	permission_mode: string;
	image_count: string;
}

interface RawSessionSummaryDtoV1 {
	id: string;
	worktree_path: string;
	state: SessionState;
	error_reason: string | null;
	created_at_ms: string;
	updated_at_ms: string;
	first_message: string;
	message_count: string;
	agent_session_id: string | null;
	context_carry: ContextCarryState | null;
	permission_mode: string;
	plan_mode: boolean;
	permission_profile_id: string | null;
	backend_id: string | null;
	workflow_node_session: boolean;
	workflow_node_context: Record<string, unknown> | null;
}

function publicInteger(value: string): number {
	if (!/^(0|[1-9][0-9]*)$/.test(value)) {
		throw new Error("backend returned a non-canonical public integer");
	}
	return Number(value);
}

function nullableField<T>(value: T | null): T | undefined {
	return value === null ? undefined : value;
}

function convertPermissionRequestDtoV1(
	request: Record<string, unknown>,
): PermissionRequest {
	const toolUseId = nullableField(request.tool_use_id as string | null);
	const input = nullableField(request.input as Record<string, unknown> | null);
	const plan = nullableField(request.plan as string | null);
	const allowedPrompts = (
		request.allowed_prompts as Array<Record<string, unknown>>
	).map((item) => ({ tool: String(item.tool), prompt: String(item.prompt) }));
	const questions = (request.questions as Array<Record<string, unknown>>).map(
		(question) => ({
			question: String(question.question),
			header: (question.header as string | null) ?? null,
			options: (question.options as Array<Record<string, unknown>>).map(
				(option) => ({
					label: String(option.label),
					description: (option.description as string | null) ?? null,
				}),
			),
			multiSelect: Boolean(question.multi_select),
		}),
	);
	const title = nullableField(request.title as string | null);
	const displayName = nullableField(request.display_name as string | null);
	const description = nullableField(request.description as string | null);
	const decisionReason = nullableField(
		request.decision_reason as string | null,
	);
	return {
		id: String(request.id),
		toolName: String(request.tool_name),
		kind: request.kind as PermissionRequest["kind"],
		...(toolUseId === undefined ? {} : { toolUseId }),
		...(input === undefined ? {} : { input }),
		...(plan === undefined ? {} : { plan }),
		...(allowedPrompts.length === 0 ? {} : { allowedPrompts }),
		...(questions.length === 0 ? {} : { questions }),
		...(title === undefined ? {} : { title }),
		...(displayName === undefined ? {} : { displayName }),
		...(description === undefined ? {} : { description }),
		...(decisionReason === undefined ? {} : { decisionReason }),
	};
}

function convertMessagePartDtoV1(part: Record<string, unknown>): MessagePart {
	const parentToolUseId = nullableField(
		part.parent_tool_use_id as string | null,
	);
	switch (part.type) {
		case "thinking":
		case "text":
		case "error":
			return {
				type: part.type,
				content: String(part.content),
				parentToolUseId,
			};
		case "tool_use":
			return {
				type: "tool_use",
				tool: String(part.tool),
				input: part.input as Record<string, unknown>,
				id: String(part.id),
				parentToolUseId,
			};
		case "tool_result": {
			const contentRef = part.content_ref as Record<string, unknown> | null;
			const summary = part.summary as Record<string, unknown> | null;
			return {
				type: "tool_result",
				content: String(part.content),
				isError: Boolean(part.is_error),
				toolUseId: nullableField(part.tool_use_id as string | null),
				parentToolUseId,
				contentRef: contentRef
					? {
							id: String(contentRef.id),
							byteSize: publicInteger(String(contentRef.byte_size)),
						}
					: undefined,
				summary: summary
					? {
							lineCount: publicInteger(String(summary.line_count)),
							byteSize: publicInteger(String(summary.byte_size)),
							isError: Boolean(summary.is_error),
							truncated: Boolean(summary.truncated),
						}
					: undefined,
			};
		}
		case "permission":
			return {
				type: "permission",
				request: convertPermissionRequestDtoV1(
					part.request as Record<string, unknown>,
				),
				status: part.status as "pending" | "allowed" | "denied" | "cancelled",
				answers: nullableField(
					part.answers as Record<string, string | string[]> | null,
				),
				parentToolUseId,
			};
		case "task_status":
			return {
				type: "task_status",
				taskToolUseId: String(part.task_tool_use_id),
				status: part.status as Extract<
					MessagePart,
					{ type: "task_status" }
				>["status"],
				description: nullableField(part.description as string | null),
				summary: nullableField(part.summary as string | null),
			};
		case "todo_list_snapshot":
			return {
				type: "todo_list_snapshot",
				items: part.items as Array<{ text: string; completed: boolean }>,
			};
		case "system_notification":
			return {
				type: "system_notification",
				notificationType: part.notification_type as
					| "compaction"
					| "session_recovery",
				status: part.status as Extract<
					MessagePart,
					{ type: "system_notification" }
				>["status"],
				label: String(part.label),
				detail: nullableField(part.detail as string | null),
				hookId: nullableField(part.hook_id as string | null),
			};
		case "image":
			return {
				type: "image",
				data: String(part.data),
				mediaType: String(part.media_type),
			};
		case "image_ref": {
			const attachment = part.attachment as Record<string, unknown>;
			return {
				type: "image_ref",
				attachment: {
					id: String(attachment.id),
					mediaType: String(attachment.media_type),
					byteSize: publicInteger(String(attachment.byte_size)),
				},
			};
		}
		default:
			throw new Error("backend returned an unknown public message part");
	}
}

function convertChatMessageDtoV1(raw: RawChatMessageDtoV1): ChatMessage {
	return {
		id: raw.id,
		role: raw.role,
		parts: raw.parts
			? raw.parts.map(convertMessagePartDtoV1)
			: legacyToParts({
					id: raw.id,
					role: raw.role,
					content: raw.content,
					thinking: raw.thinking ?? undefined,
					timestamp: publicInteger(raw.timestamp_ms) / 1000,
				}),
		timestamp: publicInteger(raw.timestamp_ms) / 1000,
		mentions: raw.mentions?.map((mention) => ({
			filePath: mention.file_path,
			startLine: mention.start_line
				? publicInteger(mention.start_line)
				: undefined,
			endLine: mention.end_line ? publicInteger(mention.end_line) : undefined,
		})),
	};
}

function convertTokenUsageDtoV1(
	value: RawTokenUsageDtoV1 | null,
): TokenUsage | null {
	return value
		? {
				inputTokens: publicInteger(value.input_tokens),
				outputTokens: publicInteger(value.output_tokens),
				totalTokens: value.total_tokens
					? publicInteger(value.total_tokens)
					: undefined,
				contextWindowTokens: value.context_window_tokens
					? publicInteger(value.context_window_tokens)
					: undefined,
			}
		: null;
}

function convertQueuedTurnDtoV1(
	value: RawQueuedAgentTurnDtoV1,
): QueuedAgentTurn {
	return {
		id: value.id,
		contentPreview: value.content_preview,
		createdAt: publicInteger(value.created_at_ms) / 1000,
		permissionMode: normalizePermissionMode(value.permission_mode),
		imageCount: publicInteger(value.image_count),
	};
}

function convertSessionSummaryDtoV1(
	value: RawSessionSummaryDtoV1,
): SessionSummary {
	return {
		id: value.id,
		worktreePath: value.worktree_path,
		state: value.state,
		errorReason: value.error_reason,
		createdAt: publicInteger(value.created_at_ms) / 1000,
		updatedAt: publicInteger(value.updated_at_ms) / 1000,
		firstMessage: value.first_message,
		messageCount: publicInteger(value.message_count),
		agentSessionId: value.agent_session_id,
		contextCarry: value.context_carry,
		permissionMode: normalizePermissionMode(value.permission_mode),
		planMode: value.plan_mode,
		permissionProfileId: value.permission_profile_id,
		backendId: value.backend_id,
		workflowNodeSession: value.workflow_node_session,
		workflowNodeContext: value.workflow_node_context as never,
	};
}

interface RawSessionPage {
	messages: RawChatMessageDtoV1[];
	message_metadata: Array<{
		message_id: string;
		token_meta: unknown | null;
		run_meta: unknown | null;
	}>;
	next_cursor: string | null;
	has_more: boolean;
	total_count: string;
	latest_token_usage: RawTokenUsageDtoV1 | null;
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
	id: string;
	worktree_path: string;
	messages: RawChatMessageDtoV1[];
	state: SessionState;
	error_reason: string | null;
	created_at_ms: string;
	updated_at_ms: string;
	agent_session_id: string | null;
	context_carry: ContextCarryState | null;
	permission_mode: string;
	plan_mode: boolean;
	permission_profile_id: string | null;
	backend_id: string | null;
	selected_model: string | null;
	available_models: ModelInfo[];
	can_change_backend: boolean;
	pending_queue: RawQueuedAgentTurnDtoV1[];
	pending_queue_count: string;
	queue_paused: boolean;
	pending_permission_request: Record<string, unknown> | null;
	pending_permission_state_revision: string;
	latest_token_usage: RawTokenUsageDtoV1 | null;
	workflow_node_session: boolean;
	workflow_node_context: Record<string, unknown> | null;
	session_revision: string;
	active_turn_id: string | null;
	turn_phase: TurnPhase;
	last_turn_interruption: {
		message_id: string;
		reason: TurnInterruption["reason"];
	} | null;
	initial_page: {
		next_cursor: string | null;
		has_more: boolean;
		total_count: string;
	} | null;
}

function convertRawGetSessionResponse(
	raw: RawGetSessionResponse,
): GetSessionResponse {
	return {
		session: {
			id: raw.id,
			worktreePath: raw.worktree_path,
			messages: raw.messages.map(convertChatMessageDtoV1),
			state: raw.state,
			errorReason: raw.error_reason,
			createdAt: publicInteger(raw.created_at_ms) / 1000,
			updatedAt: publicInteger(raw.updated_at_ms) / 1000,
			agentSessionId: raw.agent_session_id,
			contextCarry: raw.context_carry,
			permissionMode: normalizePermissionMode(raw.permission_mode),
			planMode: raw.plan_mode,
			permissionProfileId: raw.permission_profile_id,
			backendId: raw.backend_id,
			workflowNodeSession: raw.workflow_node_session,
			workflowNodeContext: raw.workflow_node_context as never,
			sessionRevision: raw.session_revision,
			activeTurnId: raw.active_turn_id,
			lastTurnInterruption: raw.last_turn_interruption
				? {
						messageId: raw.last_turn_interruption.message_id,
						reason: raw.last_turn_interruption.reason,
					}
				: null,
		},
		turnPhase: raw.turn_phase,
		selectedModel: raw.selected_model ?? "",
		availableModels: raw.available_models,
		canChangeBackend: raw.can_change_backend,
		pendingQueue: raw.pending_queue.map(convertQueuedTurnDtoV1),
		pendingQueueCount: publicInteger(raw.pending_queue_count),
		queuePaused: raw.queue_paused,
		pendingPermissionRequest: raw.pending_permission_request
			? convertPermissionRequestDtoV1(raw.pending_permission_request)
			: null,
		pendingPermissionStateRevision: raw.pending_permission_state_revision,
		latestTokenUsage: convertTokenUsageDtoV1(raw.latest_token_usage),
		initialPage: raw.initial_page
			? {
					nextCursor: raw.initial_page.next_cursor,
					hasMore: raw.initial_page.has_more,
					totalCount: publicInteger(raw.initial_page.total_count),
				}
			: undefined,
	};
}

function convertRawSessionPage(raw: RawSessionPage): GetSessionPageResponse {
	return {
		messages: raw.messages.map(convertChatMessageDtoV1),
		messageMetadata: raw.message_metadata.map((metadata) => ({
			messageId: metadata.message_id,
			tokenMeta: metadata.token_meta ?? undefined,
			runMeta: metadata.run_meta ?? undefined,
		})),
		nextCursor: raw.next_cursor,
		hasMore: raw.has_more,
		totalCount: publicInteger(raw.total_count),
		latestTokenUsage: convertTokenUsageDtoV1(raw.latest_token_usage),
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

// A failed/reply-lost load reuses the exact caller attempt for this renderer
// process. A fresh renderer process intentionally creates a new identity:
// startup recovery has already turned the prior process' abandoned slot into
// canonical outcome-unknown feedback.
const sessionLoadAttempts = new Map<string, string>();

export async function getSession(
	sessionId: string,
): Promise<GetSessionResponse | null> {
	const attemptId =
		sessionLoadAttempts.get(sessionId) ?? `load-${crypto.randomUUID()}`;
	sessionLoadAttempts.set(sessionId, attemptId);
	const raw = await invoke<RawGetSessionResponse | null>("get_session", {
		sessionId,
		attemptId,
	});
	if (sessionLoadAttempts.get(sessionId) === attemptId) {
		sessionLoadAttempts.delete(sessionId);
	}
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
	const raw = await invoke<RawChatSessionDtoV1>("create_session", {
		worktreePath,
		permissionMode,
		backendId: backendId ?? null,
		modelId: modelId ?? null,
	});
	return convertChatSessionDtoV1(raw);
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

type SessionLifecycleAction =
	| { type: "close" }
	| { type: "archive_open" }
	| { type: "archive_closed" }
	| { type: "switch_backend"; backend_id: string };

const lifecycleAttempts = loadStringMap(
	"releash.session-lifecycle-attempts.v1",
);

async function requestSessionLifecycle(
	sessionId: string,
	action: SessionLifecycleAction,
): Promise<void> {
	const current = await getSession(sessionId);
	if (!current?.session.sessionRevision) {
		throw new Error("Session lifecycle revision is unavailable");
	}
	const attemptKey = JSON.stringify({
		sessionId,
		action,
		expectedSessionRevision: current.session.sessionRevision,
	});
	const requestId =
		lifecycleAttempts.get(attemptKey) ?? `lifecycle-${crypto.randomUUID()}`;
	lifecycleAttempts.set(attemptKey, requestId);
	saveStringMap("releash.session-lifecycle-attempts.v1", lifecycleAttempts);
	const args = {
		request: {
			request_id: requestId,
			session_id: sessionId,
			expected_session_revision: current.session.sessionRevision,
			action,
		},
	};
	type LifecycleState = { type: string };
	type LifecycleResult = {
		type: string;
		receipt?: { operation_id: string };
		state?: LifecycleState;
	};
	let result: LifecycleResult;
	try {
		result = await invoke<LifecycleResult>("request_session_lifecycle", args);
	} catch {
		// The exact caller identity and immutable request are safe to replay;
		// querying requires the backend-issued operation id, which a lost first
		// response did not reveal.
		result = await invoke<LifecycleResult>("request_session_lifecycle", args);
	}
	if (result.type !== "accepted") {
		if (result.type !== "outcome_unknown") {
			lifecycleAttempts.delete(attemptKey);
			saveStringMap("releash.session-lifecycle-attempts.v1", lifecycleAttempts);
		}
		throw new Error(`Session lifecycle was not accepted: ${result.type}`);
	}
	let state = result.state;
	const operationId = result.receipt?.operation_id;
	for (let poll = 0; state?.type === "accepted" && poll < 200; poll += 1) {
		if (!operationId) break;
		await new Promise((resolve) => setTimeout(resolve, 50));
		[, state] = await invoke<[unknown, LifecycleState]>(
			"get_session_lifecycle_operation",
			{ operationId },
		);
	}
	if (state?.type !== "completed") {
		throw new Error(
			`Session lifecycle requires reconciliation: ${state?.type ?? "missing_state"}`,
		);
	}
	lifecycleAttempts.delete(attemptKey);
	saveStringMap("releash.session-lifecycle-attempts.v1", lifecycleAttempts);
}

export async function redispatchPendingLifecycleAttempts(
	requestIds: ReadonlySet<string>,
): Promise<void> {
	for (const [attemptKey, requestId] of [...lifecycleAttempts]) {
		if (!requestIds.has(requestId)) continue;
		let snapshot: {
			sessionId: string;
			action: SessionLifecycleAction;
			expectedSessionRevision: string;
		};
		try {
			snapshot = JSON.parse(attemptKey) as typeof snapshot;
		} catch {
			// Older entries did not retain the exact revision and cannot be
			// replayed safely. The backend attempt remains visible for manual
			// reconciliation instead of being guessed from current state.
			continue;
		}
		const result = await invoke<{ type: string; state?: { type: string } }>(
			"request_session_lifecycle",
			{
				request: {
					request_id: requestId,
					session_id: snapshot.sessionId,
					expected_session_revision: snapshot.expectedSessionRevision,
					action: snapshot.action,
				},
			},
		);
		if (
			result.type === "rejected" ||
			(result.type === "accepted" && result.state?.type === "completed")
		) {
			lifecycleAttempts.delete(attemptKey);
		}
	}
	saveStringMap("releash.session-lifecycle-attempts.v1", lifecycleAttempts);
}

export async function closeSession(sessionId: string): Promise<void> {
	return requestSessionLifecycle(sessionId, { type: "close" });
}

export async function archiveSession(sessionId: string): Promise<void> {
	return requestSessionLifecycle(sessionId, { type: "archive_closed" });
}

export async function archiveOpenSession(sessionId: string): Promise<void> {
	return requestSessionLifecycle(sessionId, { type: "archive_open" });
}

export async function forkSession(sessionId: string): Promise<ChatSession> {
	const raw = await invoke<RawChatSessionDtoV1>("fork_session", { sessionId });
	return convertChatSessionDtoV1(raw);
}

export async function setSessionTitle(
	sessionId: string,
	title: string | null,
): Promise<SessionSummary> {
	const summary = await invoke<RawSessionSummaryDtoV1>("set_session_title", {
		sessionId,
		title,
	});
	return convertSessionSummaryDtoV1(summary);
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
	const sessions = await invoke<RawSessionSummaryDtoV1[]>(
		"list_closed_sessions",
		{ worktreePath },
	);
	return sessions.map(convertSessionSummaryDtoV1);
}

interface RawChatSessionDtoV1 {
	id: string;
	worktree_path: string;
	messages: RawChatMessageDtoV1[];
	state: SessionState;
	error_reason: string | null;
	created_at_ms: string;
	updated_at_ms: string;
	agent_session_id: string | null;
	context_carry: ContextCarryState | null;
	permission_mode: string;
	plan_mode: boolean;
	selected_model: string | null;
	permission_profile_id: string | null;
	backend_id: string | null;
	workflow_node_session: boolean;
	workflow_node_context: Record<string, unknown> | null;
}

function convertChatSessionDtoV1(raw: RawChatSessionDtoV1): ChatSession {
	return {
		id: raw.id,
		worktreePath: raw.worktree_path,
		messages: raw.messages.map(convertChatMessageDtoV1),
		state: raw.state,
		errorReason: raw.error_reason,
		createdAt: publicInteger(raw.created_at_ms) / 1000,
		updatedAt: publicInteger(raw.updated_at_ms) / 1000,
		agentSessionId: raw.agent_session_id,
		contextCarry: raw.context_carry,
		permissionMode: normalizePermissionMode(raw.permission_mode),
		planMode: raw.plan_mode,
		permissionProfileId: raw.permission_profile_id,
		backendId: raw.backend_id,
		workflowNodeSession: raw.workflow_node_session,
		workflowNodeContext: raw.workflow_node_context as never,
	};
}

export interface SendOperationView {
	receipt: {
		operation_id: string;
		session_id: string;
		input_ref: string;
		disposition:
			| { type: "started_turn"; turn_id: string }
			| { type: "queued"; queue_item_id: string };
	};
	latest_status: { type: string; [key: string]: unknown };
}

export type DurableSendCommandResult =
	| { type: "accepted"; operation: SendOperationView }
	| { type: "rejected_before_commit"; failure: unknown }
	| { type: "outcome_unknown"; operation_id: string };

const SEND_ATTEMPTS_STORAGE_KEY = "releash.agent-send-attempts.v1";
const ACCEPTED_SEND_STORAGE_KEY = "releash.accepted-send-operations.v1";

function loadStringMap(key: string): Map<string, string> {
	try {
		const value = globalThis.localStorage?.getItem(key);
		return new Map(value ? (JSON.parse(value) as [string, string][]) : []);
	} catch {
		return new Map();
	}
}

function loadBoundedStringMap(key: string): Map<string, string> {
	const mirror = loadStringMap(key);
	if (pruneStringMap(mirror)) {
		saveStringMap(key, mirror);
	}
	return mirror;
}

function pruneStringMap(value: Map<string, string>): boolean {
	let pruned = false;
	while (value.size > MAX_OPERATION_MIRROR_ENTRIES) {
		const oldestKey = value.keys().next().value;
		if (oldestKey === undefined) break;
		value.delete(oldestKey);
		pruned = true;
	}
	return pruned;
}

function saveStringMap(key: string, value: Map<string, string>): void {
	try {
		globalThis.localStorage?.setItem(key, JSON.stringify([...value]));
	} catch {
		// UI persistence is only a mirror; the backend caller journal remains authoritative.
	}
}

function saveBoundedStringMap(key: string, value: Map<string, string>): void {
	pruneStringMap(value);
	saveStringMap(key, value);
}

const sendAttempts = loadStringMap(SEND_ATTEMPTS_STORAGE_KEY);
const acceptedSendOperations = loadBoundedStringMap(ACCEPTED_SEND_STORAGE_KEY);

interface DirectSendAttemptSnapshot {
	type: "direct";
	chatSessionId: string | null;
	worktreePath: string;
	content: string;
	permissionMode: PermissionMode;
	planMode: boolean;
	backendId: string | null;
	modelId: string | null;
	images: ImageAttachment[];
	mentions: MentionReference[];
	editorContext: AgentEditorContext | null;
}

interface WorkflowApprovalSendAttemptSnapshot {
	type: "workflow_approval";
	executionId: string;
	content: string;
	permissionMode: PermissionMode;
	planMode: boolean;
	images: ImageAttachment[];
	mentions: MentionReference[];
}

type DurableSendAttemptSnapshot =
	| DirectSendAttemptSnapshot
	| WorkflowApprovalSendAttemptSnapshot;

function invokeDurableSendAttempt(
	snapshot: DurableSendAttemptSnapshot,
	operationId: string,
): Promise<DurableSendCommandResult> {
	if (snapshot.type === "workflow_approval") {
		return invoke<DurableSendCommandResult>(
			"send_workflow_approval_chat_message",
			{
				operationId,
				executionId: snapshot.executionId,
				content: snapshot.content,
				permissionMode: snapshot.permissionMode,
				planMode: snapshot.planMode,
				images: snapshot.images.length > 0 ? snapshot.images : undefined,
				mentions: snapshot.mentions.length > 0 ? snapshot.mentions : undefined,
			},
		);
	}
	return invoke<DurableSendCommandResult>("send_agent_message", {
		operationId,
		chatSessionId: snapshot.chatSessionId,
		worktreePath: snapshot.worktreePath,
		content: snapshot.content,
		permissionMode: snapshot.permissionMode,
		planMode: snapshot.planMode,
		backendId: snapshot.backendId,
		modelId: snapshot.modelId,
		images: snapshot.images.length > 0 ? snapshot.images : undefined,
		mentions: snapshot.mentions.length > 0 ? snapshot.mentions : undefined,
		editorContext: snapshot.editorContext ?? undefined,
	});
}

async function sendDurableAttempt(
	snapshot: DurableSendAttemptSnapshot,
): Promise<DurableSendCommandResult> {
	const attemptKey = JSON.stringify(snapshot);
	const operationId =
		sendAttempts.get(attemptKey) ?? `send-${crypto.randomUUID()}`;
	sendAttempts.set(attemptKey, operationId);
	saveStringMap(SEND_ATTEMPTS_STORAGE_KEY, sendAttempts);
	let result: DurableSendCommandResult;
	try {
		result = await invokeDurableSendAttempt(snapshot, operationId);
	} catch (error) {
		try {
			const operation = await invoke<SendOperationView>(
				"get_agent_send_operation",
				{ operationId },
			);
			result = { type: "accepted", operation };
		} catch {
			throw error;
		}
	}
	if (result.type === "rejected_before_commit") {
		sendAttempts.delete(attemptKey);
		saveStringMap(SEND_ATTEMPTS_STORAGE_KEY, sendAttempts);
		throw new Error("Send was rejected before its durable acceptance commit");
	}
	if (result.type === "outcome_unknown") {
		const unknownOperationId = result.operation_id;
		try {
			const operation = await invoke<SendOperationView>(
				"get_agent_send_operation",
				{ operationId: unknownOperationId },
			);
			result = { type: "accepted", operation };
		} catch {
			throw new Error(
				`Send acceptance is unknown; retry operation ${unknownOperationId}`,
			);
		}
	}
	sendAttempts.delete(attemptKey);
	saveStringMap(SEND_ATTEMPTS_STORAGE_KEY, sendAttempts);
	acceptedSendOperations.set(
		result.operation.receipt.session_id,
		result.operation.receipt.operation_id,
	);
	saveBoundedStringMap(ACCEPTED_SEND_STORAGE_KEY, acceptedSendOperations);
	return result;
}

export async function getAcceptedSendOperation(
	sessionId: string,
): Promise<SendOperationView | null> {
	const operationId = acceptedSendOperations.get(sessionId);
	if (!operationId) return null;
	return invoke<SendOperationView>("get_agent_send_operation", { operationId });
}

/**
 * Re-dispatch only attempts whose durable caller journal still says Pending.
 * The exact immutable snapshot and caller id come from the persisted renderer
 * outbox; this never invents a new id and is not an Accepted-operation retry.
 */
export async function redispatchPendingSendAttempts(
	operationIds: ReadonlySet<string>,
): Promise<void> {
	for (const [attemptKey, operationId] of [...sendAttempts]) {
		if (!operationIds.has(operationId)) continue;
		let snapshot: DurableSendAttemptSnapshot;
		try {
			snapshot = JSON.parse(attemptKey) as DurableSendAttemptSnapshot;
			if (snapshot.type !== "direct" && snapshot.type !== "workflow_approval") {
				continue;
			}
		} catch {
			// An old local mirror without the exact immutable command cannot be
			// reconstructed. Keep the backend Pending attempt visible for manual
			// reconciliation instead of inventing a replacement payload or id.
			continue;
		}
		const result = await invokeDurableSendAttempt(snapshot, operationId);
		if (result.type === "accepted") {
			sendAttempts.delete(attemptKey);
			acceptedSendOperations.set(
				result.operation.receipt.session_id,
				result.operation.receipt.operation_id,
			);
		} else if (result.type === "rejected_before_commit") {
			sendAttempts.delete(attemptKey);
		}
	}
	saveStringMap(SEND_ATTEMPTS_STORAGE_KEY, sendAttempts);
	saveBoundedStringMap(ACCEPTED_SEND_STORAGE_KEY, acceptedSendOperations);
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
): Promise<DurableSendCommandResult> {
	return sendDurableAttempt({
		type: "direct",
		chatSessionId,
		worktreePath,
		content,
		permissionMode,
		planMode,
		backendId: backendId ?? null,
		modelId: modelId ?? null,
		images: images ?? [],
		mentions: mentions ?? [],
		editorContext: editorContext ?? null,
	});
}

export interface PermissionResponseOperationView {
	receipt: {
		operation_id: string;
		session_id: string;
		request_id: string;
		input_ref: string;
	};
	latest_status: { type: string; [key: string]: unknown };
}

export type DurablePermissionResponseResult =
	| { type: "accepted"; operation: PermissionResponseOperationView }
	| { type: "rejected_before_commit"; failure: unknown }
	| { type: "outcome_unknown"; operation_id: string };

type AcceptedPermissionResponseResult = Extract<
	DurablePermissionResponseResult,
	{ type: "accepted" }
>;

interface PermissionResponseAttemptSnapshot {
	type: "permission_response";
	sessionId: string;
	requestId: string;
	behavior: "allow" | "deny";
	message: string | null;
	updatedInput: string | null;
}

const PERMISSION_RESPONSE_ATTEMPTS_STORAGE_KEY =
	"releash.agent-permission-response-attempts.v1";
const ACCEPTED_PERMISSION_RESPONSE_STORAGE_KEY =
	"releash.accepted-permission-response-operations.v1";
const permissionResponseAttempts = loadStringMap(
	PERMISSION_RESPONSE_ATTEMPTS_STORAGE_KEY,
);
const acceptedPermissionResponseOperations = loadBoundedStringMap(
	ACCEPTED_PERMISSION_RESPONSE_STORAGE_KEY,
);

function permissionResponseTargetKey(
	sessionId: string,
	requestId: string,
): string {
	return JSON.stringify([sessionId, requestId]);
}

function invokeDurablePermissionResponseAttempt(
	snapshot: PermissionResponseAttemptSnapshot,
	operationId: string,
): Promise<DurablePermissionResponseResult> {
	return invoke<DurablePermissionResponseResult>("respond_agent_permission", {
		operationId,
		chatSessionId: snapshot.sessionId,
		requestId: snapshot.requestId,
		behavior: snapshot.behavior,
		message: snapshot.message,
		updatedInput: snapshot.updatedInput,
	});
}

function rememberAcceptedPermissionResponse(
	operation: PermissionResponseOperationView,
): void {
	acceptedPermissionResponseOperations.set(
		permissionResponseTargetKey(
			operation.receipt.session_id,
			operation.receipt.request_id,
		),
		operation.receipt.operation_id,
	);
	saveBoundedStringMap(
		ACCEPTED_PERMISSION_RESPONSE_STORAGE_KEY,
		acceptedPermissionResponseOperations,
	);
}

export async function getAcceptedPermissionResponseOperation(
	sessionId: string,
	requestId: string,
): Promise<PermissionResponseOperationView | null> {
	const operationId = acceptedPermissionResponseOperations.get(
		permissionResponseTargetKey(sessionId, requestId),
	);
	if (!operationId) return null;
	return invoke<PermissionResponseOperationView>(
		"get_agent_permission_response_operation",
		{ operationId },
	);
}

/**
 * Keep one operation identity for the exact user response until the backend
 * proves whether its durable acceptance commit exists.
 */
export async function respondAgentPermission(
	sessionId: string,
	requestId: string,
	allow: boolean,
	updatedInput?: Record<string, unknown>,
): Promise<AcceptedPermissionResponseResult> {
	const snapshot: PermissionResponseAttemptSnapshot = {
		type: "permission_response",
		sessionId,
		requestId,
		behavior: allow ? "allow" : "deny",
		message: allow ? null : "User denied",
		updatedInput: updatedInput ? JSON.stringify(updatedInput) : null,
	};
	const attemptKey = JSON.stringify(snapshot);
	const operationId =
		permissionResponseAttempts.get(attemptKey) ??
		`permission-response-${crypto.randomUUID()}`;
	permissionResponseAttempts.set(attemptKey, operationId);
	saveStringMap(
		PERMISSION_RESPONSE_ATTEMPTS_STORAGE_KEY,
		permissionResponseAttempts,
	);

	let result: DurablePermissionResponseResult;
	try {
		result = await invokeDurablePermissionResponseAttempt(
			snapshot,
			operationId,
		);
	} catch (error) {
		try {
			const operation = await invoke<PermissionResponseOperationView>(
				"get_agent_permission_response_operation",
				{ operationId },
			);
			result = { type: "accepted", operation };
		} catch {
			throw error;
		}
	}
	if (result.type === "rejected_before_commit") {
		// Retain the exact snapshot and identity. A deliberate retry of this
		// response must not silently become a different caller operation.
		throw new Error(
			"Permission response was rejected before its durable acceptance commit",
		);
	}
	if (result.type === "outcome_unknown") {
		const unknownOperationId = result.operation_id;
		try {
			const operation = await invoke<PermissionResponseOperationView>(
				"get_agent_permission_response_operation",
				{ operationId: unknownOperationId },
			);
			result = { type: "accepted", operation };
		} catch {
			throw new Error(
				`Permission response acceptance is unknown; retry operation ${unknownOperationId}`,
			);
		}
	}
	permissionResponseAttempts.delete(attemptKey);
	saveStringMap(
		PERMISSION_RESPONSE_ATTEMPTS_STORAGE_KEY,
		permissionResponseAttempts,
	);
	const acceptedResult: AcceptedPermissionResponseResult = result;
	rememberAcceptedPermissionResponse(acceptedResult.operation);
	return acceptedResult;
}

export async function redispatchPendingPermissionResponseAttempts(
	operationIds: ReadonlySet<string>,
): Promise<void> {
	for (const [attemptKey, operationId] of [...permissionResponseAttempts]) {
		if (!operationIds.has(operationId)) continue;
		let snapshot: PermissionResponseAttemptSnapshot;
		try {
			snapshot = JSON.parse(attemptKey) as PermissionResponseAttemptSnapshot;
			if (snapshot.type !== "permission_response") continue;
		} catch {
			continue;
		}
		const result = await invokeDurablePermissionResponseAttempt(
			snapshot,
			operationId,
		);
		if (result.type === "accepted") {
			permissionResponseAttempts.delete(attemptKey);
			rememberAcceptedPermissionResponse(result.operation);
		}
	}
	saveStringMap(
		PERMISSION_RESPONSE_ATTEMPTS_STORAGE_KEY,
		permissionResponseAttempts,
	);
}

const stopAttempts = loadStringMap("releash.agent-stop-attempts.v1");

type DurableStopCommandResult =
	| {
			type: "accepted";
			receipt: { operation_id: string };
			state: StopOperationState;
	  }
	| { type: "rejected_before_commit"; failure: unknown }
	| { type: "outcome_unknown"; request_id: string };

type StopOperationState =
	| { type: "accepted" }
	| { type: "completed"; resolution: "succeeded" | "superseded" }
	| { type: "reconciliation_required"; failure: unknown };

type StopOperationLookup = [{ operation_id: string }, StopOperationState];

function isClosedStopState(state: StopOperationState): boolean {
	return state.type === "completed";
}

export async function redispatchPendingStopAttempts(
	requestIds: ReadonlySet<string>,
): Promise<void> {
	for (const [attemptKey, requestId] of [...stopAttempts]) {
		if (!requestIds.has(requestId)) continue;
		const [sessionId, turnId, expectedSessionRevision] = attemptKey.split(":");
		if (!sessionId || !turnId || expectedSessionRevision === undefined)
			continue;
		const result = await invoke<DurableStopCommandResult>(
			"stop_agent_session",
			{
				request: {
					request_id: requestId,
					session_id: sessionId,
					turn_id: turnId,
					expected_session_revision: expectedSessionRevision,
				},
			},
		);
		if (
			result.type === "rejected_before_commit" ||
			(result.type === "accepted" && isClosedStopState(result.state))
		) {
			stopAttempts.delete(attemptKey);
		}
	}
	saveStringMap("releash.agent-stop-attempts.v1", stopAttempts);
}

export async function requestAgentStop(
	sessionId: string,
	turnId: string,
	expectedSessionRevision: string,
): Promise<void> {
	const attemptKey = `${sessionId}:${turnId}:${expectedSessionRevision}`;
	const requestId =
		stopAttempts.get(attemptKey) ?? `stop-${crypto.randomUUID()}`;
	stopAttempts.set(attemptKey, requestId);
	saveStringMap("releash.agent-stop-attempts.v1", stopAttempts);
	const request = {
		request_id: requestId,
		session_id: sessionId,
		turn_id: turnId,
		expected_session_revision: expectedSessionRevision,
	};
	let attemptClosed = false;
	try {
		let result: DurableStopCommandResult;
		try {
			result = await invoke<DurableStopCommandResult>("stop_agent_session", {
				request,
			});
		} catch (error) {
			try {
				// This public lookup accepts the caller request identity and resolves it
				// to the deterministic backend-owned Stop operation.
				const [, state] = await invoke<StopOperationLookup>(
					"get_stop_operation",
					{ operationId: requestId },
				);
				attemptClosed = isClosedStopState(state);
				return;
			} catch {
				throw error;
			}
		}
		if (result.type === "rejected_before_commit") {
			attemptClosed = true;
			throw new Error("Stop was rejected before commit");
		}
		if (result.type === "outcome_unknown") {
			const [, state] = await invoke<StopOperationLookup>(
				"get_stop_operation",
				{
					operationId: result.request_id,
				},
			);
			attemptClosed = isClosedStopState(state);
		} else {
			attemptClosed = isClosedStopState(result.state);
		}
	} finally {
		if (attemptClosed) {
			stopAttempts.delete(attemptKey);
			saveStringMap("releash.agent-stop-attempts.v1", stopAttempts);
		}
	}
}

export async function sendWorkflowApprovalChatMessage(
	executionId: string,
	content: string,
	permissionMode: PermissionMode,
	planMode: boolean,
	images?: ImageAttachment[],
	mentions?: MentionReference[],
): Promise<DurableSendCommandResult> {
	return sendDurableAttempt({
		type: "workflow_approval",
		executionId,
		content,
		permissionMode,
		planMode,
		images: images ?? [],
		mentions: mentions ?? [],
	});
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

export async function resumeAgentQueue(chatSessionId: string): Promise<void> {
	return invoke<void>("resume_agent_queue", { chatSessionId });
}

interface RawInitSessionsResponse {
	sessions: RawSessionSummaryDtoV1[];
	active_session: RawGetSessionResponse | null;
	permission_mode: string;
	plan_mode: boolean;
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
	const activeSession = raw.active_session
		? convertRawGetSessionResponse(raw.active_session)
		: null;
	return {
		sessions: raw.sessions.map(convertSessionSummaryDtoV1),
		activeSession,
		permissionMode: normalizePermissionMode(raw.permission_mode),
		planMode: raw.plan_mode,
	};
}

export async function setSessionBackend(
	chatSessionId: string,
	backendId: string,
): Promise<GetSessionResponse> {
	await requestSessionLifecycle(chatSessionId, {
		type: "switch_backend",
		backend_id: backendId,
	});
	const current = await getSession(chatSessionId);
	if (!current) throw new Error("Session disappeared after backend switch");
	return current;
}

export interface BackendListResult {
	backends: BackendInfo[];
	defaultId: string | null;
}

export async function listAgentBackends(): Promise<BackendListResult> {
	return invoke<BackendListResult>("list_agent_backends");
}
