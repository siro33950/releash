import { X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { ChatSessionView } from "@/components/panels/AgentChatPanel";
import {
	WorkflowPanel,
	type WorkflowStepSelection,
} from "@/components/panels/WorkflowPanel";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
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
 *   right = 上下 split：
 *           上 = Tab bar + ChatSessionView（選択中タブの agent step session）
 *           下 = WorkflowStepDetail（選択中タブの step 詳細）
 *
 * - 現 worktree に紐づく workflow run の active / history を 1 つの panel 内で観測。
 * - timeline 上で step を選ぶと右ペインにタブとして蓄積され、ユーザーは任意の step を
 *   同時に開いて切り替えながら inspect できる。session を持たない step（bash 等）も
 *   タブ化され、chat 部分は "No agent conversation" プレースホルダになる。
 * - approval / reject / abort などの state 変化は既存 Tauri command 経路に委ねる。
 *
 * 観測対象は現 worktree に紐づく run に限定する（engine 側の認可境界に依拠）。
 * 本コンポーネント自体は表示用整形・ローカル選択状態（開いているタブとアクティブ
 * タブ）の保持に閉じ、事実列・state 復元・approval 可否判定など run の意味解釈は
 * 一切持たない。
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

	const [openTabs, setOpenTabs] = useState<WorkflowStepSelection[]>([]);
	const [activeTabKey, setActiveTabKey] = useState<string | null>(null);

	// 現 worktree から離れたとき / 別 run へ遷移したときに開いているタブを全クリアする。
	const executionId = workflowState?.executionId ?? null;
	const identityKey = `${worktreePath}|${executionId ?? ""}`;
	const [prevIdentityKey, setPrevIdentityKey] = useState(identityKey);
	if (prevIdentityKey !== identityKey) {
		setPrevIdentityKey(identityKey);
		setOpenTabs([]);
		setActiveTabKey(null);
	}

	const activeTab = useMemo(
		() =>
			activeTabKey
				? (openTabs.find((tab) => stepTabKey(tab) === activeTabKey) ?? null)
				: null,
		[openTabs, activeTabKey],
	);
	// trace 上の Eye/EyeOff の "open" 状態が tab bar と完全に一致するよう、開いて
	// いる全タブの sessionId 一覧を WorkflowPanel に渡す。
	const openStepSessionIds = useMemo(
		() =>
			openTabs.map((tab) => tab.sessionId).filter((id): id is string => !!id),
		[openTabs],
	);

	const handleSelectStepSession = useCallback((next: WorkflowStepSelection) => {
		const key = stepTabKey(next);
		setOpenTabs((prev) =>
			prev.some((tab) => stepTabKey(tab) === key) ? prev : [...prev, next],
		);
		setActiveTabKey(key);
	}, []);

	const handleSelectTab = useCallback((key: string) => {
		setActiveTabKey(key);
	}, []);

	const handleCloseTab = useCallback(
		(key: string) => {
			setOpenTabs((prev) => {
				const idx = prev.findIndex((tab) => stepTabKey(tab) === key);
				if (idx === -1) return prev;
				const next = prev.filter((_, i) => i !== idx);
				if (activeTabKey === key) {
					// 閉じたタブがアクティブなら、右隣（無ければ左隣、それも無ければ null）に移す。
					const fallback = next[idx] ?? next[idx - 1] ?? null;
					setActiveTabKey(fallback ? stepTabKey(fallback) : null);
				}
				return next;
			});
		},
		[activeTabKey],
	);

	// WorkflowPanel（timeline）側からの "session を閉じる" 要求は sessionId 主語で
	// 渡ってくる。対応するタブを閉じる。
	const handleCloseSessionFromTimeline = useCallback(
		(sessionId: string) => {
			const target = openTabs.find((tab) => tab.sessionId === sessionId);
			if (target) handleCloseTab(stepTabKey(target));
		},
		[openTabs, handleCloseTab],
	);

	// アクティブタブの session 本文を取得・反映する。session を持たないタブの場合は
	// viewedStepSession をクリア。
	const activeSessionId = activeTab?.sessionId ?? null;
	useEffect(() => {
		if (activeSessionId) {
			loadStepSession(activeSessionId).catch((e) =>
				console.warn("[WorkflowSidebarPanel] loadStepSession failed", e),
			);
		} else {
			clearStepSession();
		}
	}, [activeSessionId, loadStepSession, clearStepSession]);

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

	const showChat = activeTab !== null && activeTab.sessionId != null;
	// step に session が紐づいているかだけでゲートする。nodeType は engine 側で
	// 追加・変更され得るため、UI 側で type 列挙すると新 type 追加時に破綻する。
	// approval step も current step session（被承認 agent step の session）を
	// 引き継いでおり、approval chat 経由で対話できる（engine の
	// validate_approval_chat_instruction / send_workflow_approval_chat_message 経路）。

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
							onCloseSession={handleCloseSessionFromTimeline}
							openStepSessionIds={openStepSessionIds}
							selectedStep={
								activeTab
									? {
											stepName: activeTab.stepName,
											runIndex: activeTab.runIndex,
										}
									: null
							}
						/>
					</div>
				</Panel>
				<Separator />
				<Panel id="workflow-step-right" defaultSize="40%" minSize="20%">
					<div className="flex h-full flex-col overflow-hidden border-l border-border">
						{openTabs.length === 0 ? (
							<div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">
								Select a node in the workflow to see its details.
							</div>
						) : (
							<Group orientation="vertical">
								<Panel id="workflow-step-chat" defaultSize="60%" minSize="20%">
									<div className="flex h-full flex-col overflow-hidden">
										<WorkflowStepTabBar
											openTabs={openTabs}
											activeTabKey={activeTabKey}
											onSelectTab={handleSelectTab}
											onCloseTab={handleCloseTab}
										/>
										<div className="flex-1 overflow-hidden">
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
													onPermissionModeChange={
														handleStepPermissionModeChange
													}
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
													No agent conversation for this step.
												</div>
											)}
										</div>
									</div>
								</Panel>
								<Separator />
								<Panel
									id="workflow-step-detail"
									defaultSize="40%"
									minSize="15%"
								>
									<div className="h-full overflow-auto border-t border-border">
										{activeTab && (
											<WorkflowStepDetail
												selection={activeTab}
												worktreePath={worktreePath}
												onClose={() => handleCloseTab(stepTabKey(activeTab))}
											/>
										)}
									</div>
								</Panel>
							</Group>
						)}
					</div>
				</Panel>
			</Group>
		</div>
	);
}

/**
 * step を識別する安定キー。sessionId があれば一意。sessionId を持たない step
 * （bash / parallel parent / 未紐付け completed）でも stepName + runIndex で
 * タブを区別できるようにする。
 */
function stepTabKey(sel: WorkflowStepSelection): string {
	return sel.sessionId ?? `step:${sel.stepName}#${sel.runIndex ?? 0}`;
}

interface WorkflowStepTabBarProps {
	openTabs: WorkflowStepSelection[];
	activeTabKey: string | null;
	onSelectTab: (key: string) => void;
	onCloseTab: (key: string) => void;
}

function WorkflowStepTabBar({
	openTabs,
	activeTabKey,
	onSelectTab,
	onCloseTab,
}: WorkflowStepTabBarProps) {
	return (
		<Tabs
			value={activeTabKey ?? ""}
			onValueChange={onSelectTab}
			className="shrink-0 gap-0"
		>
			<div className="flex items-center gap-2 border-b bg-background px-2 py-1">
				<TabsList
					data-testid="workflow-step-tab-list"
					className="w-auto max-w-full overflow-x-auto overflow-y-hidden justify-start [&::-webkit-scrollbar]:hidden [scrollbar-width:none]"
				>
					{openTabs.map((tab) => {
						const key = stepTabKey(tab);
						const label = tab.stepName || "step";
						return (
							<TabsTrigger key={key} value={key} asChild>
								<div className="gap-2">
									<span className="truncate max-w-[140px]">{label}</span>
									<button
										type="button"
										onPointerDown={(e) => e.stopPropagation()}
										onMouseDown={(e) => e.stopPropagation()}
										onClick={(e) => {
											e.stopPropagation();
											onCloseTab(key);
										}}
										className="p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
										aria-label={`Close tab ${label}`}
									>
										<X className="size-3.5" />
									</button>
								</div>
							</TabsTrigger>
						);
					})}
				</TabsList>
			</div>
		</Tabs>
	);
}
