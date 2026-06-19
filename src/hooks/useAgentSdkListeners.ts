import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Dispatch } from "react";
import { useEffect } from "react";
import type { AgentSupportedCommandsUpdated } from "@/types/protocol";
import {
	type MessagePart,
	type ModelInfo,
	normalizeModelSelectionId,
	normalizePermissionMode,
	type PermissionRequest,
	type SessionState,
	type TokenUsage,
	type TurnPhase,
} from "@/types/session";
import type { AgentChatAction } from "./agentChatReducer";
import { getSession, updateSessionState } from "./useSessionStore";

interface PermissionRequestMessage {
	type: "permission_request";
	session_id?: string;
	chat_session_id?: string;
	request_id: string;
	tool_name: string;
	input: Record<string, unknown>;
	tool_use_id: string;
	title?: string;
	display_name?: string;
	description?: string;
	decision_reason?: string;
}

type SdkMessage =
	| PermissionRequestMessage
	| {
			type: string;
			session_id?: string;
			chat_session_id?: string;
			parent_tool_use_id?: string | null;
			[key: string]: unknown;
	  };

interface SessionStateChanged {
	chat_session_id: string;
	turn_phase: TurnPhase;
	exit_code: number | null;
	completed_at?: number | null;
}

interface StreamingMessageUpdated {
	chat_session_id: string;
	message_id: string;
	parts: MessagePart[];
}

interface PendingMessageConsumed {
	chat_session_id: string;
	queued_turn_id?: string;
	/**
	 * drain 時に永続化された人間メッセージ。キュー投入時は transcript に追加せず、
	 * ここで初めて transcript に出す（二重表示の防止）。
	 */
	human_message?: {
		id: string;
		role: "human";
		content: string;
		parts?: MessagePart[] | null;
		timestamp: number;
	};
	agent_message: {
		id: string;
		role: "agent";
		timestamp: number;
	};
}

interface ModelsUpdated {
	chat_session_id: string;
	available_models: ModelInfo[];
	selected_model: string;
}

/**
 * SDK listener gating のための「現在 UI 上で表示中の session id 集合」を引く registry。
 * 各 panel が表示開始時に register、unmount/離脱時に cleanup を呼ぶ。listener は本 set に
 * 含まれる session に対してのみ ADD_MESSAGE / SET_STREAMING_MESSAGE 等を dispatch する。
 */
export interface ViewableSessionRegistry {
	register: (sessionId: string) => () => void;
	getIds: () => Set<string>;
}

export interface AgentSdkListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	viewableRegistry: ViewableSessionRegistry;
	refreshSessions: () => Promise<unknown>;
	/**
	 * 指定 session に message_id のメッセージが既に存在するか。streaming 更新時の
	 * cache-miss hydration を「本当に未存在の時だけ」に限定するために使う。毎回
	 * getSession→UPSERT_SESSION すると、drain 直後の楽観追加メッセージを古い
	 * snapshot で上書きして消すレースが起きる。
	 */
	hasMessage: (sessionId: string, messageId: string) => boolean;
}

function isViewable(
	sessionId: string,
	viewableRegistry: ViewableSessionRegistry,
): boolean {
	return viewableRegistry.getIds().has(sessionId);
}

function handlePermissionRequest(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
): void {
	if (msg.type !== "permission_request" || !chatSessionId) return;
	const prMsg = msg as PermissionRequestMessage;
	const req: PermissionRequest = {
		request_id: prMsg.request_id,
		tool_name: prMsg.tool_name,
		input: prMsg.input,
		tool_use_id: prMsg.tool_use_id,
		title: prMsg.title,
		display_name: prMsg.display_name,
		description: prMsg.description,
		decision_reason: prMsg.decision_reason,
	};
	dispatch({
		type: "SET_PENDING_PERMISSION",
		sessionId: chatSessionId,
		request: req,
	});
}

function handleSystemMessage(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	viewableRegistry: ViewableSessionRegistry,
): void {
	if (msg.type !== "system" || !chatSessionId) return;
	// task subtypes are handled by Rust accumulation
	const subtype = typeof msg.subtype === "string" ? msg.subtype : undefined;
	if (
		subtype === "task_started" ||
		subtype === "task_notification" ||
		subtype === "task_progress"
	)
		return;
	// Skip dispatching for sessions not currently shown (Rust persists these)
	if (!isViewable(chatSessionId, viewableRegistry)) return;
	const text =
		typeof msg.message === "string"
			? msg.message
			: typeof msg.content === "string"
				? msg.content
				: null;
	if (text) {
		dispatch({
			type: "ADD_MESSAGE",
			sessionId: chatSessionId,
			message: {
				id: `system-${Date.now()}`,
				role: "system",
				parts: [{ type: "text", content: text }],
				timestamp: Date.now(),
			},
		});
	}
}

function handleResultErrors(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
	viewableRegistry: ViewableSessionRegistry,
): void {
	if (msg.type !== "result" || !chatSessionId) return;
	if (!isViewable(chatSessionId, viewableRegistry)) return;
	const resultMsg = msg as {
		type: "result";
		errors?: string[];
	};
	if (resultMsg.errors && resultMsg.errors.length > 0) {
		dispatch({
			type: "ADD_MESSAGE",
			sessionId: chatSessionId,
			message: {
				id: `system-error-${Date.now()}`,
				role: "agent",
				parts: [{ type: "error", content: resultMsg.errors.join("\n") }],
				timestamp: Date.now(),
			},
		});
	}
}

function numberFromUnknown(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function tokenUsageFromResultMessage(msg: SdkMessage): TokenUsage | null {
	if (msg.type !== "result") return null;
	const modelUsage = msg.modelUsage;
	if (!modelUsage || typeof modelUsage !== "object") return null;

	let inputTokens = 0;
	let outputTokens = 0;
	let totalTokens = 0;
	let sawExplicitTotal = false;
	let contextWindowTokens: number | undefined;

	for (const usage of Object.values(modelUsage as Record<string, unknown>)) {
		if (!usage || typeof usage !== "object") continue;
		const entry = usage as Record<string, unknown>;
		const input = numberFromUnknown(entry.inputTokens) ?? 0;
		const output = numberFromUnknown(entry.outputTokens) ?? 0;
		inputTokens += input;
		outputTokens += output;
		const total = numberFromUnknown(entry.totalTokens);
		if (total != null) {
			totalTokens += total;
			sawExplicitTotal = true;
		}
		const window = numberFromUnknown(entry.contextWindowTokens);
		if (window != null) {
			contextWindowTokens =
				contextWindowTokens == null
					? window
					: Math.max(contextWindowTokens, window);
		}
	}

	if (inputTokens === 0 && outputTokens === 0 && !sawExplicitTotal) {
		return null;
	}

	return {
		inputTokens,
		outputTokens,
		totalTokens: sawExplicitTotal ? totalTokens : inputTokens + outputTokens,
		contextWindowTokens,
	};
}

function handleResultTokenUsage(
	msg: SdkMessage,
	chatSessionId: string | undefined,
	dispatch: Dispatch<AgentChatAction>,
): void {
	if (!chatSessionId) return;
	const usage = tokenUsageFromResultMessage(msg);
	if (!usage) return;
	dispatch({
		type: "SET_LATEST_TOKEN_USAGE",
		sessionId: chatSessionId,
		usage,
	});
}

export function useAgentSdkListeners(refs: AgentSdkListenerRefs): void {
	const { dispatch, viewableRegistry, refreshSessions, hasMessage } = refs;

	// Listen to SDK messages for meta events (permissions, system messages)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SdkMessage>("agent-sdk-message", (event) => {
			const msg = event.payload;
			const chatSessionId = msg.chat_session_id;

			handlePermissionRequest(msg, chatSessionId, dispatch);
			handleSystemMessage(msg, chatSessionId, dispatch, viewableRegistry);
			handleResultErrors(msg, chatSessionId, dispatch, viewableRegistry);
			handleResultTokenUsage(msg, chatSessionId, dispatch);
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, viewableRegistry]);

	// Listen to agent-supported-commands-updated from Rust backend.
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<AgentSupportedCommandsUpdated>(
			"agent-supported-commands-updated",
			(event) => {
				const { chat_session_id, commands } = event.payload;
				dispatch({
					type: "SET_RUNTIME_SLASH_COMMANDS",
					sessionId: chat_session_id,
					commands,
				});
			},
		).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch]);

	// Listen to agent-permission-mode-changed from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<{ chat_session_id: string; permission_mode: string }>(
			"agent-permission-mode-changed",
			(event) => {
				const { chat_session_id, permission_mode } = event.payload;
				// Only update global permission mode if the event session is currently viewable.
				if (isViewable(chat_session_id, viewableRegistry)) {
					dispatch({
						type: "SET_PERMISSION_MODE",
						mode: normalizePermissionMode(permission_mode),
					});
				}
			},
		).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, viewableRegistry]);

	// Listen to agent-streaming-updated from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;
		let refreshInFlight = false;

		listen<StreamingMessageUpdated>(
			"agent-streaming-updated",
			async (event) => {
				const { chat_session_id, message_id, parts } = event.payload;

				// viewable でない session の streaming 更新は他のハンドラと同様にスキップ。
				// SET_STREAMING_MESSAGE のみガード外にあると非表示 session の streaming が
				// sessionsById に反映される非対称が生じるため、ここで早期 return する。
				if (!isViewable(chat_session_id, viewableRegistry)) {
					return;
				}

				// Cache miss: SET_STREAMING_MESSAGE は session.messages 内に message_id が
				// 存在しない場合 no-op になる。viewable な session で message_id が
				// 未確認の場合のみ getSession で fetch → UPSERT_SESSION で sessionsById を
				// 更新し、後続の SET_STREAMING_MESSAGE が反映できる状態に揃える。
				//
				// 重要: message_id が既に存在する場合に毎回 getSession→UPSERT_SESSION
				// すると、drain 直後に楽観追加したメッセージを「投入前の古い snapshot」で
				// 上書きして一瞬で消すレースが起きる。必ず未存在時に限定する。
				if (!hasMessage(chat_session_id, message_id) && !refreshInFlight) {
					refreshInFlight = true;
					try {
						const response = await getSession(chat_session_id);
						if (response && !cancelled) {
							dispatch({ type: "UPSERT_SESSION", session: response.session });
						}
					} finally {
						refreshInFlight = false;
					}
				}

				dispatch({
					type: "SET_STREAMING_MESSAGE",
					sessionId: chat_session_id,
					messageId: message_id,
					parts,
				});
			},
		).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, viewableRegistry, hasMessage]);

	// Listen to agent-session-state-changed (unified state event from Rust)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SessionStateChanged>("agent-session-state-changed", (event) => {
			const { chat_session_id, turn_phase, exit_code, completed_at } =
				event.payload;

			dispatch({
				type: "SET_TURN_PHASE",
				sessionId: chat_session_id,
				turnPhase: turn_phase,
			});

			// Turn completed (idle with exit_code): update session state and clear permissions
			if (turn_phase === "idle" && exit_code != null) {
				if (typeof completed_at === "number" && Number.isFinite(completed_at)) {
					dispatch({
						type: "MARK_AGENT_TURN_COMPLETED",
						sessionId: chat_session_id,
						completedAt: completed_at,
					});
				}

				dispatch({
					type: "SET_PENDING_PERMISSION",
					sessionId: chat_session_id,
					request: null,
				});

				const newState: SessionState = exit_code === 0 ? "done" : "error";
				if (isViewable(chat_session_id, viewableRegistry)) {
					dispatch({
						type: "UPDATE_SESSION_STATE",
						sessionId: chat_session_id,
						state: newState,
					});
				}

				updateSessionState(chat_session_id, newState).catch((e) =>
					console.error("Failed to update session state:", e),
				);

				refreshSessions().catch((e) =>
					console.error("Failed to refresh sessions:", e),
				);
			}
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, viewableRegistry, refreshSessions]);

	// Listen to agent-pending-message-consumed (Rust auto-consumed pending message after turn_complete)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<PendingMessageConsumed>(
			"agent-pending-message-consumed",
			(event) => {
				const {
					chat_session_id,
					queued_turn_id,
					human_message,
					agent_message,
				} = event.payload;
				if (!isViewable(chat_session_id, viewableRegistry)) return;
				if (queued_turn_id) {
					dispatch({
						type: "REMOVE_PENDING_QUEUE_ITEM",
						sessionId: chat_session_id,
						queuedTurnId: queued_turn_id,
					});
				}
				// drain 時に永続化された人間メッセージを、agent メッセージより先に
				// transcript へ追加する（キュー投入時は意図的に追加していない）。
				if (human_message) {
					// ChatMessage は parts のみを持つ（content は text part として表現）。
					const humanParts: MessagePart[] =
						human_message.parts && human_message.parts.length > 0
							? human_message.parts
							: [{ type: "text", content: human_message.content }];
					dispatch({
						type: "ADD_MESSAGE",
						sessionId: chat_session_id,
						message: {
							id: human_message.id,
							role: human_message.role,
							parts: humanParts,
							timestamp: human_message.timestamp,
						},
					});
				}
				dispatch({
					type: "ADD_MESSAGE",
					sessionId: chat_session_id,
					message: {
						id: agent_message.id,
						role: agent_message.role,
						parts: [],
						timestamp: agent_message.timestamp,
					},
				});
			},
		).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, viewableRegistry]);

	// Listen to agent-models-updated (session 単位の更新) from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<ModelsUpdated>("agent-models-updated", (event) => {
			const { chat_session_id, available_models, selected_model } =
				event.payload;
			const modelId = normalizeModelSelectionId(
				available_models,
				selected_model,
			);
			if (isViewable(chat_session_id, viewableRegistry)) {
				dispatch({
					type: "SET_AVAILABLE_MODELS",
					models: available_models,
				});
			}
			dispatch({
				type: "SET_SESSION_MODEL",
				sessionId: chat_session_id,
				modelId,
			});
		}).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [dispatch, viewableRegistry]);
}
