import type React from "react";
import { useCallback, useEffect, useMemo } from "react";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import { deriveActivityStatus } from "@/hooks/deriveActivityStatus";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import { useSessionStatus } from "@/hooks/useSessionStatus";
import type {
	AgentEditorContext,
	AgentEditorSelection,
	ImageAttachment,
	MentionReference,
	PermissionMode,
} from "@/types/session";
import { ChatSessionView } from "./ChatSessionView";

interface BoundSessionChatProps {
	/** 表示対象 session の id。null の場合は何もレンダリングしない（empty fallback は親側）。*/
	sessionId: string | null;
	worktreePath: string;
	activeEditorPath?: string | null;
	openEditorPaths?: string[];
	activeEditorSelection?: AgentEditorSelection | null;
	registerDropZone?: (
		zone: DropZoneType,
		element: HTMLElement | null,
		onDrop?: (paths: string[]) => void,
	) => void;
	dropZoneName?: DropZoneType;
	sendMessageRef?: React.MutableRefObject<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>;
	/**
	 * sessionId が既に AgentChatPanel の active として読み込み済みかどうかを親が知っている
	 * ケース向けの最適化フック。指定なし（default false）の場合は本コンポーネントが
	 * sessionId 変更時に `loadSession` を呼び出して sessionsById に upsert する。
	 */
	skipInitialLoad?: boolean;
}

/**
 * 「指定 sessionId に対する完全な chat UI（message stream + MessageInput + handlers）」を
 * 提供する component。AgentChatPanel と WorkflowView の両方から、sessionId だけを
 * 渡すと chat 部分は共通実装になる、というのが本 component の責務境界。
 *
 * 内部処理:
 *   - `getSessionById(sessionId)` で sessionsById から ChatSession を解決
 *   - sessionId 変化時に `loadSession(sessionId)` を呼び、sessionsById を upsert
 *   - `registerViewableSession(sessionId)` で SDK listener gating の対象に登録
 *   - turnPhase / activity / handlers を sessionId に bind して ChatSessionView に渡す
 */
export function BoundSessionChat({
	sessionId,
	worktreePath,
	activeEditorPath,
	openEditorPaths,
	activeEditorSelection,
	registerDropZone,
	dropZoneName,
	sendMessageRef,
	skipInitialLoad = false,
}: BoundSessionChatProps) {
	const {
		getSessionById,
		loadSession,
		registerViewableSession,
		getSessionTurnPhase,
		getSessionInterrupting,
		getSessionSelectedModel,
		getSessionPendingQueue = () => [],
		getSessionLatestTokenUsage = () => null,
		getSessionCodexGoal = () => null,
		getSessionCodexRuntimeStatus = () => null,
		getSessionRuntimeSlashCommands = () => [],
		availableModels,
		backends,
		error,
		permissionMode: contextPermissionMode,
		sendMessage,
		interrupt,
		cancelQueuedTurn = async () => {},
		rewindSessionToMessage,
		setPermissionMode,
		setModel,
		setBackend,
		respondPermission,
	} = useAgentChatContext();

	// SDK listener gating: 本 view が表示している session を viewable に登録する。
	useEffect(() => {
		if (!sessionId) return;
		const cleanup = registerViewableSession(sessionId);
		return cleanup;
	}, [sessionId, registerViewableSession]);

	// sessionId 変化時に sessionsById に最新を upsert する。
	useEffect(() => {
		if (!sessionId || skipInitialLoad) return;
		loadSession(sessionId).catch((e) =>
			console.warn("[BoundSessionChat] loadSession failed", e),
		);
	}, [sessionId, skipInitialLoad, loadSession]);

	const session = getSessionById(sessionId);
	const sessionStatus = useSessionStatus(sessionId);

	const turnPhase = sessionId ? getSessionTurnPhase(sessionId) : "idle";
	const isStreaming =
		turnPhase === "streaming" || turnPhase === "waiting_permission";
	// interrupt 要求を出してから idle になるまでの楽観フラグ（停止ボタンの即時反映用）。
	const isInterrupting = sessionId ? getSessionInterrupting(sessionId) : false;
	const activityStatus = useMemo(() => {
		if (!session) return null;
		return deriveActivityStatus(session.messages, turnPhase);
	}, [session, turnPhase]);

	const handleSend = useCallback(
		(
			content: string,
			images?: ImageAttachment[],
			mentions?: MentionReference[],
			options?: {
				activateNewSession?: boolean;
				forkNewSession?: boolean;
				editorContext?: AgentEditorContext;
			},
		) => {
			const targetSessionId = options?.forkNewSession ? null : sessionId;
			if (!targetSessionId && !options?.forkNewSession)
				return Promise.resolve();
			const sendOptions =
				options?.activateNewSession === undefined && !options?.editorContext
					? undefined
					: {
							activateNewSession: options?.activateNewSession,
							editorContext: options?.editorContext,
						};
			if (!sendOptions) {
				return sendMessage(targetSessionId, content, images, mentions);
			}
			return sendMessage(
				targetSessionId,
				content,
				images,
				mentions,
				sendOptions,
			);
		},
		[sessionId, sendMessage],
	);

	const handleInterrupt = useCallback(() => {
		if (!sessionId) return;
		interrupt(sessionId);
	}, [sessionId, interrupt]);

	const handlePermissionModeChange = useCallback(
		(mode: PermissionMode) => {
			setPermissionMode(sessionId, mode);
		},
		[sessionId, setPermissionMode],
	);

	const handleModelChange = useCallback(
		(modelId: string) => {
			if (!sessionId) return;
			setModel(sessionId, modelId);
		},
		[sessionId, setModel],
	);

	const handleBackendChange = useCallback(
		(backendId: string | null) => {
			setBackend(sessionId, backendId);
		},
		[sessionId, setBackend],
	);

	const handleRespondPermission = useCallback(
		(
			requestId: string,
			allow: boolean,
			updatedInput?: Record<string, unknown>,
		) => {
			if (!sessionId) return;
			respondPermission(sessionId, requestId, allow, updatedInput);
		},
		[sessionId, respondPermission],
	);

	if (!session) return null;

	// 契約: Rust から届いた後の selected_model は常に非 null。session meta が
	// 反映される前の transient な null は、デフォルト（固定リスト先頭 = 表示中
	// backend の available models[0]）に解決して常に string を伝播する。
	const selectedModel =
		getSessionSelectedModel(session.id) ?? availableModels[0]?.value ?? "";
	const pendingQueue = getSessionPendingQueue(session.id);
	const latestTokenUsage = getSessionLatestTokenUsage(session.id);
	const codexGoal = getSessionCodexGoal(session.id);
	const codexRuntimeStatus = getSessionCodexRuntimeStatus(session.id);
	const runtimeSlashCommands = getSessionRuntimeSlashCommands(session.id);
	const canChangeBackend =
		session.messages.length === 0 && !session.agentSessionId && !isStreaming;

	return (
		<ChatSessionView
			key={session.id}
			session={session}
			sessionStatus={sessionStatus}
			isStreaming={isStreaming}
			isInterrupting={isInterrupting}
			activityStatus={activityStatus}
			error={error}
			permissionMode={contextPermissionMode}
			availableModels={availableModels}
			selectedModel={selectedModel}
			pendingQueue={pendingQueue}
			latestTokenUsage={latestTokenUsage}
			codexGoal={codexGoal}
			codexRuntimeStatus={codexRuntimeStatus}
			runtimeSlashCommands={runtimeSlashCommands}
			backends={backends}
			selectedBackendId={session.backendId ?? null}
			canChangeBackend={canChangeBackend}
			worktreePath={worktreePath}
			activeEditorPath={activeEditorPath}
			openEditorPaths={openEditorPaths}
			activeEditorSelection={activeEditorSelection}
			onSend={handleSend}
			onInterrupt={handleInterrupt}
			onCancelQueuedTurn={(queuedTurnId) =>
				cancelQueuedTurn(session.id, queuedTurnId)
			}
			onRewindToMessage={(messageId, options) =>
				rewindSessionToMessage(session.id, messageId, options)
			}
			onPermissionModeChange={handlePermissionModeChange}
			onModelChange={handleModelChange}
			onBackendChange={handleBackendChange}
			onRespondPermission={handleRespondPermission}
			registerDropZone={registerDropZone}
			dropZoneName={dropZoneName}
			sendMessageRef={sendMessageRef}
		/>
	);
}
