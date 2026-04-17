import { Group, Panel, Separator } from "react-resizable-panels";
import { PaneLeafContainer } from "@/components/panels/PaneLeafContainer";
import type { TerminalPanelHandle } from "@/components/panels/TerminalPanel";
import type { Theme } from "@/types/settings";
import type { PaneNode, SplitDirection } from "@/types/terminal-pane";

interface PaneTreeRendererProps {
	node: PaneNode;
	focusedPaneId: string;
	isOnlyPane: boolean;
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;

	onFocus: (paneId: string) => void;
	onClose: (paneId: string) => void;
	onSplit: (paneId: string, direction: SplitDirection) => void;
	setTerminalRef: (
		paneId: string,
	) => (handle: TerminalPanelHandle | null) => void;
	onPtyReady?: (paneId: string, ptyId: number, sessionKey: string) => void;
	onDropTab?: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
	onDropPane?: (
		sourcePaneId: string,
		targetPaneId: string,
		direction: SplitDirection,
		insertBefore: boolean,
	) => void;
	onBreakToTab?: (paneId: string) => void;
	canBreakToTab?: boolean;
}

export function PaneTreeRenderer({
	node,
	focusedPaneId,
	isOnlyPane,
	cwd,
	theme,
	terminalStartupCommand,

	onFocus,
	onClose,
	onSplit,
	setTerminalRef,
	onPtyReady,
	onDropTab,
	onDropPane,
	onBreakToTab,
	canBreakToTab,
}: PaneTreeRendererProps) {
	if (node.type === "leaf") {
		return (
			<PaneLeafContainer
				pane={node}
				isFocused={node.id === focusedPaneId}
				isOnlyPane={isOnlyPane}
				cwd={cwd}
				theme={theme}
				terminalStartupCommand={terminalStartupCommand}
				onFocus={onFocus}
				onClose={onClose}
				onSplit={onSplit}
				setTerminalRef={setTerminalRef}
				onPtyReady={onPtyReady}
				onDropTab={onDropTab}
				onDropPane={onDropPane}
				onBreakToTab={onBreakToTab}
				canBreakToTab={canBreakToTab}
			/>
		);
	}

	// container ノード → Group + Panel + Separator
	// direction: "horizontal" (上下分割) → orientation="vertical"
	// direction: "vertical" (左右分割) → orientation="horizontal"
	const orientation =
		node.direction === "horizontal" ? "vertical" : "horizontal";

	return (
		<Group orientation={orientation}>
			{node.children.map((child, i) => (
				<PaneTreePanel
					key={child.id}
					child={child}
					ratio={node.ratios[i]}
					isLast={i === node.children.length - 1}
					focusedPaneId={focusedPaneId}
					isOnlyPane={isOnlyPane}
					cwd={cwd}
					theme={theme}
					terminalStartupCommand={terminalStartupCommand}
					onFocus={onFocus}
					onClose={onClose}
					onSplit={onSplit}
					setTerminalRef={setTerminalRef}
					onPtyReady={onPtyReady}
					onDropTab={onDropTab}
					onDropPane={onDropPane}
					onBreakToTab={onBreakToTab}
					canBreakToTab={canBreakToTab}
				/>
			))}
		</Group>
	);
}

interface PaneTreePanelProps {
	child: PaneNode;
	ratio: number;
	isLast: boolean;
	focusedPaneId: string;
	isOnlyPane: boolean;
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;

	onFocus: (paneId: string) => void;
	onClose: (paneId: string) => void;
	onSplit: (paneId: string, direction: SplitDirection) => void;
	setTerminalRef: (
		paneId: string,
	) => (handle: TerminalPanelHandle | null) => void;
	onPtyReady?: (paneId: string, ptyId: number, sessionKey: string) => void;
	onDropTab?: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
	onDropPane?: (
		sourcePaneId: string,
		targetPaneId: string,
		direction: SplitDirection,
		insertBefore: boolean,
	) => void;
	onBreakToTab?: (paneId: string) => void;
	canBreakToTab?: boolean;
}

function PaneTreePanel({
	child,
	ratio,
	isLast,
	focusedPaneId,
	isOnlyPane,
	cwd,
	theme,
	terminalStartupCommand,

	onFocus,
	onClose,
	onSplit,
	setTerminalRef,
	onPtyReady,
	onDropTab,
	onDropPane,
	onBreakToTab,
	canBreakToTab,
}: PaneTreePanelProps) {
	return (
		<>
			<Panel id={child.id} defaultSize={`${ratio * 100}%`} minSize="10%">
				<PaneTreeRenderer
					node={child}
					focusedPaneId={focusedPaneId}
					isOnlyPane={isOnlyPane}
					cwd={cwd}
					theme={theme}
					terminalStartupCommand={terminalStartupCommand}
					onFocus={onFocus}
					onClose={onClose}
					onSplit={onSplit}
					setTerminalRef={setTerminalRef}
					onPtyReady={onPtyReady}
					onDropTab={onDropTab}
					onDropPane={onDropPane}
					onBreakToTab={onBreakToTab}
					canBreakToTab={canBreakToTab}
				/>
			</Panel>
			{!isLast && <Separator />}
		</>
	);
}
