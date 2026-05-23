import { X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { BoundSessionChat } from "@/components/panels/AgentChatPanel";
import {
	WorkflowPanel,
	type WorkflowStepSelection,
} from "@/components/panels/WorkflowPanel";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
	closeSession as closeSessionApi,
	openWorkflowStepTab,
} from "@/hooks/useSessionStore";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import { useWorktreeSessionStatuses } from "@/hooks/useWorktreeSessionStatuses";
import type { AgentState } from "@/types/protocol";
import type { PermissionMode } from "@/types/session";
import type { WorkflowState } from "@/types/workflow";
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

	// Step タブの状態アイコン用: AgentChatPanel と同じく Rust 中央管理から取得した
	// SessionStatus を sessionId -> AgentState の Map に整形する。
	const worktreeSessionStatuses = useWorktreeSessionStatuses(worktreePath);
	const sessionAgentStates = useMemo(() => {
		const map = new Map<string, AgentState>();
		for (const [sessionId, status] of worktreeSessionStatuses) {
			map.set(sessionId, status.agent_state);
		}
		return map;
	}, [worktreeSessionStatuses]);

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

	// spec issues-1023 後続改善: タブの open/close の真実源は Rust 側 lifecycle
	// (`runtime_states[sid].tab_open`) に集約する。step 開始時に Rust が
	// `mark_step_tab_open` で true を立て、step 完了時に `close_step_tab` で false に
	// 倒すため、その状態を `openTabs` へ同期するだけで「開始で Show / 完了で Hide」
	// の自動挙動が得られる。手動操作は楽観的にローカル state を更新しつつ、Rust
	// commands (`open_workflow_step_tab` / `close_session`) を呼んで真実源を更新する。
	const handleSelectStepSession = useCallback((next: WorkflowStepSelection) => {
		const key = stepTabKey(next);
		setOpenTabs((prev) =>
			prev.some((tab) => stepTabKey(tab) === key) ? prev : [...prev, next],
		);
		setActiveTabKey(key);
		// Rust 側 tab_open=true を立てる。失敗時は次回 workflowState 同期で整合する。
		if (next.sessionId) {
			void openWorkflowStepTab(next.sessionId).catch(() => {});
		}
	}, []);

	const handleSelectTab = useCallback((key: string) => {
		setActiveTabKey(key);
	}, []);

	const handleCloseTab = useCallback(
		(key: string) => {
			const target = openTabs.find((tab) => stepTabKey(tab) === key);
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
			// Rust 側 tab_open=false を立てる。`close_session` は workflow step session に
			// 対しては tab を閉じるだけで session 履歴は破壊しない（session_commands.rs:24-44）。
			if (target?.sessionId) {
				void closeSessionApi(target.sessionId).catch(() => {});
			}
		},
		[activeTabKey, openTabs],
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

	// Rust 由来の `runtimeStates[sid].tab_open` を `openTabs` に同期する。
	// - tab_open=true で openTabs に無い sessionId → タブを自動追加（= 開始で Show）
	// - tab_open=false に変化した sessionId が openTabs に有る → 自動除去（= 完了で Hide）
	// sessionId を持たないタブ（bash / parallel parent / 未紐付け completed）は Rust
	// lifecycle 対象外なので、フロント useState 管理のまま保持する。
	useEffect(() => {
		if (!workflowState?.runtimeStates) return;
		const runtimeStates = workflowState.runtimeStates;
		setOpenTabs((prev) => {
			const filtered = prev.filter((tab) => {
				if (!tab.sessionId) return true;
				const rs = runtimeStates[tab.sessionId];
				if (rs && rs.tabOpen === false) return false;
				return true;
			});
			const existingIds = new Set(
				filtered
					.map((t) => t.sessionId)
					.filter((id): id is string => id != null),
			);
			const additions: WorkflowStepSelection[] = [];
			for (const [sid, rs] of Object.entries(runtimeStates)) {
				if (!rs.tabOpen) continue;
				if (existingIds.has(sid)) continue;
				const sel = resolveStepSelection(workflowState, sid);
				if (sel) additions.push(sel);
			}
			if (filtered.length === prev.length && additions.length === 0) {
				return prev;
			}
			return [...filtered, ...additions];
		});
	}, [workflowState]);

	// 自動 close で activeTab が openTabs から消えた場合のフォールバック。
	// （手動 close は handleCloseTab 内で隣接タブに移すため、ここは自動経路用の保険）
	useEffect(() => {
		if (!activeTabKey) return;
		if (openTabs.some((t) => stepTabKey(t) === activeTabKey)) return;
		const last = openTabs[openTabs.length - 1];
		setActiveTabKey(last ? stepTabKey(last) : null);
	}, [openTabs, activeTabKey]);

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
											sessionAgentStates={sessionAgentStates}
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

/**
 * Rust 由来の `runtimeStates` から「自動 Show」対象として現れた sessionId を、
 * timeline 上の step メタ（stepName / nodeType / runIndex）に解決して
 * `WorkflowStepSelection` を構築する。解決経路は次の優先順：
 *   1. current step (`currentSessionId`)
 *   2. active parallel children (`activeParallelSteps`)
 *   3. step history（top-level entry / parallel child snapshot の順）
 * いずれの経路でも見つからない場合は `null` を返す（同期 effect 側で skip）。
 */
function resolveStepSelection(
	workflowState: WorkflowState,
	sessionId: string,
): WorkflowStepSelection | null {
	const runId = workflowState.executionId;
	const nodesByName = new Map(
		workflowState.workflowDefinition.nodes.map((n) => [n.name, n]),
	);

	if (
		workflowState.currentSessionId === sessionId &&
		workflowState.currentStepName
	) {
		const stepName = workflowState.currentStepName;
		const node = nodesByName.get(stepName);
		const runIndex = workflowState.stepExecutionCounts[stepName] ?? 1;
		return {
			runId,
			sessionId,
			stepName,
			nodeType: (node?.type ?? "unknown") as WorkflowStepSelection["nodeType"],
			runIndex,
		};
	}

	if (workflowState.activeParallelSteps) {
		for (const ps of workflowState.activeParallelSteps) {
			if (ps.sessionId !== sessionId) continue;
			let nodeType: WorkflowStepSelection["nodeType"] = "agent";
			for (const parent of workflowState.workflowDefinition.nodes) {
				const child = parent.parallel_children?.find(
					(c) => c.name === ps.stepName,
				);
				if (child) {
					nodeType = child.type;
					break;
				}
			}
			return {
				runId,
				sessionId,
				stepName: ps.stepName,
				nodeType,
				runIndex: ps.runIndex,
			};
		}
	}

	for (const entry of workflowState.stepHistory) {
		if (entry.sessionId === sessionId) {
			const node = nodesByName.get(entry.stepName);
			return {
				runId,
				sessionId,
				stepName: entry.stepName,
				nodeType: (node?.type ??
					"unknown") as WorkflowStepSelection["nodeType"],
				runIndex: entry.runIndex,
			};
		}
		if (entry.childOutputs) {
			const parentNode = nodesByName.get(entry.stepName);
			for (const co of entry.childOutputs) {
				if (co.sessionId !== sessionId) continue;
				const child = parentNode?.parallel_children?.find(
					(c) => c.name === co.stepName,
				);
				return {
					runId,
					sessionId,
					stepName: co.stepName,
					nodeType: child?.type ?? "agent",
					runIndex: co.runIndex,
				};
			}
		}
	}

	return null;
}

interface WorkflowStepTabBarProps {
	openTabs: WorkflowStepSelection[];
	activeTabKey: string | null;
	onSelectTab: (key: string) => void;
	onCloseTab: (key: string) => void;
	/**
	 * sessionId → AgentState の Map。
	 * AgentChatPanel のタブと同じく、各 step タブの左端に状態アイコンを描画するために使う。
	 * sessionId を持たない tab（bash / parallel parent / 未紐付け completed）では
	 * Map に該当 entry が存在せず、AgentStateIcon は state=undefined の
	 * グレーフォールバック表示になる。
	 */
	sessionAgentStates: Map<string, AgentState>;
}

function WorkflowStepTabBar({
	openTabs,
	activeTabKey,
	onSelectTab,
	onCloseTab,
	sessionAgentStates,
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
						const agentState = tab.sessionId
							? sessionAgentStates.get(tab.sessionId)
							: undefined;
						return (
							<TabsTrigger key={key} value={key} asChild>
								<div className="gap-2">
									<AgentStateIcon state={agentState} />
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
