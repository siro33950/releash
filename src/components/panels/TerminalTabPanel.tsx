import { X } from "lucide-react";
import {
	type DragEvent,
	forwardRef,
	useCallback,
	useEffect,
	useImperativeHandle,
	useRef,
} from "react";
import { PaneTreeRenderer } from "@/components/panels/PaneTreeRenderer";
import type { TerminalPanelHandle } from "@/components/panels/TerminalPanel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useTerminalPanes } from "@/hooks/useTerminalPanes";
import { countLeaves, getAllLeaves } from "@/lib/paneTree";
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
	sessionKey?: string;
	tabPrefix?: string;
}

export const TerminalTabPanel = forwardRef<
	TerminalTabPanelHandle,
	TerminalTabPanelProps
>(function TerminalTabPanel(
	{
		cwd,
		theme,
		terminalStartupCommand,
		agentType,
		sessionKey,
		tabPrefix = "Terminal",
	},
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
		activeTab,
	} = useTerminalPanes(tabPrefix);

	const terminalRefs = useRef<Map<string, TerminalPanelHandle>>(new Map());

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

	const handleTabDragStart = useCallback((e: DragEvent, tabId: string) => {
		e.dataTransfer.setData(TAB_DRAG_TYPE, tabId);
		e.dataTransfer.effectAllowed = "move";
	}, []);

	const handleDropTab = useCallback(
		(tabId: string, targetPaneId: string, direction: SplitDirection) => {
			const tab = tabs.find((t) => t.id === tabId);
			if (!tab) return;

			// タブのルートがリーフの場合のみ対応
			const leaves = getAllLeaves(tab.paneTree);
			if (leaves.length !== 1) return;

			// ドロップ先ペインを分割して元タブのリーフを挿入
			setFocusedPane(targetPaneId);
			splitFocusedPane(direction);

			// 元タブを閉じる
			closeTab(tabId);
		},
		[tabs, setFocusedPane, splitFocusedPane, closeTab],
	);

	// キーボードショートカット
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			const mod = e.metaKey || e.ctrlKey;

			// Cmd+D: 垂直分割
			if (mod && !e.shiftKey && !e.altKey && e.key === "d") {
				e.preventDefault();
				splitFocusedPane("vertical");
				return;
			}

			// Cmd+Shift+D: 水平分割
			if (mod && e.shiftKey && !e.altKey && e.key === "D") {
				e.preventDefault();
				splitFocusedPane("horizontal");
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
	}, [splitFocusedPane, moveFocus]);

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTabId}
				onValueChange={setActiveTabId}
				className="flex flex-col h-full gap-0"
			>
				<div className="flex items-center gap-2 shrink-0 px-2 pt-2 bg-background">
					<TabsList aria-label="ターミナルタブ">
						{tabs.map((tab) => (
							<TabsTrigger key={tab.id} value={tab.id} asChild>
								{/* biome-ignore lint/a11y/noStaticElementInteractions: TabsTrigger asChild が role を付与 */}
								<div
									className="gap-2"
									draggable={tabs.length > 1}
									onDragStart={(e) => handleTabDragStart(e, tab.id)}
								>
									<span>{tab.label}</span>
									{tabs.length > 1 && (
										<button
											type="button"
											onPointerDown={(e) => e.stopPropagation()}
											onMouseDown={(e) => e.stopPropagation()}
											onClick={(e) => {
												e.stopPropagation();
												closeTab(tab.id);
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
								sessionKey={sessionKey}
								onFocus={setFocusedPane}
								onClose={closeSpecificPane}
								onSplit={handleSplit}
								setTerminalRef={setTerminalRef}
								onDropTab={handleDropTab}
							/>
						</TabsContent>
					))}
				</div>
			</Tabs>
		</div>
	);
});
