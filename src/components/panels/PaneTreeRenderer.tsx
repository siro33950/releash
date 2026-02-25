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
	agentType?: string;
	sessionKey?: string;
	onFocus: (paneId: string) => void;
	onClose: (paneId: string) => void;
	onSplit: (paneId: string, direction: SplitDirection) => void;
	setTerminalRef: (
		paneId: string,
	) => (handle: TerminalPanelHandle | null) => void;
	onDropTab?: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
}

export function PaneTreeRenderer({
	node,
	focusedPaneId,
	isOnlyPane,
	cwd,
	theme,
	terminalStartupCommand,
	agentType,
	sessionKey,
	onFocus,
	onClose,
	onSplit,
	setTerminalRef,
	onDropTab,
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
				agentType={agentType}
				sessionKey={sessionKey}
				onFocus={onFocus}
				onClose={onClose}
				onSplit={onSplit}
				setTerminalRef={setTerminalRef}
				onDropTab={onDropTab}
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
					agentType={agentType}
					sessionKey={sessionKey}
					onFocus={onFocus}
					onClose={onClose}
					onSplit={onSplit}
					setTerminalRef={setTerminalRef}
					onDropTab={onDropTab}
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
	agentType?: string;
	sessionKey?: string;
	onFocus: (paneId: string) => void;
	onClose: (paneId: string) => void;
	onSplit: (paneId: string, direction: SplitDirection) => void;
	setTerminalRef: (
		paneId: string,
	) => (handle: TerminalPanelHandle | null) => void;
	onDropTab?: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
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
	agentType,
	sessionKey,
	onFocus,
	onClose,
	onSplit,
	setTerminalRef,
	onDropTab,
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
					agentType={agentType}
					sessionKey={sessionKey}
					onFocus={onFocus}
					onClose={onClose}
					onSplit={onSplit}
					setTerminalRef={setTerminalRef}
					onDropTab={onDropTab}
				/>
			</Panel>
			{!isLast && <Separator />}
		</>
	);
}
