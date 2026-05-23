import type React from "react";
import { useCallback, useEffect, useMemo } from "react";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import { deriveActivityStatus } from "@/hooks/deriveActivityStatus";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import type {
	ImageAttachment,
	MentionReference,
	PermissionMode,
} from "@/types/session";
import { ChatSessionView } from "./ChatSessionView";

interface BoundSessionChatProps {
	/** 表示対象 session の id。null の場合は何もレンダリングしない（empty fallback は親側）。*/
	sessionId: string | null;
	worktreePath: string;
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
 * 提供する component。AgentChatPanel と WorkflowSidebarPanel の両方から、sessionId だけを
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
		getSessionSelectedModel,
		availableModels,
		backends,
		error,
		permissionMode: contextPermissionMode,
		sendMessage,
		interrupt,
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

	const turnPhase = sessionId ? getSessionTurnPhase(sessionId) : "idle";
	const isStreaming =
		turnPhase === "streaming" || turnPhase === "waiting_permission";
	const activityStatus = useMemo(() => {
		if (!session) return null;
		return deriveActivityStatus(session.messages, turnPhase);
	}, [session, turnPhase]);

	const handleSend = useCallback(
		(
			content: string,
			images?: ImageAttachment[],
			mentions?: MentionReference[],
		) => {
			if (!sessionId) return Promise.resolve();
			return sendMessage(sessionId, content, images, mentions);
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
		(modelId: string | null) => {
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

	const selectedModel = getSessionSelectedModel(session.id);
	const canChangeBackend =
		session.messages.length === 0 && !session.agentSessionId && !isStreaming;

	return (
		<ChatSessionView
			key={session.id}
			session={session}
			isStreaming={isStreaming}
			activityStatus={activityStatus}
			error={error}
			permissionMode={contextPermissionMode}
			availableModels={availableModels}
			selectedModel={selectedModel}
			backends={backends}
			selectedBackendId={session.backendId ?? null}
			canChangeBackend={canChangeBackend}
			worktreePath={worktreePath}
			onSend={handleSend}
			onInterrupt={handleInterrupt}
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
