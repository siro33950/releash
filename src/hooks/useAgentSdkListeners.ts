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

interface SessionStateChanged {
	chat_session_id: string;
	turn_phase: TurnPhase;
	exit_code: number | null;
	completed_at?: number | null;
	interrupted?: boolean;
	session_state?: SessionState | null;
	pending_permission_request?: PermissionRequest | null;
	pending_permission_state_revision?: number | null;
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

interface AgentTurnUsageUpdated {
	chatSessionId: string;
	tokenUsage: TokenUsage;
}

/**
 * SDK listener gating のための「現在 UI 上で表示中の session id 集合」を引く registry。
 * 各 panel が表示開始時に register、unmount/離脱時に cleanup を呼ぶ。listener は本 set に
 * 含まれる session に対してのみ ADD_MESSAGE / SET_STREAMING_MESSAGE 等を dispatch する。
 */
interface ViewableSessionRegistry {
	register: (sessionId: string) => () => void;
	getIds: () => Set<string>;
}

type StreamingDeltaDropReason = "missing_session" | "missing_message";

export interface AgentSdkListenerRefs {
	dispatch: Dispatch<AgentChatAction>;
	viewableRegistry: ViewableSessionRegistry;
	refreshSessions: () => Promise<unknown>;
	getStreamingDeltaDropReason?: (
		sessionId: string,
		messageId: string,
	) => StreamingDeltaDropReason | null;
	worktreePath?: string;
}

function isViewable(
	sessionId: string,
	viewableRegistry: ViewableSessionRegistry,
): boolean {
	return viewableRegistry.getIds().has(sessionId);
}

function toPreparedChatMessage(
	message: LegacyChatMessage & { parts?: MessagePart[] | null },
) {
	return convertLegacyMessage({
		...message,
		parts: message.parts ?? undefined,
	});
}

function warnDroppedStreamingDelta(
	reason: StreamingDeltaDropReason,
	sessionId: string,
	messageId: string,
	seq: number,
): void {
	console.warn(
		reason === "missing_session"
			? "Dropped agent-streaming-delta for missing session"
			: "Dropped agent-streaming-delta for missing message",
		{
			sessionId,
			messageId,
			seq,
		},
	);
}

export function useAgentSdkListeners(refs: AgentSdkListenerRefs): void {
	const {
		dispatch,
		viewableRegistry,
		refreshSessions,
		getStreamingDeltaDropReason,
		worktreePath,
	} = refs;

	// Listen to typed turn usage updates emitted by Rust backend.
	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let cancelled = false;

		listen<AgentTurnUsageUpdated>("agent-turn-usage-updated", (event) => {
			const { chatSessionId, tokenUsage } = event.payload;
			dispatch({
				type: "SET_LATEST_TOKEN_USAGE",
				sessionId: chatSessionId,
				usage: tokenUsage,
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

			const dropReason = getStreamingDeltaDropReason?.(
				chat_session_id,
				message_id,
			);
			if (dropReason) {
				warnDroppedStreamingDelta(dropReason, chat_session_id, message_id, seq);
			}

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
	}, [dispatch, getStreamingDeltaDropReason]);

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
				pending_permission_state_revision,
			} = event.payload;
			const pendingPermissionStateRevision =
				typeof pending_permission_state_revision === "number" &&
				Number.isFinite(pending_permission_state_revision)
					? {
							pendingPermissionStateRevision: pending_permission_state_revision,
						}
					: {};

			dispatch({
				type: "SET_TURN_PHASE",
				sessionId: chat_session_id,
				turnPhase: turn_phase,
				...pendingPermissionStateRevision,
			});

			dispatch({
				type: "SET_PENDING_PERMISSION",
				sessionId: chat_session_id,
				request: pending_permission_request ?? null,
				...pendingPermissionStateRevision,
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
