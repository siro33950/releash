import { X } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { BoundSessionChat } from "@/components/panels/AgentChatPanel";
import {
	WorkflowPanel,
	type WorkflowStepSelection,
} from "@/components/panels/WorkflowPanel";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import type { PermissionMode } from "@/types/session";
import { WorkflowStepDetail } from "./WorkflowStepDetail";

interface WorkflowViewProps {
	worktreePath: string;
	permissionMode?: PermissionMode;
}

/**
 * spec issues-1023: 中央エリアの Workflow モードに対応するトップレベル panel。
 *
 * レイアウト：
 *   left  = WorkflowPanel（timeline 全体）
 *   right = 上下 split：
 *           上 = Tab bar + BoundSessionChat（選択中タブの agent step session）
 *           下 = WorkflowStepDetail（選択中タブの step 詳細）
 *
 * chat 本文と MessageInput は {@link BoundSessionChat} に委譲し、AgentChatPanel と
 * 同一実装を共有する（issue #1023 「タブ含めて同じ UI で session フィルタだけが違う」設計）。
 * 本 panel 固有の責務は「タブ列管理 + WorkflowPanel との session id 受け渡し」のみ。
 */
export function WorkflowView({
	worktreePath,
	permissionMode = "readonly",
}: WorkflowViewProps) {
	const { workflowState } = useWorkflowState(worktreePath);

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

	const activeSessionId = activeTab?.sessionId ?? null;
	const showChat = activeTab !== null && activeTab.sessionId != null;

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
										<div className="flex flex-1 flex-col min-h-0 overflow-hidden">
											{showChat && activeSessionId ? (
												<BoundSessionChat
													sessionId={activeSessionId}
													worktreePath={worktreePath}
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
