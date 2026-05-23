import { useCallback, useEffect, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { ChatSessionView } from "@/components/panels/AgentChatPanel";
import {
	WorkflowPanel,
	type WorkflowStepSelection,
} from "@/components/panels/WorkflowPanel";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import type { PermissionMode } from "@/types/session";
import { WorkflowStepDetail } from "./WorkflowStepDetail";

interface WorkflowSidebarPanelProps {
	worktreePath: string;
	permissionMode?: PermissionMode;
}

/**
 * spec issues-1023: 右パネルの Workflow モードに対応するトップレベル panel。
 *
 * レイアウト：
 *   left  = WorkflowPanel（timeline 全体）
 *   right = 上下 split：上 = ChatSessionView（agent step session）、下 = WorkflowStepDetail
 *
 * - 現 worktree に紐づく workflow run の active / history を 1 つの panel 内で観測
 * - timeline 上で agent step を選ぶと、その step の chat session（composer 付き）を
 *   右上に inline 表示する。tab bar を開く必要は無い（spec issues-1023）。
 * - approval / reject / abort などの state 変化は既存 Tauri command 経路に委ねる。
 *
 * 観測対象は現 worktree に紐づく run に限定する（engine 側の認可境界に依拠）。
 * 本コンポーネント自体は表示用整形・ローカル選択状態の保持に閉じ、事実列・state 復元・
 * approval 可否判定など run の意味解釈は一切持たない。
 */
export function WorkflowSidebarPanel({
	worktreePath,
	permissionMode = "readonly",
}: WorkflowSidebarPanelProps) {
	const { workflowState } = useWorkflowState(worktreePath);
	const {
		viewedStepSession,
		viewedStepSessionStreaming,
		viewedStepSessionActivityStatus,
		error,
		loadStepSession,
		clearStepSession,
		sendMessage,
		interrupt,
		setPermissionMode,
		respondPermission,
		setModel,
		setBackend,
		availableModels,
		backends,
		getSessionSelectedModel,
	} = useAgentChatContext();

	const [selection, setSelection] = useState<WorkflowStepSelection | null>(
		null,
	);

	// 現 worktree から離れたとき / 別 run へ遷移したときに選択をクリア。
	const executionId = workflowState?.executionId ?? null;
	const identityKey = `${worktreePath}|${executionId ?? ""}`;
	const [prevIdentityKey, setPrevIdentityKey] = useState(identityKey);
	if (prevIdentityKey !== identityKey) {
		setPrevIdentityKey(identityKey);
		setSelection(null);
	}

	const handleSelectStepSession = useCallback(
		(next: WorkflowStepSelection) => setSelection(next),
		[],
	);
	const handleClearStepSession = useCallback(() => {
		setSelection(null);
		clearStepSession();
	}, [clearStepSession]);

	const selectedStepSessionId = selection?.sessionId ?? null;
	const selectedStepKey = selection
		? { stepName: selection.stepName, runIndex: selection.runIndex }
		: null;
	// step に session が紐づいているかだけでゲートする。nodeType は engine 側で
	// 追加・変更され得るため、UI 側で type 列挙すると新 type 追加時に破綻する。
	// approval step も current step session（被承認 agent step の session）を
	// 引き継いでおり、approval chat 経由で対話できる（engine の
	// validate_approval_chat_instruction / send_workflow_approval_chat_message 経路）。
	const showChat = selection !== null && selection.sessionId != null;
	const showDetail = selection !== null;

	// 選択中 step session の本文を取得・反映する。step 選択が変わるか、agent でなくなる
	// 場合は viewedStepSession をクリアする。
	useEffect(() => {
		if (showChat && selection?.sessionId) {
			loadStepSession(selection.sessionId).catch((e) =>
				console.warn("[WorkflowSidebarPanel] loadStepSession failed", e),
			);
		} else {
			clearStepSession();
		}
	}, [showChat, selection?.sessionId, loadStepSession, clearStepSession]);

	// session-explicit handlers を viewedStepSession の id にバインドして
	// ChatSessionView に渡す。
	const stepSessionId = viewedStepSession?.id ?? null;
	const handleStepSend = useCallback(
		(
			content: string,
			images?: Parameters<typeof sendMessage>[2],
			mentions?: Parameters<typeof sendMessage>[3],
		) => {
			if (!stepSessionId) return Promise.resolve();
			return sendMessage(stepSessionId, content, images, mentions);
		},
		[stepSessionId, sendMessage],
	);
	const handleStepInterrupt = useCallback(() => {
		if (stepSessionId) interrupt(stepSessionId);
	}, [stepSessionId, interrupt]);
	const handleStepPermissionModeChange = useCallback(
		(mode: Parameters<typeof setPermissionMode>[1]) => {
			setPermissionMode(stepSessionId, mode);
		},
		[stepSessionId, setPermissionMode],
	);
	const handleStepModelChange = useCallback(
		(modelId: string | null) => {
			if (stepSessionId) setModel(stepSessionId, modelId);
		},
		[stepSessionId, setModel],
	);
	const handleStepBackendChange = useCallback(
		(backendId: string | null) => setBackend(stepSessionId, backendId),
		[stepSessionId, setBackend],
	);
	const handleStepRespondPermission = useCallback(
		(
			requestId: string,
			allow: boolean,
			updatedInput?: Record<string, unknown>,
		) => {
			if (!stepSessionId) return;
			respondPermission(stepSessionId, requestId, allow, updatedInput);
		},
		[stepSessionId, respondPermission],
	);

	const stepSessionPermissionMode = viewedStepSession?.permissionMode ?? "edit";
	const stepSessionSelectedModel = stepSessionId
		? getSessionSelectedModel(stepSessionId)
		: null;
	const stepSessionBackendId = viewedStepSession?.backendId ?? null;
	const stepCanChangeBackend =
		!!viewedStepSession &&
		viewedStepSession.messages.length === 0 &&
		!viewedStepSession.agentSessionId &&
		!viewedStepSessionStreaming;

	return (
		<div
			data-testid="workflow-sidebar-panel"
			className="flex h-full flex-col overflow-hidden"
		>
			<Group orientation="horizontal">
				<Panel id="workflow-trace" defaultSize="60%" minSize="20%">
					<div className="h-full overflow-hidden">
						<WorkflowPanel
							workflowState={workflowState ?? null}
							worktreePath={worktreePath}
							permissionMode={permissionMode}
							onSessionClick={handleSelectStepSession}
							onCloseSession={handleClearStepSession}
							selectedStepSessionId={selectedStepSessionId}
							selectedStep={selectedStepKey}
						/>
					</div>
				</Panel>
				<Separator />
				<Panel id="workflow-step-right" defaultSize="40%" minSize="20%">
					<div className="flex h-full flex-col overflow-hidden border-l border-border">
						{showDetail && selection ? (
							<Group orientation="vertical">
								<Panel id="workflow-step-chat" defaultSize="60%" minSize="20%">
									<div className="h-full overflow-hidden">
										{showChat && viewedStepSession ? (
											<ChatSessionView
												key={viewedStepSession.id}
												session={viewedStepSession}
												isStreaming={viewedStepSessionStreaming}
												activityStatus={viewedStepSessionActivityStatus}
												error={error}
												permissionMode={stepSessionPermissionMode}
												availableModels={availableModels}
												selectedModel={stepSessionSelectedModel}
												backends={backends}
												selectedBackendId={stepSessionBackendId}
												canChangeBackend={stepCanChangeBackend}
												worktreePath={worktreePath}
												onSend={handleStepSend}
												onInterrupt={handleStepInterrupt}
												onPermissionModeChange={handleStepPermissionModeChange}
												onModelChange={handleStepModelChange}
												onBackendChange={handleStepBackendChange}
												onRespondPermission={handleStepRespondPermission}
												// drop zone は WorkflowSidebarPanel 専用に登録しない。
												// 画像 drop は AgentChatPanel 側に閉じる（spec 範囲外の追加機能を増やさない）。
											/>
										) : showChat ? (
											<div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">
												Loading step conversation...
											</div>
										) : (
											<div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">
												This step has no agent conversation.
											</div>
										)}
									</div>
								</Panel>
								<Separator />
								<Panel
									id="workflow-step-detail"
									defaultSize="40%"
									minSize="15%"
								>
									<div className="h-full overflow-auto border-t border-border">
										<WorkflowStepDetail
											selection={selection}
											worktreePath={worktreePath}
											onClose={handleClearStepSession}
										/>
									</div>
								</Panel>
							</Group>
						) : (
							<div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">
								Select a node in the workflow to see its details.
							</div>
						)}
					</div>
				</Panel>
			</Group>
		</div>
	);
}
