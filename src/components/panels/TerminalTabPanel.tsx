import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import {
	type DragEvent,
	forwardRef,
	useCallback,
	useEffect,
	useImperativeHandle,
	useRef,
	useState,
} from "react";
import { PANE_DRAG_TYPE } from "@/components/panels/PaneDropZone";
import { PaneTreeRenderer } from "@/components/panels/PaneTreeRenderer";
import type { TerminalPanelHandle } from "@/components/panels/TerminalPanel";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useTerminalPanes } from "@/hooks/useTerminalPanes";
import { highestPriorityState } from "@/lib/agentStateUtils";
import { normalizePath } from "@/lib/normalizePath";
import { countLeaves, getAllLeaves } from "@/lib/paneTree";
import type { AgentState, AgentStateSync } from "@/types/protocol";
import type { Theme } from "@/types/settings";
import type { SplitDirection } from "@/types/terminal-pane";

const TAB_DRAG_TYPE = "application/x-terminal-tab";

const MAX_TABS = 8;

export interface TerminalTabPanelHandle {
	writeToTerminal: (data: string) => void;
}

interface TerminalTabPanelProps {
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	agentType?: string;
	tabPrefix?: string;
}

export const TerminalTabPanel = forwardRef<
	TerminalTabPanelHandle,
	TerminalTabPanelProps
>(function TerminalTabPanel(
	{ cwd, theme, terminalStartupCommand, agentType, tabPrefix = "Terminal" },
	ref,
) {
	const {
		tabs,
		activeTabId,
		setActiveTabId,
		addTab,
		closeTab,
		splitFocusedPane,
		closeSpecificPane,
		moveFocus,
		setFocusedPane,
		moveTabToPane,
		movePaneToTab,
		movePaneInTab,
		updatePaneSessionKey,
		activeTab,
	} = useTerminalPanes(tabPrefix, cwd ? `${cwd}::${tabPrefix}` : null);

	const terminalRefs = useRef<Map<string, TerminalPanelHandle>>(new Map());

	// paneId → numeric ptyId マッピング
	const paneIdToPtyIdRef = useRef<Map<string, number>>(new Map());
	// pty_id (string) → AgentState マッピング
	const [agentStateByPtyId, setAgentStateByPtyId] = useState<
		Map<string, AgentState>
	>(new Map());

	const handlePtyReady = useCallback(
		(paneId: string, ptyId: number, sessionKey: string) => {
			paneIdToPtyIdRef.current.set(paneId, ptyId);
			updatePaneSessionKey(paneId, sessionKey);
		},
		[updatePaneSessionKey],
	);

	// agent-state-changed イベントリスナー
	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			const { worktree_path, state, pty_id } = event.payload;
			if (!cwd || normalizePath(worktree_path) !== normalizePath(cwd)) return;
			if (!pty_id) return;
			setAgentStateByPtyId((prev) => {
				const next = new Map(prev);
				next.set(pty_id, state);
				return next;
			});
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [cwd]);

	// worktree マウント時に孤立 PTY を GC
	const tabsRef = useRef(tabs);
	tabsRef.current = tabs;
	useEffect(() => {
		if (!cwd) return;
		const knownKeys = tabsRef.current.flatMap((tab) =>
			getAllLeaves(tab.paneTree)
				.map((leaf) => leaf.sessionKey)
				.filter((k): k is string => k !== null),
		);
		invoke("gc_ptys_for_worktree", {
			worktreePath: cwd,
			keepSessionKeys: knownKeys,
		}).catch((e) => console.warn("gc_ptys_for_worktree failed:", e));
	}, [cwd]);

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal: (data: string) => {
				if (!activeTab) return;
				const handle = terminalRefs.current.get(activeTab.focusedPaneId);
				handle?.writeToTerminal(data);
			},
		}),
		[activeTab],
	);

	const setTerminalRef = useCallback(
		(paneId: string) => (handle: TerminalPanelHandle | null) => {
			if (handle) {
				terminalRefs.current.set(paneId, handle);
			} else {
				terminalRefs.current.delete(paneId);
			}
		},
		[],
	);

	const handleSplit = useCallback(
		(paneId: string, direction: SplitDirection) => {
			setFocusedPane(paneId);
			splitFocusedPane(direction);
		},
		[setFocusedPane, splitFocusedPane],
	);

	const handleCloseTab = useCallback(
		(tabId: string) => {
			const tab = tabs.find((t) => t.id === tabId);
			if (tab) {
				const leaves = getAllLeaves(tab.paneTree);
				for (const leaf of leaves) {
					terminalRefs.current.get(leaf.id)?.requestKill();
				}
			}
			closeTab(tabId);
		},
		[tabs, closeTab],
	);

	const isDraggingTabRef = useRef(false);

	const handleTabDragStart = useCallback((e: DragEvent, tabId: string) => {
		isDraggingTabRef.current = true;
		e.dataTransfer.setData(TAB_DRAG_TYPE, tabId);
		e.dataTransfer.effectAllowed = "move";
	}, []);

	const handleTabDragEnd = useCallback(() => {
		isDraggingTabRef.current = false;
	}, []);

	const handleTabValueChange = useCallback(
		(value: string) => {
			if (isDraggingTabRef.current) return;
			setActiveTabId(value);
		},
		[setActiveTabId],
	);

	const handleDropTab = useCallback(
		(tabId: string, targetPaneId: string, direction: SplitDirection) => {
			moveTabToPane(tabId, targetPaneId, direction);
		},
		[moveTabToPane],
	);

	const handleDropPane = useCallback(
		(
			sourcePaneId: string,
			targetPaneId: string,
			direction: SplitDirection,
			insertBefore: boolean,
		) => {
			movePaneInTab(sourcePaneId, targetPaneId, direction, insertBefore);
		},
		[movePaneInTab],
	);

	const handleBreakToTab = useCallback(
		(paneId: string) => {
			movePaneToTab(paneId);
		},
		[movePaneToTab],
	);

	// タブバーへのペインドロップ
	const handlePaneDragOverTabBar = useCallback((e: DragEvent) => {
		if (!e.dataTransfer.types.includes(PANE_DRAG_TYPE)) return;
		e.preventDefault();
		e.dataTransfer.dropEffect = "move";
	}, []);

	const handlePaneDropOnTabBar = useCallback(
		(e: DragEvent) => {
			const paneId = e.dataTransfer.getData(PANE_DRAG_TYPE);
			if (!paneId) return;
			e.preventDefault();
			movePaneToTab(paneId);
		},
		[movePaneToTab],
	);

	// キーボードショートカット
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			const mod = e.metaKey || e.ctrlKey;
			const key = e.key.toLowerCase();

			// Cmd+D: 垂直分割
			if (mod && !e.shiftKey && !e.altKey && key === "d") {
				e.preventDefault();
				splitFocusedPane("vertical");
				return;
			}

			// Cmd+Shift+D: 水平分割
			if (mod && e.shiftKey && !e.altKey && key === "d") {
				e.preventDefault();
				splitFocusedPane("horizontal");
				return;
			}

			// Cmd+Shift+T: フォーカスペインをタブに分離
			if (mod && e.shiftKey && !e.altKey && e.key === "T") {
				e.preventDefault();
				const tab = tabs.find((t) => t.id === activeTabId);
				if (tab && countLeaves(tab.paneTree) > 1 && tabs.length < MAX_TABS) {
					movePaneToTab(tab.focusedPaneId);
				}
				return;
			}

			// Cmd+Option+矢印: フォーカス移動
			if (mod && e.altKey) {
				switch (e.key) {
					case "ArrowLeft":
						e.preventDefault();
						moveFocus("left");
						return;
					case "ArrowRight":
						e.preventDefault();
						moveFocus("right");
						return;
					case "ArrowUp":
						e.preventDefault();
						moveFocus("up");
						return;
					case "ArrowDown":
						e.preventDefault();
						moveFocus("down");
						return;
				}
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [splitFocusedPane, moveFocus, movePaneToTab, tabs, activeTabId]);

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTabId}
				onValueChange={handleTabValueChange}
				className="flex flex-col h-full gap-0"
			>
				{/* biome-ignore lint/a11y/noStaticElementInteractions: タブバーへのペインドロップ */}
				<div
					className="flex items-center gap-2 shrink-0 px-2 pt-2 bg-background"
					onDragOver={handlePaneDragOverTabBar}
					onDrop={handlePaneDropOnTabBar}
				>
					<TabsList aria-label="ターミナルタブ">
						{tabs.map((tab) => (
							<TabsTrigger key={tab.id} value={tab.id} asChild>
								{/* biome-ignore lint/a11y/noStaticElementInteractions: TabsTrigger asChild が role を付与 */}
								{/* biome-ignore lint/a11y/useKeyWithClickEvents: TabsTrigger がキーボード操作を処理 */}
								<div
									className="gap-2"
									draggable={tabs.length > 1}
									onPointerDown={() => {
										if (tabs.length > 1) isDraggingTabRef.current = true;
									}}
									onClick={() => {
										isDraggingTabRef.current = false;
										setActiveTabId(tab.id);
									}}
									onDragStart={(e) => handleTabDragStart(e, tab.id)}
									onDragEnd={handleTabDragEnd}
								>
									<span>{tab.label}</span>
									<TabAgentBadge
										tab={tab}
										paneIdToPtyIdRef={paneIdToPtyIdRef}
										agentStateByPtyId={agentStateByPtyId}
									/>
									{tabs.length > 1 && (
										<button
											type="button"
											onPointerDown={(e) => e.stopPropagation()}
											onMouseDown={(e) => e.stopPropagation()}
											onClick={(e) => {
												e.stopPropagation();
												handleCloseTab(tab.id);
											}}
											className="p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
											aria-label={`Close ${tab.label}`}
										>
											<X className="size-3.5" />
										</button>
									)}
								</div>
							</TabsTrigger>
						))}
					</TabsList>
					{tabs.length < MAX_TABS && (
						<button
							type="button"
							onClick={addTab}
							aria-label="Add terminal tab"
							className="px-2 h-full text-sm text-muted-foreground hover:text-foreground transition-colors shrink-0"
						>
							+
						</button>
					)}
				</div>
				<div className="flex-1 relative" style={{ minHeight: 0 }}>
					{tabs.map((tab) => (
						<TabsContent
							key={tab.id}
							value={tab.id}
							forceMount
							className="absolute inset-0 m-0 data-[state=inactive]:hidden"
						>
							<PaneTreeRenderer
								node={tab.paneTree}
								focusedPaneId={tab.focusedPaneId}
								isOnlyPane={countLeaves(tab.paneTree) === 1}
								cwd={cwd}
								theme={theme}
								terminalStartupCommand={terminalStartupCommand}
								agentType={agentType}
								onFocus={setFocusedPane}
								onClose={closeSpecificPane}
								onSplit={handleSplit}
								setTerminalRef={setTerminalRef}
								onPtyReady={handlePtyReady}
								onDropTab={handleDropTab}
								onDropPane={handleDropPane}
								onBreakToTab={handleBreakToTab}
								canBreakToTab={
									tabs.length < MAX_TABS && countLeaves(tab.paneTree) > 1
								}
							/>
						</TabsContent>
					))}
				</div>
			</Tabs>
		</div>
	);
});

function TabAgentBadge({
	tab,
	paneIdToPtyIdRef,
	agentStateByPtyId,
}: {
	tab: { paneTree: import("@/types/terminal-pane").PaneNode };
	paneIdToPtyIdRef: React.RefObject<Map<string, number>>;
	agentStateByPtyId: Map<string, AgentState>;
}) {
	const leaves = getAllLeaves(tab.paneTree);
	const states: AgentState[] = [];
	for (const leaf of leaves) {
		const ptyId = paneIdToPtyIdRef.current.get(leaf.id);
		if (ptyId != null) {
			const state = agentStateByPtyId.get(String(ptyId));
			if (state) states.push(state);
		}
	}
	const best = highestPriorityState(states);
	if (!best) return null;
	return <AgentStateIcon state={best} />;
}
