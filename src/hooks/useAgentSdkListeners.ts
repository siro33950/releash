import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Dispatch } from "react";
import { useEffect } from "react";
import type {
	AgentSessionContextCarryUpdated,
	AgentSupportedCommandsUpdated,
} from "@/types/protocol";
import {
	type LegacyChatMessage,
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
import {
	convertLegacyMessage,
	convertLegacySession,
	type LegacyChatSession,
} from "./useSessionStore";

type SdkMessage = {
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
	interrupted?: boolean;
	session_state?: SessionState | null;
	pending_permission_request?: PermissionRequest | null;
}

interface StreamingMessageUpdated {
	chat_session_id: string;
	message_id: string;
	seq: number;
	snapshot?: boolean;
	parts: MessagePart[];
}

interface AgentTurnPrepared {
	chat_session_id: string;
	session: LegacyChatSession;
	human_message: LegacyChatMessage & { parts?: MessagePart[] | null };
	agent_message: LegacyChatMessage & { parts?: MessagePart[] | null };
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
	worktreePath?: string;
}

function isViewable(
	sessionId: string,
	viewableRegistry: ViewableSessionRegistry,
): boolean {
	return viewableRegistry.getIds().has(sessionId);
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

function toPreparedChatMessage(
	message: LegacyChatMessage & { parts?: MessagePart[] | null },
) {
	return convertLegacyMessage({
		...message,
		parts: message.parts ?? undefined,
	});
}

export function useAgentSdkListeners(refs: AgentSdkListenerRefs): void {
	const { dispatch, viewableRegistry, refreshSessions, worktreePath } = refs;

	// Listen to SDK messages for meta events that are not part of session state.
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SdkMessage>("agent-sdk-message", (event) => {
			const msg = event.payload;
			const chatSessionId = msg.chat_session_id;

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

	// Listen to prepared turn read model emitted before runtime streaming starts.
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<AgentTurnPrepared>("agent-turn-prepared", (event) => {
			const { chat_session_id, session, human_message, agent_message } =
				event.payload;
			if (worktreePath && session.worktreePath !== worktreePath) return;
			dispatch({
				type: "UPSERT_SESSION",
				session: convertLegacySession(session),
			});
			dispatch({
				type: "ADD_MESSAGE",
				sessionId: chat_session_id,
				message: toPreparedChatMessage(human_message),
			});
			dispatch({
				type: "ADD_MESSAGE",
				sessionId: chat_session_id,
				message: toPreparedChatMessage(agent_message),
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
	}, [dispatch, worktreePath]);

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
						sessionId: chat_session_id,
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

	// Listen to context-carry updates persisted by the Rust runtime.
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<AgentSessionContextCarryUpdated>(
			"agent-session-context-carry-updated",
			(event) => {
				const { chat_session_id, agent_session_id, context_carry, updated_at } =
					event.payload;
				if (!isViewable(chat_session_id, viewableRegistry)) return;
				dispatch({
					type: "SET_CONTEXT_CARRY",
					sessionId: chat_session_id,
					agentSessionId: agent_session_id ?? null,
					contextCarry: context_carry ?? null,
					updatedAt:
						typeof updated_at === "number" && Number.isFinite(updated_at)
							? updated_at
							: null,
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

	// Listen to agent-streaming-delta from Rust backend
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<StreamingMessageUpdated>("agent-streaming-delta", (event) => {
			const { chat_session_id, message_id, seq, snapshot, parts } =
				event.payload;

			if (snapshot) {
				dispatch({
					type: "SET_STREAMING_MESSAGE",
					sessionId: chat_session_id,
					messageId: message_id,
					parts,
				});
				return;
			}

			dispatch({
				type: "APPLY_STREAMING_DELTA",
				sessionId: chat_session_id,
				messageId: message_id,
				seq,
				parts,
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
	}, [dispatch]);

	// Listen to agent-session-state-changed (unified state event from Rust)
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<SessionStateChanged>("agent-session-state-changed", (event) => {
			const {
				chat_session_id,
				turn_phase,
				exit_code,
				completed_at,
				interrupted,
				session_state,
				pending_permission_request,
			} = event.payload;

			dispatch({
				type: "SET_TURN_PHASE",
				sessionId: chat_session_id,
				turnPhase: turn_phase,
			});

			dispatch({
				type: "SET_PENDING_PERMISSION",
				sessionId: chat_session_id,
				request: pending_permission_request ?? null,
			});

			// Turn completed (idle with exit_code): mirror backend state and clear permissions
			if (turn_phase === "idle" && exit_code != null) {
				if (
					!interrupted &&
					typeof completed_at === "number" &&
					Number.isFinite(completed_at)
				) {
					dispatch({
						type: "MARK_AGENT_TURN_COMPLETED",
						sessionId: chat_session_id,
						completedAt: completed_at,
					});
				}

				if (session_state && isViewable(chat_session_id, viewableRegistry)) {
					dispatch({
						type: "UPDATE_SESSION_STATE",
						sessionId: chat_session_id,
						state: session_state,
					});
				}

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
